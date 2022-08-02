use std::ptr::NonNull;

use crate::{
    entity_id::EntityId, prelude::Bundle, query::WorldQuery, CommandBuffer, Component, World,
    WorldError,
};

pub struct Commands<'a> {
    cmd: &'a CommandBuffer<CommandPayload>,
}

unsafe impl<'a> Send for Commands<'a> {}
unsafe impl<'a> Sync for Commands<'a> {}

impl<'a> WorldQuery<'a> for Commands<'a> {
    fn new(w: &'a World, commands_index: usize) -> Self {
        Self::new(w, commands_index)
    }

    fn components_mut(_set: &mut std::collections::HashSet<std::any::TypeId>) {
        // noop
    }

    fn resources_mut(_set: &mut std::collections::HashSet<std::any::TypeId>) {
        // noop
    }

    fn components_const(_set: &mut std::collections::HashSet<std::any::TypeId>) {
        // noop
    }

    fn resources_const(_set: &mut std::collections::HashSet<std::any::TypeId>) {
        // noop
    }

    fn exclusive() -> bool {
        false
    }
}

impl<'a> Commands<'a> {
    pub(crate) fn new(w: &'a World, commands_index: usize) -> Self {
        Self {
            cmd: &w.commands[commands_index],
        }
    }

    pub fn entity(&mut self, id: EntityId) -> &mut EntityCommands {
        unsafe {
            let cmd = &mut *self.cmd.get();
            cmd.push(CommandPayload::Entity(EntityCommands {
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
}

pub(crate) enum CommandPayload {
    Entity(EntityCommands),
    Resource(ErasedResourceCommand),
}

impl CommandPayload {
    pub(crate) fn apply(self, world: &mut World) -> Result<(), WorldError> {
        match self {
            CommandPayload::Entity(c) => c.apply(world),
            CommandPayload::Resource(c) => c.apply(world),
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
    /// if the action is delete, then `payload` is ignored
    action: EntityAction,
    payload: Vec<ErasedComponentCommand>,
}

enum EntityAction {
    Fetch(EntityId),
    Insert,
    Delete(EntityId),
}

impl EntityCommands {
    pub(crate) fn apply(self, world: &mut World) -> Result<(), WorldError> {
        let id = match self.action {
            EntityAction::Fetch(id) => id,
            EntityAction::Insert => world.insert_entity()?,
            EntityAction::Delete(id) => return world.delete_entity(id),
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

    pub fn insert_bundle<T: Bundle>(&mut self, component: T) -> &mut Self {
        self.payload
            .push(ErasedComponentCommand::from_bundle(BundleCommand::Insert(
                component,
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
    inner: *mut u8,
    apply: fn(NonNull<u8>, EntityId, &mut World) -> Result<(), WorldError>,
    drop: fn(NonNull<u8>),
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
                world.remove_component::<T>(entity_id)?;
            }
        }
        Ok(())
    }
}

pub(crate) struct ErasedResourceCommand {
    inner: *mut u8,
    apply: fn(NonNull<u8>, &mut World) -> Result<(), WorldError>,
    drop: fn(NonNull<u8>),
}

unsafe impl Send for ErasedResourceCommand {}
unsafe impl Sync for ErasedResourceCommand {}

impl Drop for ErasedResourceCommand {
    fn drop(&mut self) {
        (self.drop)(NonNull::new(self.inner).unwrap());
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
                let ptr: Box<ResourceCommand<T>> = unsafe { Box::from_raw(ptr.as_mut()) };
                ptr.apply(world)?;
                Ok(())
            },
        }
    }

    pub fn apply(self, world: &mut World) -> Result<(), WorldError> {
        (self.apply)(NonNull::new(self.inner).unwrap(), world)
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
                world.remove_resource::<T>();
            }
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

        let id = world.insert_entity().unwrap();
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

        let id = world.insert_entity().unwrap();
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
}
