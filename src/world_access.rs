use crate::query::WorldQuery;

pub struct WorldAccess {
    world: std::ptr::NonNull<crate::World>,
}

impl WorldAccess {
    pub fn world(&self) -> &crate::World {
        unsafe { self.world.as_ref() }
    }

    pub fn world_mut(&mut self) -> &mut crate::World {
        unsafe { self.world.as_mut() }
    }
}

unsafe impl Send for WorldAccess {}
unsafe impl Sync for WorldAccess {}

impl<'a> WorldQuery<'a> for WorldAccess {
    fn new(db: &'a crate::World, _commands_index: usize) -> Self {
        WorldAccess { world: db.into() }
    }

    fn components_mut(_set: &mut std::collections::HashSet<std::any::TypeId>) {
        // noop
    }

    fn components_const(_set: &mut std::collections::HashSet<std::any::TypeId>) {
        // noop
    }

    fn resources_mut(_set: &mut std::collections::HashSet<std::any::TypeId>) {
        // noop
    }

    fn resources_const(_set: &mut std::collections::HashSet<std::any::TypeId>) {
        // noop
    }

    fn exclusive() -> bool {
        true
    }
}
