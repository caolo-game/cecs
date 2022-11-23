#![feature(const_type_id)]

use std::{
    any::TypeId, cell::UnsafeCell, collections::BTreeMap, mem::transmute, pin::Pin, ptr::NonNull,
};

use archetype::ArchetypeStorage;
use commands::CommandPayload;
use entity_id::EntityId;
use entity_index::EntityIndex;
use prelude::Bundle;
use resources::ResourceStorage;
use systems::SystemStage;

pub mod bundle;
pub mod commands;
pub mod entity_id;
pub mod entity_index;
pub mod prelude;
pub mod query;
pub mod query_set;
pub mod resources;
pub mod systems;
pub mod world_access;

#[cfg(feature = "serde")]
pub mod persister;

mod archetype;

#[cfg(feature = "parallel")]
mod scheduler;

#[cfg(feature = "parallel")]
pub use rayon;
use world_access::WorldLock;

#[cfg(test)]
mod world_tests;

type CommandBuffer<T> = std::cell::UnsafeCell<Vec<T>>;

pub struct World {
    pub(crate) this_lock: WorldLock,
    pub(crate) entity_ids: UnsafeCell<EntityIndex>,
    pub(crate) archetypes: BTreeMap<TypeHash, Pin<Box<ArchetypeStorage>>>,
    pub(crate) resources: ResourceStorage,
    pub(crate) commands: Vec<CommandBuffer<CommandPayload>>,
    pub(crate) system_stages: Vec<SystemStage<'static>>,
    // for each system stage: a group of parallel systems
    //
    #[cfg(feature = "parallel")]
    pub(crate) schedule: Vec<Vec<Vec<usize>>>,
}

unsafe impl Send for World {}
unsafe impl Sync for World {}

#[cfg(feature = "clone")]
impl Clone for World {
    fn clone(&self) -> Self {
        // we don't actually mutate archetypes here, we just need mutable references to cast to
        // pointers
        let mut archetypes = self.archetypes.clone();
        let commands = Vec::default();

        let mut entity_ids = self.entity_ids().clone();
        for id in prelude::Query::<EntityId>::new(self).iter() {
            let (ptr, row_index) = self.entity_ids().read(id).unwrap();
            let ty = unsafe { ptr.as_ref() }.ty();
            let new_arch = archetypes.get_mut(&ty).unwrap();
            unsafe {
                entity_ids.update(id, new_arch.as_mut().get_mut() as *mut _, row_index);
            }
        }

        let resources = self.resources.clone();

        let systems = self.system_stages.clone();

        #[cfg(feature = "parallel")]
        let schedule = self.schedule.clone();

        Self {
            this_lock: WorldLock::new(),
            entity_ids: UnsafeCell::new(entity_ids),
            archetypes,
            commands,
            resources,
            system_stages: systems,
            #[cfg(feature = "parallel")]
            schedule,
        }
    }
}

type TypeHash = u64;

const fn hash_ty<T: 'static>() -> u64 {
    let ty = TypeId::of::<T>();
    // FIXME extreme curse
    //
    debug_assert!(std::mem::size_of::<TypeId>() == std::mem::size_of::<u64>());
    let ty: u64 = unsafe { transmute(ty) };
    if ty == unsafe { transmute::<_, u64>(TypeId::of::<()>()) } {
        // ensure that unit type has hash=0
        0
    } else {
        ty
    }
}

const VOID_TY: TypeHash = hash_ty::<()>();

#[derive(Clone, Debug, thiserror::Error)]
pub enum WorldError {
    #[error("World is full and can not take more entities")]
    OutOfCapacity,
    #[error("Entity was not found")]
    EntityNotFound,
    #[error("Entity doesn't have specified component")]
    ComponentNotFound,
}

pub type WorldResult<T> = Result<T, WorldError>;
pub type RowIndex = u32;

#[cfg(feature = "parallel")]
pub trait ParallelComponent: Send + Sync {}
#[cfg(not(feature = "parallel"))]
pub trait ParallelComponent {}

