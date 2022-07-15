pub mod filters;
pub mod resource_query;

#[cfg(test)]
mod query_tests;

use crate::{archetype::ArchetypeStorage, entity_id::EntityId, Component, Index, RowIndex, World};
use filters::Filter;
use std::{any::TypeId, collections::HashSet, marker::PhantomData};

pub(crate) trait WorldQuery<'a> {
    fn new(db: &'a World) -> Self;

    /// List of component types this query needs exclusive access to
    fn components_mut(set: &mut HashSet<TypeId>);
    /// List of component types this query needs
    fn components_const(set: &mut HashSet<TypeId>);
    /// List of resource types this query needs exclusive access to
    fn resources_mut(set: &mut HashSet<TypeId>);
    /// List of resource types this query needs
    fn resources_const(set: &mut HashSet<TypeId>);
}

pub struct Query<'a, T, F = ()> {
    world: &'a crate::World,
    _m: PhantomData<(T, F)>,
}

impl<'a, T, F> WorldQuery<'a> for Query<'a, T, F>
where
    ArchQuery<T>: QueryFragment<'a>,
    F: Filter,
{
    fn new(db: &'a World) -> Self {
        Self::new(db)
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

impl<'a, T, F> Query<'a, T, F>
where
    ArchQuery<T>: QueryFragment<'a>,
    F: Filter,
{
    pub fn new(world: &'a crate::World) -> Self {
        Query {
            world,
            _m: PhantomData,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = <ArchQuery<T> as QueryFragment<'a>>::Item> {
        self.world
            .archetypes
            .iter()
            .filter(|(_, arch)| F::filter(arch))
            .flat_map(|(_, arch)| ArchQuery::<T>::iter(arch))
    }

    pub fn fetch(&self, id: EntityId) -> Option<<ArchQuery<T> as QueryFragment<'a>>::Item> {
        let (arch, index) = self.world.entity_ids.read(id).ok()?;
        unsafe {
            if !F::filter(arch.as_ref()) {
                return None;
            }

            ArchQuery::<T>::fetch(arch.as_ref(), index)
        }
    }

    pub fn contains(&self, id: EntityId) -> bool {
        let (arch, _index) = match self.world.entity_ids.read(id).ok() {
            None => return false,
            Some(x) => x,
        };
        unsafe {
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

    fn iter(archetype: &'a ArchetypeStorage) -> Self::It;
    fn fetch(archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::Item>;
    fn types_mut(set: &mut HashSet<TypeId>);
    fn types_const(set: &mut HashSet<TypeId>);
    fn contains(archetype: &'a ArchetypeStorage) -> bool;
}

pub trait QueryPrimitive<'a> {
    type Item;
    type It: Iterator<Item = Self::Item> + 'a;

    fn iter_prim(archetype: &'a ArchetypeStorage) -> Self::It;
    fn fetch_prim(archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::Item>;
    fn contains_prim(archetype: &'a ArchetypeStorage) -> bool;
    fn types_mut(set: &mut HashSet<TypeId>);
    fn types_const(set: &mut HashSet<TypeId>);
}

impl<'a> QueryPrimitive<'a> for ArchQuery<EntityId> {
    type Item = EntityId;
    type It = std::iter::Copied<std::slice::Iter<'a, EntityId>>;

    fn iter_prim(archetype: &'a ArchetypeStorage) -> Self::It {
        archetype.entities.iter().copied()
    }

    fn fetch_prim(archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::Item> {
        archetype.entities.get(index as usize).copied()
    }

    fn types_mut(_set: &mut HashSet<TypeId>) {
        // noop
    }

    fn types_const(_set: &mut HashSet<TypeId>) {
        // noop
        // entity_id is not considered while scheduling
    }

    fn contains_prim(_archetype: &'a ArchetypeStorage) -> bool {
        true
    }
}

// Optional query fetch functions return Option<Option<T>> where the outer optional is always Some.
// This awkward interface is there because of combined queries
//
impl<'a, T: Component> QueryPrimitive<'a> for ArchQuery<Option<&'a T>> {
    type Item = Option<&'a T>;
    type It = Box<dyn Iterator<Item = Self::Item> + 'a>;

    fn iter_prim(archetype: &'a ArchetypeStorage) -> Self::It {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => Box::new(unsafe { (*columns.get()).as_inner::<T>().iter() }.map(Some)),
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }

    fn fetch_prim(archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::Item> {
        Some(archetype.get_component::<T>(index))
    }

    fn types_mut(_set: &mut HashSet<TypeId>) {
        // noop
    }

    fn types_const(set: &mut HashSet<TypeId>) {
        set.insert(TypeId::of::<T>());
    }

    fn contains_prim(_archetype: &'a ArchetypeStorage) -> bool {
        true
    }
}

impl<'a, T: Component> QueryPrimitive<'a> for ArchQuery<Option<&'a mut T>> {
    type Item = Option<&'a mut T>;
    type It = Box<dyn Iterator<Item = Self::Item> + 'a>;

    fn iter_prim(archetype: &'a ArchetypeStorage) -> Self::It {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => {
                Box::new(unsafe { (*columns.get()).as_inner_mut::<T>().iter_mut() }.map(Some))
            }
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }

    fn fetch_prim(archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::Item> {
        Some(archetype.get_component_mut::<T>(index))
    }

    fn types_mut(set: &mut HashSet<TypeId>) {
        set.insert(TypeId::of::<T>());
    }

    fn types_const(set: &mut HashSet<TypeId>) {
        set.insert(TypeId::of::<T>());
    }

    fn contains_prim(_archetype: &'a ArchetypeStorage) -> bool {
        true
    }
}

impl<'a, T: Component> QueryPrimitive<'a> for ArchQuery<&'a T> {
    type Item = &'a T;
    type It = std::iter::Flatten<std::option::IntoIter<std::slice::Iter<'a, T>>>;

    fn iter_prim(archetype: &'a ArchetypeStorage) -> Self::It {
        archetype
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { (*columns.get()).as_inner::<T>().iter() })
            .into_iter()
            .flatten()
    }

    fn fetch_prim(archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::Item> {
        archetype.get_component::<T>(index)
    }

    fn contains_prim(archetype: &'a ArchetypeStorage) -> bool {
        archetype.contains_column::<T>()
    }

    fn types_mut(_set: &mut HashSet<TypeId>) {
        // noop
    }

    fn types_const(set: &mut HashSet<TypeId>) {
        set.insert(TypeId::of::<T>());
    }
}

impl<'a, T: Component> QueryPrimitive<'a> for ArchQuery<&'a mut T> {
    type Item = &'a mut T;
    type It = std::iter::Flatten<std::option::IntoIter<std::slice::IterMut<'a, T>>>;

    fn iter_prim(archetype: &'a ArchetypeStorage) -> Self::It {
        archetype
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { (*columns.get()).as_inner_mut::<T>().iter_mut() })
            .into_iter()
            .flatten()
    }

    fn fetch_prim(archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::Item> {
        archetype.get_component_mut::<T>(index)
    }

    fn contains_prim(archetype: &'a ArchetypeStorage) -> bool {
        archetype.contains_column::<T>()
    }

    fn types_mut(set: &mut HashSet<TypeId>) {
        set.insert(TypeId::of::<T>());
    }

    fn types_const(set: &mut HashSet<TypeId>) {
        set.insert(TypeId::of::<T>());
    }
}

impl<'a, T> QueryFragment<'a> for ArchQuery<T>
where
    ArchQuery<T>: QueryPrimitive<'a>,
{
    type Item = <Self as QueryPrimitive<'a>>::Item;
    type It = <Self as QueryPrimitive<'a>>::It;

    fn iter(archetype: &'a ArchetypeStorage) -> Self::It {
        Self::iter_prim(archetype)
    }

    fn fetch(archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::Item> {
        Self::fetch_prim(archetype, index)
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

        impl<'a, $($t,)+> QueryFragment<'a> for ArchQuery<($($t,)+)>
        where
        $(
            $t: 'a,
            ArchQuery<$t>: QueryPrimitive<'a>,
        )+
        {
            type Item=($(<ArchQuery<$t> as QueryPrimitive<'a>>::Item),+);
            type It=TupleIterator<'a, ($(<ArchQuery<$t> as QueryPrimitive<'a>>::It,)+),($($t,)+)>;

            fn iter(archetype: &'a ArchetypeStorage) -> Self::It
            {
                TupleIterator(($( ArchQuery::<$t>::iter(archetype) ),+), PhantomData)
            }

            fn fetch(archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::Item> {
                Some((
                    $(
                        ArchQuery::<$t>::fetch(archetype, index)?,
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
