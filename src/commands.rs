use std::{collections::HashSet, ptr::NonNull};

use crate::{
    entity_id::EntityId, prelude::Bundle, query::WorldQuery, CommandBuffer, Component, World,
    WorldError,
};

pub struct Commands<'a> {
    world: &'a World,
    cmd: &'a CommandBuffer<CommandPayload>,
}

unsafe impl<'a> Send for Commands<'a> {}
unsafe impl<'a> Sync for Commands<'a> {}

impl<'a> WorldQuery<'a> for Commands<'a> {
    fn new(w: &'a World, commands_index: usize) -> Self {
        Self::new(w, commands_index)
    }

    fn components_mut(_set: &mut HashSet<std::any::TypeId>) {
        // noop
    }

    fn resources_mut(_set: &mut HashSet<std::any::TypeId>) {
        // noop
    }

    fn components_const(_set: &mut HashSet<std::any::TypeId>) {
        // noop
    }

    fn resources_const(_set: &mut HashSet<std::any::TypeId>) {
        // noop
    }

    fn exclusive() -> bool {
        false
    }

    fn read_only() -> bool {
        false
    }
}

impl<'a> Commands<'a> {
    pub(crate) fn new(w: &'a World, commands_index: usize) -> Self {
        Self {
            world: &w,
            cmd: &w.commands[commands_index],
        }
    }

    /// Reserve storage for  `additional` number of additional entities.
    ///
    /// Reserve happens lazily, this can be used in conjunction with `spawn` but not with `id()`
    ///
    ///
    /// ```
    /// # use cecs::prelude::*;
    /// fn sys(mut cmd: Commands) {
    ///     cmd.reserve_entities(128);
    ///     for i in 0..128 {
    ///         // this is fine
    ///         cmd.spawn().insert(i);
    ///     }
    /// }
    /// # let mut world = World::new(0);
    /// # world.run_system(sys);
    /// ```
    ///
    /// ```should_panic
    /// # use cecs::prelude::*;
    /// fn sys(mut cmd: Commands) {
    ///     cmd.reserve_entities(128);
    ///     for i in 0..128 {
    ///         // id will return an error, since the reserve has not happened yet
    ///         cmd.spawn().id().unwrap();
    ///     }
    /// }
    /// # let mut world = World::new(0);
    /// # world.run_system(sys);
    /// ```
    pub fn reserve_entities(&mut self, additional: u32) {
        unsafe {
            let cmd = &mut *self.cmd.get();
            cmd.push(CommandPayload::World(WorldCommands::Reserve { additional }));
        }
    }

    pub fn entity(&mut self, id: EntityId) -> &mut EntityCommands {
        unsafe {
            let cmd = &mut *self.cmd.get();
            cmd.push(CommandPayload::Entity(EntityCommands {
                world: self.world,
                action: EntityAction::Fetch(id),
                payload: Vec::default(),
            }));
            cmd.last_mut().unwrap().entity_mut()
        }
    }

    pub fn spawn(&mut self) -> &mut EntityCommands {
        unsafe {
            let cmd = &mut *self.cmd.get();
            cmd.push(CommandPayload::Entity(EntityCommands {
                world: self.world,
                action: EntityAction::Insert,
                payload: Vec::default(),
            }));
            cmd.last_mut().unwrap().entity_mut()
        }
    }

    pub fn delete(&mut self, id: EntityId) {
        unsafe {
            let cmd = &mut *self.cmd.get();
            cmd.push(CommandPayload::Entity(EntityCommands {
                world: self.world,
                action: EntityAction::Delete(id),
                payload: Vec::default(),
            }));
        }
    }

    pub fn insert_resource<T: Component>(&mut self, resource: T) {
        unsafe {
            let cmd = &mut *self.cmd.get();
            cmd.push(CommandPayload::Resource(ErasedResourceCommand::new(
                ResourceCommand::Insert(resource),
            )));
        }
    }

    pub fn remove_resource<T: Component>(&mut self) {
        unsafe {
            let cmd = &mut *self.cmd.get();
            cmd.push(CommandPayload::Resource(ErasedResourceCommand::new(
                ResourceCommand::<T>::Delete,
            )));
        }
    }

    pub fn merge_entities(&mut self, src: EntityId, dst: EntityId) {
        unsafe {
            let cmd = &mut *self.cmd.get();
            cmd.push(CommandPayload::Entity(EntityCommands {
                world: self.world,
                action: EntityAction::Merge { src, dst },
                payload: Vec::default(),
            }));
        }
    }
}

