use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use super::WorldQuery;

pub struct Res<'a, T> {
    inner: &'a T,
    _m: PhantomData<T>,
}

impl<'a, T: 'static> WorldQuery<'a> for Res<'a, T> {
    fn new(db: &'a crate::World) -> Self {
        Self::new(db)
    }

    fn exclusive_components(_set: &mut std::collections::HashSet<std::any::TypeId>) {
        // noop
    }

    fn exclusive_resources(_set: &mut std::collections::HashSet<std::any::TypeId>) {
        // noop
    }
}

impl<'a, T: 'static> Res<'a, T> {
    pub fn new(world: &'a crate::World) -> Self {
        let inner = world.resources.fetch().unwrap();
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

pub struct ResMut<'a, T> {
    inner: &'a mut T,
    _m: PhantomData<T>,
}

impl<'a, T: 'static> ResMut<'a, T> {
    pub fn new(world: &'a crate::World) -> Self {
        let inner = world.resources.fetch_mut().unwrap();
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

impl<'a, T: 'static> WorldQuery<'a> for ResMut<'a, T> {
    fn new(db: &'a crate::World) -> Self {
        Self::new(db)
    }

    fn exclusive_components(_set: &mut std::collections::HashSet<std::any::TypeId>) {
        // noop
    }

    fn exclusive_resources(set: &mut std::collections::HashSet<std::any::TypeId>) {
        set.insert(std::any::TypeId::of::<T>());
    }
}