#[cfg(feature = "parallel")]
impl<T: Send + Sync> ParallelComponent for T {}
#[cfg(not(feature = "parallel"))]
impl<T> ParallelComponent for T {}

/// The end goal is to have a clonable ECS, that's why we have the Clone restriction.
#[cfg(feature = "clone")]
pub trait Component: 'static + Clone + ParallelComponent {}
#[cfg(feature = "clone")]
impl<T: 'static + Clone + ParallelComponent> Component for T {}

#[cfg(not(feature = "clone"))]
pub trait Component: 'static + ParallelComponent {}
#[cfg(not(feature = "clone"))]
impl<T: 'static + ParallelComponent> Component for T {}

impl World {
    pub fn new(initial_capacity: u32) -> Self {
        let entity_ids = EntityIndex::new(initial_capacity);

        let mut result = Self {
            this_lock: WorldLock::new(),
            entity_ids: UnsafeCell::new(entity_ids),
            archetypes: BTreeMap::new(),
            resources: ResourceStorage::new(),
            commands: Vec::default(),
            system_stages: Default::default(),
            #[cfg(feature = "parallel")]
            schedule: Default::default(),
        };
        let void_store = Box::pin(ArchetypeStorage::empty());
        result.archetypes.insert(VOID_TY, void_store);
        result
    }

    pub fn reserve_entities(&mut self, additional: u32) {
        self.entity_ids.get_mut().reserve(additional);
    }

    pub fn entity_capacity(&self) -> usize {
        self.entity_ids().capacity()
    }

    #[cfg(feature = "parallel")]
    pub fn write_schedule(&self, mut w: impl std::io::Write) -> std::io::Result<()> {
        for (i, stage) in self.system_stages.iter().enumerate() {
            writeln!(w, "Stage {}:", stage.name)?;
            for (j, group) in self.schedule[i].iter().enumerate() {
                writeln!(w, "\tGroup {}:", j)?;
                for k in group.iter() {
                    writeln!(w, "\t\t- {}", stage.systems.as_slice()[*k].descriptor.name)?;
                }
            }
        }
        Ok(())
    }

    /// Writes entity ids and their archetype hash
    pub fn write_entities(&self, mut w: impl std::io::Write) -> std::io::Result<()> {
        for id in prelude::Query::<EntityId>::new(self).iter() {
            let (arch, _row_index) = self.entity_ids().read(id).unwrap();
            let ty = unsafe { arch.as_ref().ty() };
            write!(w, "{}: {}, ", id, ty)?;
        }
        Ok(())
    }

    pub fn num_entities(&self) -> usize {
        self.entity_ids().len()
    }

    pub fn is_id_valid(&self, id: EntityId) -> bool {
        self.entity_ids().is_valid(id)
    }

    pub fn apply_commands(&mut self) -> WorldResult<()> {
        #[cfg(feature = "tracing")]
        tracing::trace!("Running commands");
        let mut commands = std::mem::take(&mut self.commands);
        for commands in commands.iter_mut() {
            for cmd in commands.get_mut().drain(0..) {
                cmd.apply(self)?;
            }
        }
        self.commands = commands;

        #[cfg(feature = "tracing")]
        tracing::trace!("Running commands done");
        Ok(())
    }

    pub fn insert_entity(&mut self) -> EntityId {
        let id = self.entity_ids.get_mut().allocate_with_resize();
        unsafe {
            self.init_id(id);
        }
        #[cfg(feature = "tracing")]
        tracing::trace!(id = tracing::field::display(id), "Inserted entity");
        id
    }

    /// # Safety
    /// Id must be an allocated but uninitialized entity
    pub(crate) unsafe fn init_id(&mut self, id: EntityId) {
        let void_store = self.archetypes.get_mut(&VOID_TY).unwrap();

        let index = void_store.as_mut().insert_entity(id);
        void_store.as_mut().set_component(index, ());
        self.entity_ids
            .get_mut()
            .update(id, void_store.as_mut().get_mut() as *mut _, index);
    }

