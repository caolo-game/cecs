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
    type It: Iterator<Item = Self::Item>;

    fn iter(&'a self) -> Self::It;
}

impl<'a, T: Component> Queryable<'a, &'a T> for ArchetypeStorage {
    type Item = Ref<'a, T>;
    type It = QueryIt<'a, T>;

    fn iter(&'a self) -> Self::It {
        let inner = self
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { (&mut *columns.get()).as_inner::<T>().iter() });
        let inner = inner.map(|fos| {
            let res: Box<dyn Iterator<Item = (u32, &'a T)>> = Box::new(fos);
            res
        });
        QueryIt {
            inner,
            _m: PhantomData,
        }
    }
}

impl<'a, T: Component> Queryable<'a, &'a mut T> for ArchetypeStorage {
    type Item = Mut<'a, T>;
    type It = QueryItMut<'a, T>;

    fn iter(&'a self) -> Self::It {
        let inner = self
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { (&mut *columns.get()).as_inner_mut::<T>().iter_mut() });
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

pub struct Query<T> {
    _m: PhantomData<T>,
}

pub trait QueryFragment<'a> {
    type Item;
    type It: Iterator<Item = Self::Item> + 'a;

    fn iter(&self, archetype: &'a ArchetypeStorage) -> Self::It;
}

impl<T> Default for Query<T> {
    fn default() -> Self {
        Self { _m: PhantomData }
    }
}

impl<'a, T: Component> QueryFragment<'a> for Query<&'a T> {
    type Item = Ref<'a, T>;
    type It = <ArchetypeStorage as Queryable<'a, &'a T>>::It;

    fn iter(&self, archetype: &'a ArchetypeStorage) -> Self::It {
        <ArchetypeStorage as Queryable<'_, &'a T>>::iter(archetype)
    }
}

impl<'a, T: Component> QueryFragment<'a> for Query<&'a mut T> {
    type Item = Mut<'a, T>;
    type It = <ArchetypeStorage as Queryable<'a, &'a mut T>>::It;

    fn iter(&self, archetype: &'a ArchetypeStorage) -> Self::It {
        <ArchetypeStorage as Queryable<'_, &'a mut T>>::iter(archetype)
    }
}

// TODO: macro implementing more combinations
//
impl<'a, T1, T2> QueryFragment<'a> for Query<(T1, T2)>
where
    Query<T1>: QueryFragment<'a>,
    Query<T2>: QueryFragment<'a>,
    ArchetypeStorage: Queryable<'a, T1>,
    ArchetypeStorage: Queryable<'a, T2>,
{
    type Item = (
        <Query<T1> as QueryFragment<'a>>::Item,
        <Query<T2> as QueryFragment<'a>>::Item,
    );
    type It =
        std::iter::Zip<<Query<T1> as QueryFragment<'a>>::It, <Query<T2> as QueryFragment<'a>>::It>;

    fn iter(&self, archetype: &'a ArchetypeStorage) -> Self::It {
        let it1 = Query::<T1>::default().iter(archetype);
        let it2 = Query::<T2>::default().iter(archetype);
        it1.zip(it2)
    }
}
