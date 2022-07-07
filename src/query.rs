#[cfg(test)]
mod query_tests;

use crate::{db::ArchetypeStorage, Component};
use std::{any::TypeId, marker::PhantomData};

#[derive(Clone, Copy)]
pub struct Ref<'a, T: 'static> {
    inner: &'static T,
    _m: PhantomData<&'a ()>,
}

impl<'a, T: 'static + std::fmt::Debug> std::fmt::Debug for Ref<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.inner)
    }
}

impl<'a, T: 'static> AsRef<T> for Ref<'a, T> {
    fn as_ref(&self) -> &T {
        self.inner
    }
}

impl<'a, T: 'static> std::ops::Deref for Ref<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner
    }
}

pub struct Mut<'a, T: 'static> {
    inner: &'static mut T,
    _m: PhantomData<&'a mut ()>,
}

impl<'a, T: 'static + std::fmt::Debug> std::fmt::Debug for Mut<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.inner)
    }
}

impl<'a, T: 'static> AsRef<T> for Mut<'a, T> {
    fn as_ref(&self) -> &T {
        self.inner
    }
}

impl<'a, T: 'static> std::ops::Deref for Mut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner
    }
}

impl<'a, T: 'static> std::ops::DerefMut for Mut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner
    }
}

pub struct QueryIt<'a, T> {
    inner: Option<Box<dyn Iterator<Item = (u32, &'a T)> + 'a>>,
    _m: PhantomData<&'a ()>,
}

impl<'a, T: 'static> Iterator for QueryIt<'a, T> {
    type Item = Ref<'a, T>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.as_mut().and_then(|it| it.next()).map(|(_, x)| {
            let x: &'static T = unsafe { std::mem::transmute(x) };
            Ref {
                inner: x,
                _m: PhantomData,
            }
        })
    }
}

pub struct QueryItMut<'a, T> {
    inner: Option<Box<dyn Iterator<Item = (u32, &'a mut T)> + 'a>>,
    _m: PhantomData<&'a ()>,
}

impl<'a, T: 'static> Iterator for QueryItMut<'a, T> {
    type Item = Mut<'a, T>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.as_mut().and_then(|it| it.next()).map(|(_, x)| {
            let x: &'static mut T = unsafe { std::mem::transmute(x) };
            Mut {
                inner: x,
                _m: PhantomData,
            }
        })
    }
}

pub trait Queryable<'a, T> {
    type Item;
    type ItemMut;
    type It: Iterator<Item = Self::Item>;
    type ItMut: Iterator<Item = Self::ItemMut>;

    fn iter(&'a self) -> Self::It;
    fn iter_mut(&'a mut self) -> Self::ItMut;
}

impl<'a, T: Component> Queryable<'a, &'a T> for ArchetypeStorage {
    type Item = Ref<'a, T>;
    type ItemMut = Mut<'a, T>;
    type It = QueryIt<'a, T>;
    type ItMut = QueryItMut<'a, T>;

    fn iter(&'a self) -> Self::It {
        let inner = self
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { columns.as_inner::<T>().iter() });
        let inner = inner.map(|fos| {
            let res: Box<dyn Iterator<Item = (u32, &'a T)>> = Box::new(fos);
            res
        });
        QueryIt {
            inner,
            _m: PhantomData,
        }
    }

    fn iter_mut(&'a mut self) -> Self::ItMut {
        let inner = self
            .components
            .get_mut(&TypeId::of::<T>())
            .map(|columns| unsafe { columns.as_inner_mut::<T>().iter_mut() });
        let inner = inner.map(|fos| {
            let res: Box<dyn Iterator<Item = (u32, &'a mut T)>> = Box::new(fos);
            res
        });
        QueryItMut {
            inner,
            _m: PhantomData,
        }
    }
}

pub struct ComponentQuery<T> {
    _m: PhantomData<T>,
}

impl<T> Default for ComponentQuery<T> {
    fn default() -> Self {
        Self { _m: PhantomData }
    }
}

impl<'a, T: 'static> ComponentQuery<&'a T>
where
    ArchetypeStorage: Queryable<'a, &'a T>,
{
    pub fn iter(
        &self,
        archetype: &'a ArchetypeStorage,
    ) -> <ArchetypeStorage as Queryable<'a, &'a T>>::It {
        archetype.iter()
    }
}

impl<'a, T: 'static> ComponentQuery<&'a mut T>
where
    ArchetypeStorage: Queryable<'a, &'a T>,
{
    pub fn iter(
        &self,
        archetype: &'a ArchetypeStorage,
    ) -> <ArchetypeStorage as Queryable<'a, &'a T>>::It {
        archetype.iter()
    }

    pub fn iter_mut(
        &self,
        archetype: &'a mut ArchetypeStorage,
    ) -> <ArchetypeStorage as Queryable<'a, &'a T>>::ItMut {
        archetype.iter_mut()
    }
}

// TODO: macro implementing more combinations
impl<'a, T1: 'static, T2: 'static> ComponentQuery<(&'a T1, &'a T2)>
where
    ArchetypeStorage: Queryable<'a, &'a T1> + Queryable<'a, &'a T2>,
{
    pub fn iter(
        &self,
        archetype: &'a ArchetypeStorage,
    ) -> impl Iterator<
        Item = (
            <ArchetypeStorage as Queryable<'a, &'a T1>>::Item,
            <ArchetypeStorage as Queryable<'a, &'a T2>>::Item,
        ),
    > {
        let it1 = ComponentQuery::<&'a T1>::default().iter(archetype);
        let it2 = ComponentQuery::<&'a T2>::default().iter(archetype);
        it1.zip(it2)
    }
}
