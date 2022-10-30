pub mod filters;
pub mod resource_query;

#[cfg(test)]
mod query_tests;

use crate::{archetype::ArchetypeStorage, entity_id::EntityId, Component, RowIndex, World};
use filters::Filter;
use std::{any::TypeId, collections::HashSet, marker::PhantomData};

pub(crate) trait WorldQuery<'a> {
    fn new(db: &'a World, commands_index: usize) -> Self;

    /// List of component types this query needs exclusive access to
    fn components_mut(set: &mut HashSet<TypeId>);
    /// List of component types this query needs
    fn components_const(set: &mut HashSet<TypeId>);
    /// List of resource types this query needs exclusive access to
    fn resources_mut(set: &mut HashSet<TypeId>);
    /// List of resource types this query needs
    fn resources_const(set: &mut HashSet<TypeId>);
    /// Return wether this system should run in isolation
    fn exclusive() -> bool;
}

#[derive(Default)]
pub struct QueryProperties {
    pub exclusive: bool,
    pub comp_mut: HashSet<TypeId>,
    pub comp_const: HashSet<TypeId>,
    pub res_mut: HashSet<TypeId>,
    pub res_const: HashSet<TypeId>,
}

impl QueryProperties {
    pub fn is_disjoint(&self, other: &QueryProperties) -> bool {
        !self.exclusive
            && !other.exclusive
            && self.comp_mut.is_disjoint(&other.comp_const)
            && self.res_mut.is_disjoint(&other.res_const)
            && self.comp_mut.is_disjoint(&other.comp_mut)
            && self.res_mut.is_disjoint(&other.res_mut)
            && self.comp_const.is_disjoint(&other.comp_mut)
            && self.res_const.is_disjoint(&other.res_mut)
    }

    pub fn extend(&mut self, props: QueryProperties) {
        self.exclusive = self.exclusive || props.exclusive;
        self.comp_mut.extend(props.comp_mut.into_iter());
        self.res_mut.extend(props.res_mut.into_iter());
        self.comp_const.extend(props.comp_const.into_iter());
        self.res_const.extend(props.res_const.into_iter());
    }

    pub fn is_empty(&self) -> bool {
        !self.exclusive
            && self.comp_mut.is_empty()
            && self.res_mut.is_empty()
            && self.res_const.is_empty()
            && self.comp_const.is_empty()
    }
}

/// Test if this query is valid and return its properties
#[inline]
#[allow(unused)]
pub(crate) fn ensure_query_valid<'a, T: WorldQuery<'a>>() -> QueryProperties {
    let mut comp_mut = HashSet::new();
    let mut comp_const = HashSet::new();

    T::components_mut(&mut comp_mut);
    T::components_const(&mut comp_const);

    assert!(
        comp_mut.is_disjoint(&comp_const),
        "A query may not borrow the same type as both mutable and immutable,
{}",
        std::any::type_name::<T>()
    );

    // resources do not need asserts here
    let mut res_mut = HashSet::new();
    let mut res_const = HashSet::new();
    T::resources_mut(&mut res_mut);
    T::resources_const(&mut res_const);
    QueryProperties {
        comp_mut,
        comp_const,
        res_mut,
        res_const,
        exclusive: T::exclusive(),
    }
}

pub struct Query<T, F = ()> {
    world: std::ptr::NonNull<crate::World>,
    _m: PhantomData<(T, F)>,
}

unsafe impl<T, F> Send for Query<T, F> {}
unsafe impl<T, F> Sync for Query<T, F> {}

impl<'a, T, F> WorldQuery<'a> for Query<T, F>
where
    ArchQuery<T>: QueryFragment<'a>,
    F: Filter,
{
    fn new(db: &'a World, _commands_index: usize) -> Self {
        Self::new(db)
    }

    fn exclusive() -> bool {
        false
    }

    fn components_mut(set: &mut HashSet<TypeId>) {
        <ArchQuery<T> as QueryFragment>::types_mut(set);
    }

    fn resources_mut(_set: &mut HashSet<TypeId>) {
        // noop
    }

    fn components_const(set: &mut HashSet<TypeId>) {
        <ArchQuery<T> as QueryFragment>::types_const(set);
    }

    fn resources_const(_set: &mut HashSet<TypeId>) {
        // noop
    }
}