    pub fn delete_entity(&mut self, id: EntityId) -> WorldResult<()> {
        #[cfg(feature = "tracing")]
        tracing::trace!(id = tracing::field::display(id), "Delete entity");
        if !self.entity_ids().is_valid(id) {
            return Err(WorldError::EntityNotFound);
        }

        let (mut archetype, index) = self
            .entity_ids()
            .read(id)
            .map_err(|_| WorldError::EntityNotFound)?;
        unsafe {
            if let Some(id) = archetype.as_mut().remove(index) {
                self.entity_ids.get_mut().update_row_index(id, index);
            }
            self.entity_ids.get_mut().free(id);
        }
        Ok(())
    }

    pub fn set_bundle<T: Bundle>(&mut self, entity_id: EntityId, bundle: T) -> WorldResult<()> {
        #[cfg(feature = "tracing")]
        tracing::trace!(
            entity_id = tracing::field::display(entity_id),
            ty = std::any::type_name::<T>(),
            "Set bundle"
        );

        let (mut archetype, mut index) = self
            .entity_ids()
            .read(entity_id)
            .map_err(|_| WorldError::EntityNotFound)?;
        let mut archetype = unsafe { archetype.as_mut() };

        if !bundle.can_insert(archetype) {
            let new_hash = T::compute_hash(archetype.ty);
            if !self.archetypes.contains_key(&new_hash) {
                let new_arch = T::extend(archetype);
                let (mut res, updated_entity) = self.insert_archetype(archetype, index, new_arch);
                if let Some(updated_entity) = updated_entity {
                    unsafe {
                        self.entity_ids
                            .get_mut()
                            .update_row_index(updated_entity, index);
                    }
                }
                archetype = unsafe { res.as_mut() };
                index = 0;
            } else {
                let new_arch = self.archetypes.get_mut(&new_hash).unwrap();
                let (i, updated_entity) = archetype.move_entity(new_arch, index);
                if let Some(updated_entity) = updated_entity {
                    unsafe {
                        self.entity_ids
                            .get_mut()
                            .update_row_index(updated_entity, index);
                    }
                }
                index = i;
                archetype = new_arch.as_mut().get_mut();
            }
        }
        bundle.insert_into(archetype, index)?;
        unsafe {
            self.entity_ids
                .get_mut()
                .update(entity_id, archetype, index);
        }
        Ok(())
    }

    pub fn set_component<T: Component>(
        &mut self,
        entity_id: EntityId,
        component: T,
    ) -> WorldResult<()> {
        self.set_bundle(entity_id, (component,))
    }

    pub fn get_component<T: Component>(&self, entity_id: EntityId) -> Option<&T> {
        let (arch, idx) = self.entity_ids().read(entity_id).ok()?;
        unsafe { arch.as_ref().get_component(idx) }
    }

    pub fn remove_component<T: Component>(&mut self, entity_id: EntityId) -> WorldResult<()> {
        #[cfg(feature = "tracing")]
        tracing::trace!(
            entity_id = tracing::field::display(entity_id),
            ty = std::any::type_name::<T>(),
            "Remove component"
        );

        let (mut archetype, mut index) = self
            .entity_ids()
            .read(entity_id)
            .map_err(|_| WorldError::EntityNotFound)?;
        let archetype = unsafe { archetype.as_mut() };
        let arch_ptr;
        if !archetype.contains_column::<T>() {
            return Err(WorldError::ComponentNotFound);
        }
        let new_ty = archetype.extended_hash::<T>();
        if !self.archetypes.contains_key(&new_ty) {
            let (mut res, updated_entity) =
                self.insert_archetype(archetype, index, archetype.reduce_with_column::<T>());
            if let Some(updated_entity) = updated_entity {
                unsafe {
                    self.entity_ids
                        .get_mut()
                        .update_row_index(updated_entity, index);
                }
            }
            arch_ptr = unsafe { res.as_mut() as *mut _ };
            index = 0;
        } else {
            let new_arch = self.archetypes.get_mut(&new_ty).unwrap();
            let (i, updated_entity) = archetype.move_entity(new_arch, index);
            if let Some(updated_entity) = updated_entity {
                unsafe {
                    self.entity_ids
                        .get_mut()
                        .update_row_index(updated_entity, index);
                }
            }
            index = i;
            arch_ptr = new_arch.as_mut().get_mut() as *mut _;
        }
        unsafe {
            self.entity_ids.get_mut().update(entity_id, arch_ptr, index);
        }
        Ok(())
    }

