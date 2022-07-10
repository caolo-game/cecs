#![feature(option_get_or_insert_default)]
#![feature(const_type_id)]

use std::{any::TypeId, collections::HashMap, pin::Pin, ptr::NonNull, sync::Mutex};

use commands::{EntityCommands, ErasedResourceCommand};
use db::ArchetypeStorage;
use entity_id::EntityId;
use handle_table::EntityIndex;
use resources::ResourceStorage;

pub mod commands;
pub(crate) mod db;
pub mod entity_id;
pub mod handle_table;
pub mod prelude;
pub mod query;
pub mod resources;
pub mod systems;

#[cfg(test)]
mod world_tests;

pub struct World {
    pub(crate) entity_ids: EntityIndex,
    pub(crate) archetypes: HashMap<TypeHash, Pin<Box<ArchetypeStorage>>>,
    pub(crate) resources: ResourceStorage,
    pub(crate) commands: Mutex<Vec<EntityCommands>>,
    pub(crate) resource_commands: Mutex<Vec<ErasedResourceCommand>>,
}

impl Clone for World {
    fn clone(&self) -> Self {
        let archetypes = self.archetypes.clone();
        let commands = Mutex::default();
        let resource_commands = Mutex::default();

        let mut entity_ids = self.entity_ids.clone();
        for (ptr, row_index, id) in self.entity_ids.metadata.iter() {
            let ty = unsafe { &**ptr }.ty();
            let new_arch = &archetypes[&ty];
            entity_ids
                .update(
                    *id,
                    (NonNull::from(new_arch.as_ref().get_ref()), *row_index),
                )
                .unwrap();
        }

        let resources = self.resources.clone();

        Self {
            entity_ids,
            archetypes,
            commands,
            resources,
            resource_commands,
        }
    }
}

type TypeHash = u64;