impl<'a, T, F> Query<T, F>
where
    ArchQuery<T>: QueryFragment<'a>,
    F: Filter,
{
    pub fn new(world: &'a crate::World) -> Self {
        Query {
            world: std::ptr::NonNull::from(world),
            _m: PhantomData,
        }
    }

    /// Count the number of entities this query spans
    pub fn count(&self) -> usize {
        unsafe {
            self.world
                .as_ref()
                .archetypes
                .iter()
                .filter(|(_, arch)| F::filter(arch) && ArchQuery::<T>::contains(arch))
                .map(|(_, arch)| arch.len())
                .sum::<usize>()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = <ArchQuery<T> as QueryFragment<'a>>::Item> {
        unsafe {
            self.world
                .as_ref()
                .archetypes
                .iter()
                .filter(|(_, arch)| F::filter(arch))
                .flat_map(|(_, arch)| ArchQuery::<T>::iter(arch))
        }
    }

    /// # LIMITATION
    ///
    /// Currently `iter_mut` requires that all lifetimes in the Query are the same, like so:
    /// ```
    /// use cecs::prelude::*;
    /// fn my_system<'a>(mut q: Query<(&'a mut i32, &'a u32)>) {
    ///     for (foo, _) in q.iter_mut() {
    ///         *foo = 42;
    ///     }
    /// }
    /// let mut world = World::new(0);
    /// world.run_system(my_system);
    /// ```
    pub fn iter_mut(
        &mut self,
    ) -> impl Iterator<Item = <ArchQuery<T> as QueryFragment<'a>>::ItemMut> {
        unsafe {
            self.world
                .as_ref()
                .archetypes
                .iter()
                .filter(|(_, arch)| F::filter(arch))
                .flat_map(|(_, arch)| ArchQuery::<T>::iter_mut(arch))
        }
    }

    pub fn fetch(&self, id: EntityId) -> Option<<ArchQuery<T> as QueryFragment<'a>>::Item> {
        unsafe {
            let (arch, index) = self.world.as_ref().entity_ids.read(id).ok()?;
            if !F::filter(arch.as_ref()) {
                return None;
            }

            ArchQuery::<T>::fetch(arch.as_ref(), index)
        }
    }

    /// # LIMITATION
    ///
    /// Currently `fetch_mut` requires that all lifetimes in the Query are the same, like so:
    /// ```
    /// use cecs::prelude::*;
    /// fn my_system<'a>(mut q: Query<(&'a mut i32, &'a u32)>) {
    ///     if let Some((foo, _)) = q.fetch_mut(EntityId::default()) {
    ///         *foo = 42;
    ///     }
    /// }
    /// let mut world = World::new(0);
    /// world.run_system(my_system);
    /// ```
    pub fn fetch_mut(
        &mut self,
        id: EntityId,
    ) -> Option<<ArchQuery<T> as QueryFragment<'a>>::ItemMut> {
        unsafe {
            let (arch, index) = self.world.as_ref().entity_ids.read(id).ok()?;
            if !F::filter(arch.as_ref()) {
                return None;
            }

            ArchQuery::<T>::fetch_mut(arch.as_ref(), index)
        }
    }

    pub fn contains(&self, id: EntityId) -> bool {
        unsafe {
            let (arch, _index) = match self.world.as_ref().entity_ids.read(id).ok() {
                None => return false,
                Some(x) => x,
            };
            if !F::filter(arch.as_ref()) {
                return false;
            }

            ArchQuery::<T>::contains(arch.as_ref())
        }
    }

    /// fetch the first row of the query
    /// panic if no row was found
    pub fn one(&self) -> <ArchQuery<T> as QueryFragment<'a>>::Item {
        self.iter().next().unwrap()
    }
}

pub struct ArchQuery<T> {
    _m: PhantomData<T>,
}