pub(crate) enum CommandPayload {
    Entity(EntityCommands),
    Resource(ErasedResourceCommand),
    World(WorldCommands),
}

impl CommandPayload {
    pub(crate) fn apply(self, world: &mut World) -> Result<(), WorldError> {
        match self {
            CommandPayload::Entity(c) => c.apply(world),
            CommandPayload::Resource(c) => c.apply(world),
            CommandPayload::World(c) => c.apply(world),
        }
    }

    fn entity_mut(&mut self) -> &mut EntityCommands {
        match self {
            CommandPayload::Entity(cmd) => cmd,
            _ => panic!("Command is not entity command"),
        }
    }
}

pub struct EntityCommands {
    /// if the action is delete or merge, then `payload` is ignored
    action: EntityAction,
    world: *const World,
    payload: Vec<ErasedComponentCommand>,
}

enum EntityAction {
    Fetch(EntityId),
    /// Like fetch, but initialize the id first
    /// Insert actions can become Init actions if the id is requested
    Init(EntityId),
    Insert,
    Delete(EntityId),
    Merge {
        src: EntityId,
        dst: EntityId,
    },
}

impl EntityCommands {
    /// Note: fetching the `id` will force entity allocation, which can trigger out of capacity
    /// error, even if you have reserved in this system stage.
    ///
    /// Ensure that you allocate enough capacity in a previous stage (or tick) before calling
    /// `id()`
    pub fn id(&mut self) -> Result<EntityId, crate::entity_index::HandleTableError> {
        match self.action {
            EntityAction::Init(id) | EntityAction::Fetch(id) | EntityAction::Delete(id) => Ok(id),
            EntityAction::Insert => unsafe {
                let world = &*self.world;
                let _guard = world.this_lock.lock();
                let index = world.entity_ids.get();
                let id = (*index).allocate()?;
                self.action = EntityAction::Init(id);
                Ok(id)
            },
            EntityAction::Merge { src: _, dst } => Ok(dst),
        }
    }

    pub(crate) fn apply(self, world: &mut World) -> Result<(), WorldError> {
        let id = match self.action {
            EntityAction::Fetch(id) => id,
            EntityAction::Init(id) => {
                unsafe {
                    world.init_id(id);
                }
                // ensure entity buffer growth
                world.reserve_entities(1);
                id
            }
            EntityAction::Insert => world.insert_entity(),
            EntityAction::Delete(id) => {
                if let Err(_err) = world.delete_entity(id) {
                    #[cfg(feature = "tracing")]
                    tracing::debug!(
                        id = tracing::field::display(id),
                        error = tracing::field::display(_err),
                        "Entity can't be deleted"
                    );
                }
                return Ok(());
            }
            EntityAction::Merge { src, dst } => {
                return world.merge_entities(src, dst);
            }
        };
        if !world.is_id_valid(id) {
            return Err(WorldError::EntityNotFound);
        }
        for cmd in self.payload {
            cmd.apply(id, world)?;
        }
        Ok(())
    }

    pub fn insert<T: Component>(&mut self, component: T) -> &mut Self {
        self.payload.push(ErasedComponentCommand::from_component(
            ComponentCommand::Insert(component),
        ));
        self
    }

    pub fn insert_bundle<T: Bundle>(&mut self, bundle: T) -> &mut Self {
        self.payload
            .push(ErasedComponentCommand::from_bundle(BundleCommand::Insert(
                bundle,
            )));
        self
    }

    pub fn remove<T: Component>(&mut self) -> &mut Self {
        self.payload.push(ErasedComponentCommand::from_component(
            ComponentCommand::<T>::Delete,
        ));
        self
    }
}

pub(crate) struct ErasedComponentCommand {
    inner: *mut (),
    apply: fn(NonNull<()>, EntityId, &mut World) -> Result<(), WorldError>,
    drop: fn(NonNull<()>),
}

unsafe impl Send for ErasedComponentCommand {}
unsafe impl Sync for ErasedComponentCommand {}

impl Drop for ErasedComponentCommand {
    fn drop(&mut self) {
        if !self.inner.is_null() {
            (self.drop)(NonNull::new(self.inner).unwrap());
        }
    }
}