    #[inline(never)]
    #[must_use]
    fn insert_archetype(
        &mut self,
        archetype: &mut ArchetypeStorage,
        row_index: RowIndex,
        new_arch: ArchetypeStorage,
    ) -> (NonNull<ArchetypeStorage>, Option<EntityId>) {
        let mut new_arch = Box::pin(new_arch);
        let (index, moved_entity) = archetype.move_entity(&mut new_arch, row_index);
        debug_assert_eq!(index, 0);
        let res = unsafe { NonNull::new_unchecked(new_arch.as_mut().get_mut() as *mut _) };
        self.archetypes.insert(new_arch.ty(), new_arch);
        (res, moved_entity)
    }

    pub fn insert_resource<T: Component>(&mut self, value: T) {
        self.resources.insert(value);
    }

    pub fn remove_resource<T: 'static>(&mut self) -> Option<Box<T>> {
        self.resources.remove::<T>()
    }

    pub fn get_resource<T: 'static>(&self) -> Option<&T> {
        self.resources.fetch::<T>()
    }

    pub fn get_resource_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.resources.fetch_mut::<T>()
    }

    pub fn get_resource_or_default<T: 'static + Default + Component>(&mut self) -> &mut T {
        self.resources.fetch_or_default()
    }

    /// System stages are executed in the order they were added to the World
    pub fn add_stage(&mut self, stage: SystemStage<'_>) {
        // # SAFETY
        // lifetimes are managed by the World instance from now
        let stage = unsafe { transmute(stage) };
        #[cfg(feature = "parallel")]
        {
            self.schedule.push(scheduler::schedule(&stage));
        }
        self.system_stages.push(stage);
    }

    /// Run a single stage withouth adding it to the World
    ///
    pub fn run_stage(&mut self, stage: SystemStage<'_>) {
        #[cfg(feature = "tracing")]
        tracing::trace!(stage_name = stage.name.as_str(), "Update stage");

        let i = self.system_stages.len();
        // # SAFETY
        // lifetimes are managed by the World instance from now
        let stage = unsafe { transmute(stage) };

        // move stage into the world
        #[cfg(feature = "parallel")]
        self.schedule.push(crate::scheduler::schedule(&stage));

        self.system_stages.push(stage);

        self.execute_stage(i);

        // pop the stage after execution, one-shot stages are not stored
        self.system_stages.pop();
        #[cfg(feature = "parallel")]
        self.schedule.pop();
    }

    pub fn run_system<'a, S, P, R>(&mut self, system: S) -> R
    where
        S: systems::IntoSystem<'a, P, R>,
    {
        self.resize_commands(1);
        let result = unsafe { run_system(self, &system.descriptor().into()) };
        // apply commands immediately
        self.apply_commands().unwrap();
        result
    }

    pub fn tick(&mut self) {
        #[cfg(feature = "parallel")]
        debug_assert_eq!(self.system_stages.len(), self.schedule.len());
        for i in 0..self.system_stages.len() {
            self.execute_stage(i);
            // apply commands after each stage
            self.apply_commands().unwrap();
        }
    }

    fn execute_stage(&mut self, i: usize) {
        self.resize_commands(self.system_stages[i].systems.len());
        let stage = &self.system_stages[i];

        #[cfg(feature = "tracing")]
        let stage_name = stage.name.clone();
        #[cfg(feature = "tracing")]
        tracing::trace!(stage_name = stage_name.as_str(), "Run stage");

        for condition in stage.should_run.iter() {
            if !unsafe { run_system(self, condition) } {
                // stage should not run
                #[cfg(feature = "tracing")]
                tracing::trace!(
                    stage_name = stage.name.to_string(),
                    "Stage should_run was false"
                );
                return;
            }
        }

        match stage.systems {
            systems::StageSystems::Serial(ref systems) => {
                for system in systems.iter() {
                    unsafe {
                        run_system(self, system);
                    }
                }
            }
            #[cfg(feature = "parallel")]
            systems::StageSystems::Parallel(ref systems) => {
                let schedule = &self.schedule[i];
                for group in schedule {
                    self.execute_systems_parallel(group, systems)
                }
            }
        }
        #[cfg(feature = "tracing")]
        tracing::trace!(stage_name = stage_name.as_str(), "Run stage done");
    }

    fn resize_commands(&mut self, len: usize) {
        // do not shrink
        if self.commands.len() < len {
            self.commands
                .resize_with(len, std::cell::UnsafeCell::default);
        }
    }

    #[cfg(feature = "parallel")]
    fn execute_systems_parallel<'a>(
        &'a self,
        group: &[usize],
        systems: &[systems::ErasedSystem<()>],
    ) {
        use rayon::prelude::*;

        group.par_iter().copied().for_each(|i| unsafe {
            run_system(self, &systems[i]);
        });
    }

    /// Constructs a new [[crate::commands::Commands]] instance with initialized buffers in this world
    pub fn ensure_commands(&mut self) -> prelude::Commands {
        self.resize_commands(1);
        commands::Commands::new(self, 0)
    }

    /// Delete entities with only unit `()` components
    pub fn gc_empty_entities(&mut self) {
        let void_store = self.archetypes.get_mut(&VOID_TY).unwrap();
        let to_delete = void_store.entities.clone();
        for id in to_delete {
            self.delete_entity(id).unwrap();
        }
    }

    /// Return the "type" of the Entity.
    /// Type itself is an opaque hash.
    ///
    /// This function is meant to be used to test successful saving/loading of entities
    pub fn get_entity_ty(&self, id: EntityId) -> Option<TypeHash> {
        let (arch, _) = self.entity_ids().read(id).ok()?;
        Some(unsafe { arch.as_ref().ty() })
    }

    /// Compute a checksum of the World
    ///
    /// Note that only entities and their archetypes are considered, the contents of the components
    /// themselves are ignored
    pub fn checksum(&self) -> u64 {
        use std::hash::Hasher;

        let mut hasher = std::collections::hash_map::DefaultHasher::new();

        prelude::Query::<EntityId>::new(self).iter().for_each(|id| {
            let (arch, row_index) = self.entity_ids().read(id).unwrap();
            let ty = unsafe { arch.as_ref().ty() };
            hasher.write_u32(id.into());
            hasher.write_u64(ty);
            hasher.write_u32(row_index);
        });

        hasher.finish()
    }

    pub(crate) fn entity_ids(&self) -> &EntityIndex {
        unsafe { &*self.entity_ids.get() }
    }
}

// # SAFETY
// this World instance must be borrowed as mutable by the caller, so no other thread should have
// access to the internals
//
// The system's queries must be disjoint to any other concurrently running system's
unsafe fn run_system<'a, R>(world: &'a World, sys: &'a systems::ErasedSystem<'_, R>) -> R {
    #[cfg(feature = "tracing")]
    let name = sys.descriptor.name.clone();
    #[cfg(feature = "tracing")]
    tracing::trace!(system_name = name.as_str(), "Running system");

    let index = sys.commands_index;
    let execute: &systems::InnerSystem<'_, R> = { transmute(sys.execute.as_ref()) };

    let res = (execute)(world, index);

    #[cfg(feature = "tracing")]
    tracing::trace!(system_name = name, "Running system done");

    res
}