pub trait QueryFragment<'a> {
    type Item;
    type It: Iterator<Item = Self::Item> + 'a;
    type ItemMut;
    type ItMut: Iterator<Item = Self::ItemMut> + 'a;

    fn iter(archetype: &'a ArchetypeStorage) -> Self::It;
    fn iter_mut(archetype: &'a ArchetypeStorage) -> Self::ItMut;
    fn fetch(archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::Item>;
    fn fetch_mut(archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::ItemMut>;
    fn types_mut(set: &mut HashSet<TypeId>);
    fn types_const(set: &mut HashSet<TypeId>);
    fn contains(archetype: &'a ArchetypeStorage) -> bool;
}

pub trait QueryPrimitive {
    type Item<'a>;
    type It<'a>: Iterator<Item = Self::Item<'a>>;
    type ItemMut<'a>;
    type ItMut<'a>: Iterator<Item = Self::ItemMut<'a>>;

    fn iter_prim(archetype: &ArchetypeStorage) -> Self::It<'_>;
    fn iter_prim_mut(archetype: &ArchetypeStorage) -> Self::ItMut<'_>;
    fn fetch_prim(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::Item<'_>>;
    fn fetch_prim_mut(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::ItemMut<'_>>;
    fn contains_prim(archetype: &ArchetypeStorage) -> bool;
    fn types_mut(set: &mut HashSet<TypeId>);
    fn types_const(set: &mut HashSet<TypeId>);
}

impl QueryPrimitive for ArchQuery<EntityId> {
    type Item<'a> = EntityId;
    type It<'a> = std::iter::Copied<std::slice::Iter<'a, EntityId>>;
    type ItemMut<'a> = EntityId;
    type ItMut<'a> = std::iter::Copied<std::slice::Iter<'a, EntityId>>;

    fn iter_prim<'a>(archetype: &'a ArchetypeStorage) -> Self::It<'a> {
        archetype.entities.iter().copied()
    }

    fn iter_prim_mut<'a>(archetype: &'a ArchetypeStorage) -> Self::ItMut<'a> {
        Self::iter_prim(archetype)
    }

    fn fetch_prim(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::Item<'_>> {
        archetype.entities.get(index as usize).copied()
    }

    fn fetch_prim_mut(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::ItemMut<'_>> {
        Self::fetch_prim(archetype, index)
    }

    fn types_mut(_set: &mut HashSet<TypeId>) {
        // noop
    }

    fn types_const(_set: &mut HashSet<TypeId>) {
        // noop
        // entity_id is not considered while scheduling
    }

    fn contains_prim(_archetype: &ArchetypeStorage) -> bool {
        true
    }
}

// Optional query fetch functions return Option<Option<T>> where the outer optional is always Some.
// This awkward interface is there because of combined queries
//
impl<'a, T: Component> QueryPrimitive for ArchQuery<Option<&'a T>> {
    type Item<'b> = Option<&'b T>;
    type It<'b> = Box<dyn Iterator<Item = Self::Item<'b>> + 'b>;
    type ItemMut<'b> = Option<&'b T>;
    type ItMut<'b> = Box<dyn Iterator<Item = Self::Item<'b>> + 'b>;

    fn iter_prim(archetype: &ArchetypeStorage) -> Self::It<'_> {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => Box::new(unsafe { (*columns.get()).as_inner::<T>().iter() }.map(Some)),
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }

    fn iter_prim_mut(archetype: &ArchetypeStorage) -> Self::ItMut<'_> {
        Self::iter_prim(archetype)
    }

    fn fetch_prim(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::Item<'_>> {
        Some(archetype.get_component::<T>(index))
    }

    fn fetch_prim_mut(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::ItemMut<'_>> {
        Self::fetch_prim(archetype, index)
    }

    fn types_mut(_set: &mut HashSet<TypeId>) {
        // noop
    }

    fn types_const(set: &mut HashSet<TypeId>) {
        set.insert(TypeId::of::<T>());
    }

    fn contains_prim(_archetype: &ArchetypeStorage) -> bool {
        true
    }
}

impl<'a, T: Component> QueryPrimitive for ArchQuery<Option<&'a mut T>> {
    type Item<'b> = Option<&'b T>;
    type It<'b> = Box<dyn Iterator<Item = Self::Item<'b>> + 'b>;
    type ItemMut<'b> = Option<&'b mut T>;
    type ItMut<'b> = Box<dyn Iterator<Item = Self::ItemMut<'b>> + 'b>;