impl ErasedComponentCommand {
    pub fn apply(mut self, id: EntityId, world: &mut World) -> Result<(), WorldError> {
        let ptr = NonNull::new(self.inner).unwrap();
        self.inner = std::ptr::null_mut();
        (self.apply)(ptr, id, world)
    }

    pub fn from_component<T: Component>(inner: ComponentCommand<T>) -> Self {
        let inner = (Box::leak(Box::new(inner)) as *mut ComponentCommand<T>).cast();
        Self {
            inner,
            drop: |ptr| {
                let mut ptr = ptr.cast();
                let _ptr: Box<ComponentCommand<T>> = unsafe { Box::from_raw(ptr.as_mut()) };
            },
            apply: |ptr, id, world| {
                let mut ptr = ptr.cast();
                let ptr: Box<ComponentCommand<T>> = unsafe { Box::from_raw(ptr.as_mut()) };
                ptr.apply(id, world)?;
                Ok(())
            },
        }
    }

    pub fn from_bundle<T: Bundle>(inner: BundleCommand<T>) -> Self {
        let inner = (Box::leak(Box::new(inner)) as *mut BundleCommand<T>).cast();
        Self {
            inner,
            drop: |ptr| {
                let mut ptr = ptr.cast();
                let _ptr: Box<BundleCommand<T>> = unsafe { Box::from_raw(ptr.as_mut()) };
            },
            apply: |ptr, id, world| {
                let mut ptr = ptr.cast();
                let ptr: Box<BundleCommand<T>> = unsafe { Box::from_raw(ptr.as_mut()) };
                ptr.apply(id, world)?;
                Ok(())
            },
        }
    }
}

pub(crate) enum BundleCommand<T> {
    Insert(T),
}

impl<T: Bundle> BundleCommand<T> {
    fn apply(self, entity_id: EntityId, world: &mut World) -> Result<(), WorldError> {
        match self {
            BundleCommand::Insert(bundle) => {
                world.set_bundle(entity_id, bundle)?;
            }
        }
        Ok(())
    }
}

pub(crate) enum ComponentCommand<T> {
    Insert(T),
    Delete,
}

impl<T: Component> ComponentCommand<T> {
    fn apply(self, entity_id: EntityId, world: &mut World) -> Result<(), WorldError> {
        match self {
            ComponentCommand::Insert(comp) => {
                world.set_component(entity_id, comp)?;
            }
            ComponentCommand::Delete => {
                if let Err(err) = world.remove_component::<T>(entity_id) {
                    match err {
                        WorldError::ComponentNotFound => { /*ignore*/ }
                        WorldError::OutOfCapacity | WorldError::EntityNotFound => return Err(err),
                    }
                }
            }
        }
        Ok(())
    }
}

pub(crate) struct ErasedResourceCommand {
    inner: *mut (),
    apply: fn(NonNull<()>, &mut World) -> Result<(), WorldError>,
    drop: fn(NonNull<()>),
}

unsafe impl Send for ErasedResourceCommand {}
unsafe impl Sync for ErasedResourceCommand {}

impl Drop for ErasedResourceCommand {
    fn drop(&mut self) {
        if !self.inner.is_null() {
            (self.drop)(NonNull::new(self.inner).unwrap());
        }
    }
}

impl ErasedResourceCommand {
    pub fn new<T: Component>(inner: ResourceCommand<T>) -> Self {
        let inner = (Box::leak(Box::new(inner)) as *mut ResourceCommand<T>).cast();
        Self {
            inner,
            drop: |ptr| {
                let mut ptr = ptr.cast();
                let _ptr: Box<ResourceCommand<T>> = unsafe { Box::from_raw(ptr.as_mut()) };
            },
            apply: |ptr, world| {
                let mut ptr = ptr.cast();
                let cmd: Box<ResourceCommand<T>> = unsafe { Box::from_raw(ptr.as_mut()) };
                cmd.apply(world)?;
                Ok(())
            },
        }
    }

    pub fn apply(mut self, world: &mut World) -> Result<(), WorldError> {
        // self.inner will be dropped twice unless we clear it now
        let ptr = self.inner;
        self.inner = std::ptr::null_mut();
        (self.apply)(NonNull::new(ptr).unwrap(), world)
    }
}

pub(crate) enum ResourceCommand<T> {
    Insert(T),
    Delete,
}

