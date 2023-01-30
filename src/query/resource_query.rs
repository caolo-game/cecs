use std::{
    any::TypeId,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use super::WorldQuery;

pub struct Res<'a, T> {
    inner: &'a T,
    _m: PhantomData<T>,
}

impl<'a, T: 'static> WorldQuery<'a> for Res<'a, T> {
    fn new(db: &'a crate::World, _commands_index: usize) -> Self {
        Self::new(db)
    }

    fn components_mut(_set: &mut std::collections::HashSet<TypeId>) {
        // noop
    }

    fn resources_mut(_set: &mut std::collections::HashSet<TypeId>) {
        // noop
    }

    fn components_const(_set: &mut std::collections::HashSet<TypeId>) {
        // noop
    }

    fn resources_const(set: &mut std::collections::HashSet<TypeId>) {
        set.insert(TypeId::of::<T>());
    }

    fn exclusive() -> bool {
        false
    }

    fn read_only() -> bool {
        true
    }
}

impl<'a, T: 'static> Res<'a, T> {
    pub fn new(world: &'a crate::World) -> Self {
        let inner = match world.resources.fetch() {
            Some(inner) => inner,
            None => {
                panic!(
                    "Res query on uninitialized type: {}",
                    std::any::type_name::<T>()
                );
            }
        };
        Self {
            inner,
            _m: PhantomData,
        }
    }
}

impl<'a, T: 'static> Deref for Res<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner
    }
}

impl<'a, T: 'static> AsRef<T> for Res<'a, T> {
    fn as_ref(&self) -> &T {
        self.inner
    }
}

pub struct ResMut<'a, T> {
    inner: &'a mut T,
    _m: PhantomData<fn() -> &'a mut T>,
}

impl<'a, T: 'static> ResMut<'a, T> {
    pub fn new(world: &'a crate::World) -> Self {
        let inner = match world.resources.fetch_mut() {
            Some(inner) => inner,
            None => {
                panic!(
                    "ResMut query on uninitialized type: {}",
                    std::any::type_name::<T>()
                );
            }
        };
        Self {
            inner,
            _m: PhantomData,
        }
    }
}

impl<'a, T: 'static> Deref for ResMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner
    }
}

impl<'a, T: 'static> DerefMut for ResMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner
    }
}

impl<'a, T: 'static> AsRef<T> for ResMut<'a, T> {
    fn as_ref(&self) -> &T {
        self.inner
    }
}

impl<'a, T: 'static> AsMut<T> for ResMut<'a, T> {
    fn as_mut(&mut self) -> &mut T {
        self.inner
    }
}

impl<'a, T: 'static> WorldQuery<'a> for ResMut<'a, T> {
    fn new(db: &'a crate::World, _commands_index: usize) -> Self {
        Self::new(db)
    }

    fn components_mut(_set: &mut std::collections::HashSet<TypeId>) {
        // noop
    }

    fn resources_mut(set: &mut std::collections::HashSet<TypeId>) {
        set.insert(TypeId::of::<T>());
    }

    fn resources_const(set: &mut std::collections::HashSet<TypeId>) {
        set.insert(TypeId::of::<T>());
    }

    fn components_const(_set: &mut std::collections::HashSet<TypeId>) {
        // noop
    }

    fn exclusive() -> bool {
        false
    }

    fn read_only() -> bool {
        false
    }
}

impl<'a, T: 'static> WorldQuery<'a> for Option<Res<'a, T>> {
    fn new(world: &'a crate::World, _commands_index: usize) -> Self {
        let inner = world.resources.fetch();
        inner.map(|inner| Res {
            inner,
            _m: PhantomData,
        })
    }

    fn components_mut(_set: &mut std::collections::HashSet<TypeId>) {
        // noop
    }

    fn resources_mut(_set: &mut std::collections::HashSet<TypeId>) {
        // noop
    }

    fn components_const(_set: &mut std::collections::HashSet<TypeId>) {
        // noop
    }

    fn resources_const(set: &mut std::collections::HashSet<TypeId>) {
        set.insert(TypeId::of::<T>());
    }

    fn exclusive() -> bool {
        false
    }

    fn read_only() -> bool {
        true
    }
}

impl<'a, T: 'static> WorldQuery<'a> for Option<ResMut<'a, T>> {
    fn new(world: &'a crate::World, _commands_index: usize) -> Self {
        let inner = world.resources.fetch_mut();
        inner.map(|inner| ResMut {
            inner,
            _m: PhantomData,
        })
    }

    fn components_mut(_set: &mut std::collections::HashSet<TypeId>) {
        // noop
    }

    fn resources_mut(_set: &mut std::collections::HashSet<TypeId>) {
        // noop
    }

    fn components_const(_set: &mut std::collections::HashSet<TypeId>) {
        // noop
    }

    fn resources_const(set: &mut std::collections::HashSet<TypeId>) {
        set.insert(TypeId::of::<T>());
    }

    fn exclusive() -> bool {
        false
    }

    fn read_only() -> bool {
        false
    }
}