    fn iter_prim(archetype: &ArchetypeStorage) -> Self::It<'_> {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => Box::new(unsafe { (*columns.get()).as_inner::<T>().iter() }.map(Some)),
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }

    fn iter_prim_mut(archetype: &ArchetypeStorage) -> Self::ItMut<'_> {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => {
                Box::new(unsafe { (*columns.get()).as_inner_mut::<T>().iter_mut() }.map(Some))
            }
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }

    fn fetch_prim(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::Item<'_>> {
        Some(archetype.get_component::<T>(index))
    }

    fn fetch_prim_mut(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::ItemMut<'_>> {
        Some(archetype.get_component_mut::<T>(index))
    }

    fn types_mut(set: &mut HashSet<TypeId>) {
        set.insert(TypeId::of::<T>());
    }

    fn types_const(_set: &mut HashSet<TypeId>) {
        // noop
    }

    fn contains_prim(_archetype: &ArchetypeStorage) -> bool {
        true
    }
}

impl<'a, T: Component> QueryPrimitive for ArchQuery<&'a T> {
    type Item<'b> = &'b T;
    type It<'b> = std::iter::Flatten<std::option::IntoIter<std::slice::Iter<'b, T>>>;
    type ItemMut<'b> = &'b T;
    type ItMut<'b> = std::iter::Flatten<std::option::IntoIter<std::slice::Iter<'b, T>>>;

    fn fetch_prim(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::Item<'_>> {
        archetype.get_component::<T>(index)
    }

    fn fetch_prim_mut(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::ItemMut<'_>> {
        Self::fetch_prim(archetype, index)
    }

    fn contains_prim(archetype: &ArchetypeStorage) -> bool {
        archetype.contains_column::<T>()
    }

    fn types_mut(_set: &mut HashSet<TypeId>) {
        // noop
    }

    fn types_const(set: &mut HashSet<TypeId>) {
        set.insert(TypeId::of::<T>());
    }

    fn iter_prim(archetype: &ArchetypeStorage) -> Self::It<'_> {
        archetype
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { (*columns.get()).as_inner::<T>().iter() })
            .into_iter()
            .flatten()
    }

    fn iter_prim_mut(archetype: &ArchetypeStorage) -> Self::ItMut<'_> {
        Self::iter_prim(archetype)
    }
}

impl<'a, T: Component> QueryPrimitive for ArchQuery<&'a mut T> {
    type Item<'b> = &'b T;
    type It<'b> = std::iter::Flatten<std::option::IntoIter<std::slice::Iter<'b, T>>>;
    type ItemMut<'b> = &'b mut T;
    type ItMut<'b> = std::iter::Flatten<std::option::IntoIter<std::slice::IterMut<'b, T>>>;

    fn iter_prim(archetype: &ArchetypeStorage) -> Self::It<'_> {
        archetype
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { (*columns.get()).as_inner::<T>().iter() })
            .into_iter()
            .flatten()
    }

    fn iter_prim_mut(archetype: &ArchetypeStorage) -> Self::ItMut<'_> {
        archetype
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { (*columns.get()).as_inner_mut::<T>().iter_mut() })
            .into_iter()
            .flatten()
    }

    fn fetch_prim(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::Item<'_>> {
        archetype.get_component::<T>(index)
    }

    fn fetch_prim_mut(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::ItemMut<'_>> {
        archetype.get_component_mut::<T>(index)
    }

    fn contains_prim(archetype: &ArchetypeStorage) -> bool {
        archetype.contains_column::<T>()
    }

    fn types_mut(set: &mut HashSet<TypeId>) {
        let ty = TypeId::of::<T>();
        debug_assert!(!set.contains(&ty), "A query may only borrow a type once");
        set.insert(ty);
    }

    fn types_const(_set: &mut HashSet<TypeId>) {
        // noop
    }
}

