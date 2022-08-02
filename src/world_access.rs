use crate::query::WorldQuery;

/// Gives unique access to the world
///
/// Systems that take `WorldAccess` can not have other queries and will run in their own schedule
/// group.
///
///
/// ```
/// use cecs::prelude::*;
///
/// fn sys(mut access: WorldAccess) {
///     let w = access.world_mut();
///     // you can use the world as you would normally 
///     w.run_system(||{});
/// }
///
/// let mut world = World::new(0);
/// ```
///
/// ## Limitation
/// 
/// You must not stack WorldAccess queries! Do not run a system that needs WorldAccess inside
/// another system with WorldAccess as they might violate Rust's borrow rules.
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
