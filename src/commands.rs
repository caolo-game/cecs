use std::ptr::NonNull;

use crate::{entity_id::EntityId, query::WorldQuery, Component, Mutex, World, WorldError};

pub struct Commands<'a> {
    world: &'a Mutex<Vec<EntityCommands>>,
    entity_cmd: Vec<EntityCommands>,
    resource_cmd: Vec<ErasedResourceCommand>,
}

impl Drop for Commands<'_> {
    fn drop(&mut self) {
        self.world
            .lock()
            .extend(std::mem::take(&mut self.entity_cmd).into_iter());
    }
}

impl<'a> WorldQuery<'a> for Commands<'a> {
    fn new(w: &'a World) -> Self {
        Self::new(w)
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
}

impl<'a> Commands<'a> {
    pub fn new(w: &'a World) -> Self {
        Self {
            world: &w.commands,
            entity_cmd: Vec::default(),
            resource_cmd: Vec::default(),
        }
    }

    pub fn entity(&mut self, id: EntityId) -> &mut EntityCommands {
        self.entity_cmd.push(EntityCommands {
            action: EntityAction::Fetch(id),
            payload: Vec::default(),
        });
        self.entity_cmd.last_mut().unwrap()
    }

    pub fn spawn(&mut self) -> &mut EntityCommands {
        self.entity_cmd.push(EntityCommands {
            action: EntityAction::Insert,
            payload: Vec::default(),
        });
        self.entity_cmd.last_mut().unwrap()
    }

    pub fn delete(&mut self, id: EntityId) {
        self.entity_cmd.push(EntityCommands {
            action: EntityAction::Delete(id),
            payload: Vec::default(),
        });
    }

    pub fn insert_resource<T: Component>(&mut self, resource: T) {
        self.resource_cmd
            .push(ErasedResourceCommand::new(ResourceCommand::Insert(
                resource,
            )));
    }

    pub fn remove_resource<T: Component>(&mut self) {
        self.resource_cmd
            .push(ErasedResourceCommand::new(ResourceCommand::<T>::Delete));
    }
}

pub struct EntityCommands {
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
        self.payload
            .push(ErasedComponentCommand::new(ComponentCommand::Insert(
                component,
            )));
        self
    }

    pub fn remove<T: Component>(&mut self) -> &mut Self {
        self.payload
            .push(ErasedComponentCommand::new(ComponentCommand::<T>::Delete));
        self
    }
}

pub(crate) struct ErasedComponentCommand {
    inner: *mut u8,
    apply: fn(NonNull<u8>, EntityId, &mut World) -> Result<(), WorldError>,
    drop: fn(NonNull<u8>),
}

impl Drop for ErasedComponentCommand {
    fn drop(&mut self) {
        if !self.inner.is_null() {
            (self.drop)(NonNull::new(self.inner).unwrap());
        }
    }
}

impl ErasedComponentCommand {
    pub fn apply(mut self, id: EntityId, world: &mut World) -> Result<(), WorldError> {
        let result = (self.apply)(NonNull::new(self.inner).unwrap(), id, world);
        self.inner = std::ptr::null_mut();
        result
    }

    pub fn new<T: Component>(inner: ComponentCommand<T>) -> Self {
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
}

pub(crate) enum ComponentCommand<T> {
    Insert(T),
    Delete,
}

impl<T: Component> ComponentCommand<T> {
    fn apply(self, entity_id: EntityId, world: &mut World) -> Result<(), WorldError> {
        match self {
            ComponentCommand::Insert(comp) => {
                world.set_component::<T>(entity_id, comp)?;
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

        let mut cmd = Commands::new(&mut world);
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

        let mut cmd = Commands::new(&mut world);
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

        let mut cmd = Commands::new(&mut world);
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