impl<T: Component> ResourceCommand<T> {
    fn apply(self, world: &mut World) -> Result<(), WorldError> {
        match self {
            ResourceCommand::Insert(comp) => {
                world.insert_resource::<T>(comp);
            }
            ResourceCommand::Delete => {
                let _ = world.remove_resource::<T>();
            }
        }
        Ok(())
    }
}

pub(crate) enum WorldCommands {
    Reserve { additional: u32 },
}

impl WorldCommands {
    pub fn apply(self, world: &mut World) -> Result<(), WorldError> {
        match self {
            WorldCommands::Reserve { additional } => world.reserve_entities(additional),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::query::Query;

    use super::*;

    #[test]
    fn can_add_entity_via_cmd_test() {
        let mut world = World::new(100);

        let mut cmd = world.ensure_commands();
        cmd.spawn().insert(69i32);
        cmd.spawn().insert(69i32);
        cmd.spawn().insert(69i32);

        drop(cmd);

        world.apply_commands().unwrap();

        let mut cnt = 0;
        for i in Query::<&i32>::new(&world).iter() {
            cnt += 1;
            assert_eq!(i, &69);
        }

        assert_eq!(cnt, 3);
    }

    #[test]
    fn can_remove_component_test() {
        let mut world = World::new(100);

        let id = world.insert_entity();
        world.set_component(id, 69i32).unwrap();

        let _c = Query::<&i32>::new(&world).fetch(id).unwrap();

        let mut cmd = world.ensure_commands();
        cmd.entity(id).remove::<i32>();
        drop(cmd);
        world.apply_commands().unwrap();

        let c = Query::<&i32>::new(&world).fetch(id);
        assert!(c.is_none());

        // entity still exists
        let _c = Query::<&()>::new(&world).fetch(id).unwrap();
    }

    #[test]
    fn can_delete_entity_test() {
        let mut world = World::new(100);

        let id = world.insert_entity();
        world.set_component(id, 69i32).unwrap();

        let _c = Query::<&i32>::new(&world).fetch(id).unwrap();

        let mut cmd = world.ensure_commands();
        cmd.delete(id);
        drop(cmd);
        world.apply_commands().unwrap();

        let c = Query::<&i32>::new(&world).fetch(id);
        assert!(c.is_none());

        // entity should not exists
        let c = Query::<&()>::new(&world).fetch(id);
        assert!(c.is_none());
    }

    #[test]
    fn double_insert_test() {
        let mut world = World::new(0);

        {
            let mut cmd = world.ensure_commands();
            cmd.insert_resource(0i32);

            drop(cmd);
            world.apply_commands().unwrap();
        }
        {
            let mut cmd = world.ensure_commands();
            cmd.insert_resource(42i32);

            drop(cmd);
            world.apply_commands().unwrap();
        }

        let i = world.get_resource::<i32>().unwrap();

        assert_eq!(*i, 42);
    }

    #[test]
    fn merge_entities_test() {
        let mut world = World::new(16);

        let a = world.insert_entity();
        let b = world.insert_entity();
        // control
        let c = world.insert_entity();

        world.set_component(a, 1u64).unwrap();
        world.set_component(a, 1u32).unwrap();
        world.set_component(b, 2i32).unwrap();
        world.set_component(b, 2i64).unwrap();

        world.set_component(c, 3u64).unwrap();
        world.set_component(c, 3u32).unwrap();
        world.set_component(c, 3i64).unwrap();
        world.set_component(c, 3i32).unwrap();

        {
            let mut cmd = world.ensure_commands();
            cmd.merge_entities(a, b);

            drop(cmd);
            world.apply_commands().unwrap();
        }

        assert!(!world.is_id_valid(a), "Entity a should have been deleted");

        // test if c entity is intact
        let comp = world.get_component::<u64>(c).unwrap();
        assert_eq!(comp, &3);
        let comp = world.get_component::<u32>(c).unwrap();
        assert_eq!(comp, &3);
        let comp = world.get_component::<i64>(c).unwrap();
        assert_eq!(comp, &3);
        let comp = world.get_component::<i32>(c).unwrap();
        assert_eq!(comp, &3);

        let c = world.get_component::<u64>(b).unwrap();
        assert_eq!(c, &1);
        let c = world.get_component::<u32>(b).unwrap();
        assert_eq!(c, &1);
        let c = world.get_component::<i64>(b).unwrap();
        assert_eq!(c, &2);
        let c = world.get_component::<i32>(b).unwrap();
        assert_eq!(c, &2);
    }
}