impl<'a, T> QueryFragment<'a> for ArchQuery<T>
where
    ArchQuery<T>: QueryPrimitive,
    T: 'a,
{
    type Item = <Self as QueryPrimitive>::Item<'a>;
    type It = <Self as QueryPrimitive>::It<'a>;
    type ItemMut = <Self as QueryPrimitive>::ItemMut<'a>;
    type ItMut = <Self as QueryPrimitive>::ItMut<'a>;

    fn iter(archetype: &'a ArchetypeStorage) -> Self::It {
        Self::iter_prim(archetype)
    }

    fn iter_mut(archetype: &'a ArchetypeStorage) -> Self::ItMut {
        Self::iter_prim_mut(archetype)
    }

    fn fetch(archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::Item> {
        Self::fetch_prim(archetype, index)
    }

    fn fetch_mut(archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::ItemMut> {
        Self::fetch_prim_mut(archetype, index)
    }

    fn contains(archetype: &'a ArchetypeStorage) -> bool {
        Self::contains_prim(archetype)
    }

    fn types_mut(set: &mut HashSet<TypeId>) {
        <Self as QueryPrimitive>::types_mut(set);
    }

    fn types_const(set: &mut HashSet<TypeId>) {
        <Self as QueryPrimitive>::types_const(set);
    }
}

// macro implementing more combinations
//

pub struct TupleIterator<'a, Inner, Constraint>(Inner, PhantomData<&'a Constraint>);
pub struct TupleIteratorMut<'a, Inner, Constraint>(Inner, PhantomData<&'a Constraint>);

unsafe impl<'a, Inner, Constraint> Send for TupleIterator<'a, Inner, Constraint> {}
unsafe impl<'a, Inner, Constraint> Sync for TupleIterator<'a, Inner, Constraint> {}
unsafe impl<'a, Inner, Constraint> Send for TupleIteratorMut<'a, Inner, Constraint> {}
unsafe impl<'a, Inner, Constraint> Sync for TupleIteratorMut<'a, Inner, Constraint> {}