const fn hash_ty<T: 'static>() -> u64 {
    let ty = TypeId::of::<T>();
    // FIXME extreme curse
    //
    let ty: u64 = unsafe { std::mem::transmute(ty) };
    if ty == unsafe { std::mem::transmute(TypeId::of::<()>()) } {
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

/// The end goal is to have a clonable ECS, that's why we have the Clone restriction.
pub trait Component: 'static + Clone {}
impl<T: 'static + Clone> Component for T {}

pub trait Index {
    type Id;
    type Error;

    fn allocate(&mut self) -> Result<Self::Id, Self::Error>;
    fn update(
        &mut self,
        id: Self::Id,
        payload: (NonNull<ArchetypeStorage>, RowIndex),
    ) -> Result<(), Self::Error>;
    fn delete(&mut self, id: Self::Id) -> Result<(), Self::Error>;
    fn read(&self, id: Self::Id) -> Result<(NonNull<ArchetypeStorage>, RowIndex), Self::Error>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
}

impl World {
    pub fn new(initial_capacity: u32) -> Pin<Box<Self>> {
        // FIXME: can't add assert to const fn...
        // the `hash_ty` function assumes that TypeId is a u64 under the hood
        debug_assert_eq!(std::mem::size_of::<TypeId>(), std::mem::size_of::<u64>());

        let entity_ids = EntityIndex::new(initial_capacity);

        let archetypes = HashMap::with_capacity(128);
        let result = Self {
            entity_ids,
            archetypes,
            resources: ResourceStorage::new(),
            commands: Mutex::default(),
            resource_commands: Mutex::default(),
        };
        let mut result = Box::pin(result);
        let void_store = Box::pin(ArchetypeStorage::empty());
        result.archetypes.insert(VOID_TY, void_store);
        result
    }

    pub fn is_id_valid(&self, id: EntityId) -> bool {
        self.entity_ids.is_valid(id)
    }

    pub fn apply_commands(&mut self) -> WorldResult<()> {
        let commands = std::mem::take(&mut *self.commands.lock().unwrap());
        for cmd in commands {
            cmd.apply(self)?;
        }
        let commands = std::mem::take(&mut *self.resource_commands.lock().unwrap());
        for cmd in commands {
            cmd.apply(self)?;
        }
        Ok(())
    }

    pub fn insert_entity(&mut self) -> WorldResult<EntityId> {
        let id = self
            .entity_ids
            .allocate()
            .map_err(|_| WorldError::OutOfCapacity)?;
        let void_store = self.archetypes.get_mut(&VOID_TY).unwrap();

        let index = void_store.as_mut().insert_entity(id);
        void_store.as_mut().set_component(index, ());
        self.entity_ids
            .update(
                id,
                (
                    NonNull::new(void_store.as_mut().get_mut() as *mut _).unwrap(),
                    index,
                ),
            )
            .unwrap();
        Ok(id)
    }

    pub fn delete_entity(&mut self, id: EntityId) -> WorldResult<()> {
        let (mut archetype, index) = self
            .entity_ids
            .read(id)
            .map_err(|_| WorldError::EntityNotFound)?;
        unsafe {
            if let Some(id) = archetype.as_mut().remove(index) {
                self.entity_ids.update(id, (archetype, index)).unwrap();
            }
            self.entity_ids.delete(id).unwrap();
        }
        Ok(())
    }

    pub fn set_component<T: Component>(
        &mut self,
        entity_id: EntityId,
        component: T,
    ) -> WorldResult<()> {
        let (mut archetype, mut index) = self
            .entity_ids
            .read(entity_id)
            .map_err(|_| WorldError::EntityNotFound)?;
        let mut archetype = unsafe { archetype.as_mut() };
        if !archetype.contains_column::<T>() {
            let new_ty = archetype.extended_hash::<T>();
            if !self.archetypes.contains_key(&new_ty) {
                let (mut res, updated_entity) = self.insert_archetype::<T>(
                    archetype,
                    index,
                    archetype.extend_with_column::<T>(),
                );
                if let Some(updated_entity) = updated_entity {
                    self.entity_ids
                        .update(updated_entity, (NonNull::from(archetype), index))
                        .unwrap();
                }
                archetype = unsafe { res.as_mut() };
                index = 0;
            } else {
                let new_arch = self.archetypes.get_mut(&new_ty).unwrap();
                let (i, updated_entity) = archetype.move_entity(new_arch, index);
                if let Some(updated_entity) = updated_entity {
                    self.entity_ids
                        .update(updated_entity, (NonNull::from(archetype), index))
                        .unwrap();
                }
                index = i;
                archetype = new_arch.as_mut().get_mut();
            }
        }
        archetype.set_component(index, component);
        self.entity_ids
            .update(entity_id, (NonNull::from(archetype), index))
            .unwrap();

        Ok(())
    }

    pub fn get_component<T: Component>(&self, entity_id: EntityId) -> Option<&T> {
        let (arch, idx) = self.entity_ids.read(entity_id).ok()?;
        unsafe { arch.as_ref().get_component(idx) }
    }

    pub fn remove_component<T: Component>(&mut self, entity_id: EntityId) -> WorldResult<()> {
        let (mut archetype, mut index) = self
            .entity_ids
            .read(entity_id)
            .map_err(|_| WorldError::EntityNotFound)?;
        let mut archetype = unsafe { archetype.as_mut() };
        if !archetype.contains_column::<T>() {
            return Err(WorldError::ComponentNotFound);
        }
        let new_ty = archetype.extended_hash::<T>();
        if !self.archetypes.contains_key(&new_ty) {
            let (mut res, updated_entity) =
                self.insert_archetype::<T>(archetype, index, archetype.reduce_with_column::<T>());
            if let Some(updated_entity) = updated_entity {
                self.entity_ids
                    .update(updated_entity, (NonNull::from(archetype), index))
                    .unwrap();
            }
            archetype = unsafe { res.as_mut() };
            index = 0;
        } else {
            let new_arch = self.archetypes.get_mut(&new_ty).unwrap();
            let (i, updated_entity) = archetype.move_entity(new_arch, index);
            if let Some(updated_entity) = updated_entity {
                self.entity_ids
                    .update(updated_entity, (NonNull::from(archetype), index))
                    .unwrap();
            }
            index = i;
            archetype = new_arch.as_mut().get_mut();
        }
        unsafe {
            self.entity_ids
                .update(
                    entity_id,
                    (NonNull::new_unchecked(archetype as *mut _), index),
                )
                .unwrap();
        }
        Ok(())
    }

    #[inline(never)]
    fn insert_archetype<T: Component>(
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

    pub fn insert_resource<T: 'static + Clone>(&mut self, value: T) {
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
}