macro_rules! impl_tuple {
    ($($idx: tt : $t: ident),+ $(,)?) => {
        impl<'a, $($t,)+> Iterator for TupleIterator<
            'a
            , ($( <ArchQuery<$t> as QueryFragment<'a>>::It,)*)
            , ($($t),+)
        >
        where
            $(
                $t: 'a,
                ArchQuery<$t>: QueryFragment<'a>,
            )+
        {
            type Item = (
                $(
                <ArchQuery<$t> as QueryFragment<'a>>::Item,
                )*
            );

            fn next(&mut self) -> Option<Self::Item> {
                Some((
                    $(
                        // TODO: optimization opportunity: only call next() on the first iterator
                        // and call next_unchecked() on the rest
                        self.0.$idx.next()?
                    ),+
                ))
            }
        }

        impl<'a, $($t,)+> Iterator for TupleIteratorMut<
            'a
            , ($( <ArchQuery<$t> as QueryFragment<'a>>::ItMut,)*)
            , ($($t),+)
        >
        where
            $(
                $t: 'a,
                ArchQuery<$t>: QueryFragment<'a>,
            )+
        {
            type Item = (
                $(
                <ArchQuery<$t> as QueryFragment<'a>>::ItemMut,
                )*
            );

            fn next(&mut self) -> Option<Self::Item> {
                Some((
                    $(
                        // TODO: optimization opportunity: only call next() on the first iterator
                        // and call next_unchecked() on the rest
                        self.0.$idx.next()?
                    ),+
                ))
            }
        }

        impl<'a, $($t,)+> QueryFragment<'a> for ArchQuery<($($t,)+)>
        where
        $(
            $t: 'a,
            ArchQuery<$t>: QueryPrimitive,
        )+
        {
            type Item=($(<ArchQuery<$t> as QueryPrimitive>::Item<'a>),+);
            type It=TupleIterator<'a, ($(<ArchQuery<$t> as QueryPrimitive>::It<'a>,)+),($($t,)+)>;
            type ItemMut=($(<ArchQuery<$t> as QueryPrimitive>::ItemMut<'a>),+);
            type ItMut=TupleIteratorMut<'a, ($(<ArchQuery<$t> as QueryPrimitive>::ItMut<'a>,)+),($($t,)+)>;

            fn iter(archetype: &'a ArchetypeStorage) -> Self::It
            {
                TupleIterator(($( ArchQuery::<$t>::iter(archetype) ),+), PhantomData)
            }

            fn iter_mut(archetype: &'a ArchetypeStorage) -> Self::ItMut
            {
                TupleIteratorMut(($( ArchQuery::<$t>::iter_mut(archetype) ),+), PhantomData)
            }

            fn fetch(archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::Item> {
                Some((
                    $(
                        ArchQuery::<$t>::fetch(archetype, index)?,
                    )*
                ))
            }

            fn fetch_mut(archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::ItemMut> {
                Some((
                    $(
                        ArchQuery::<$t>::fetch_mut(archetype, index)?,
                    )*
                ))
            }

            fn contains(archetype: &'a ArchetypeStorage) -> bool {
                    $(
                        ArchQuery::<$t>::contains(archetype)
                    )&&*
            }

            fn types_mut(set: &mut HashSet<TypeId>) {
                $(<ArchQuery<$t> as QueryPrimitive>::types_mut(set));+
            }

            fn types_const(set: &mut HashSet<TypeId>) {
                $(<ArchQuery<$t> as QueryPrimitive>::types_const(set));+
            }
        }
    };
}

impl_tuple!(0: T0, 1: T1);
impl_tuple!(0: T0, 1: T1, 2: T2);
impl_tuple!(0: T0, 1: T1, 2: T2, 3: T3);
impl_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4);
impl_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5);
impl_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6);
impl_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12,
    13: T13
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12,
    13: T13,
    14: T14
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12,
    13: T13,
    14: T14,
    15: T15
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12,
    13: T13,
    14: T14,
    15: T15,
    16: T16
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12,
    13: T13,
    14: T14,
    15: T15,
    16: T16,
    17: T17
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12,
    13: T13,
    14: T14,
    15: T15,
    16: T16,
    17: T17,
    18: T18
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12,
    13: T13,
    14: T14,
    15: T15,
    16: T16,
    17: T17,
    18: T18,
    19: T19
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12,
    13: T13,
    14: T14,
    15: T15,
    16: T16,
    17: T17,
    18: T18,
    19: T19,
    20: T20
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12,
    13: T13,
    14: T14,
    15: T15,
    16: T16,
    17: T17,
    18: T18,
    19: T19,
    20: T20,
    21: T21
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12,
    13: T13,
    14: T14,
    15: T15,
    16: T16,
    17: T17,
    18: T18,
    19: T19,
    20: T20,
    21: T21,
    22: T22
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12,
    13: T13,
    14: T14,
    15: T15,
    16: T16,
    17: T17,
    18: T18,
    19: T19,
    20: T20,
    21: T21,
    22: T22,
    23: T23
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12,
    13: T13,
    14: T14,
    15: T15,
    16: T16,
    17: T17,
    18: T18,
    19: T19,
    20: T20,
    21: T21,
    22: T22,
    23: T23,
    24: T24
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12,
    13: T13,
    14: T14,
    15: T15,
    16: T16,
    17: T17,
    18: T18,
    19: T19,
    20: T20,
    21: T21,
    22: T22,
    23: T23,
    24: T24,
    25: T25
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12,
    13: T13,
    14: T14,
    15: T15,
    16: T16,
    17: T17,
    18: T18,
    19: T19,
    20: T20,
    21: T21,
    22: T22,
    23: T23,
    24: T24,
    25: T25,
    26: T26
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12,
    13: T13,
    14: T14,
    15: T15,
    16: T16,
    17: T17,
    18: T18,
    19: T19,
    20: T20,
    21: T21,
    22: T22,
    23: T23,
    24: T24,
    25: T25,
    26: T26,
    27: T27
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12,
    13: T13,
    14: T14,
    15: T15,
    16: T16,
    17: T17,
    18: T18,
    19: T19,
    20: T20,
    21: T21,
    22: T22,
    23: T23,
    24: T24,
    25: T25,
    26: T26,
    27: T27,
    28: T28
);
impl_tuple!(
    0: T0,
    1: T1,
    2: T2,
    3: T3,
    4: T4,
    5: T5,
    6: T6,
    7: T7,
    8: T8,
    9: T9,
    10: T10,
    11: T11,
    12: T12,
    13: T13,
    14: T14,
    15: T15,
    16: T16,
    17: T17,
    18: T18,
    19: T19,
    20: T20,
    21: T21,
    22: T22,
    23: T23,
    24: T24,
    25: T25,
    26: T26,
    27: T27,
    28: T28,
    29: T29
);
