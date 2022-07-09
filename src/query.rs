#[cfg(test)]
mod query_tests;

use crate::{db::ArchetypeStorage, entity_id::EntityId, Component, Index, RowIndex};
use std::{any::TypeId, marker::PhantomData};

pub struct Query<'a, T> {
    world: &'a crate::World,
    _m: PhantomData<T>,
}

impl<'a, T> Query<'a, T>
where
    ArchQuery<T>: QueryFragment<'a>,
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
            .flat_map(|(_, arch)| ArchQuery::<T>::default().iter(arch))
    }

    pub fn fetch(&self, id: EntityId) -> Option<<ArchQuery<T> as QueryFragment<'a>>::FetchItem> {
        let (arch, index) = self.world.entity_ids.read(id).ok()?;
        unsafe { ArchQuery::<T>::default().fetch(arch.as_ref(), index) }
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
    type FetchItem;

    fn iter(&self, archetype: &'a ArchetypeStorage) -> Self::It;
    fn fetch(&self, archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::FetchItem>;
}

pub trait QueryPrimitive<'a> {
    type Item;
    type It: Iterator<Item = Self::Item> + 'a;
    type FetchItem;

    fn iter_prim(&self, archetype: &'a ArchetypeStorage) -> Self::It;
    fn fetch_prim(&self, archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::FetchItem>;
}

impl<T> Default for ArchQuery<T> {
    fn default() -> Self {
        Self { _m: PhantomData }
    }
}

impl<'a> QueryPrimitive<'a> for ArchQuery<EntityId> {
    type Item = EntityId;
    type It = std::iter::Copied<std::slice::Iter<'a, EntityId>>;
    type FetchItem = Self::Item;

    fn iter_prim(&self, archetype: &'a ArchetypeStorage) -> Self::It {
        archetype.entities.iter().copied()
    }

    fn fetch_prim(&self, archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::Item> {
        archetype.entities.get(index as usize).copied()
    }
}

impl<'a, T: Component> QueryPrimitive<'a> for ArchQuery<&'a T> {
    type Item = &'a T;
    type It = std::iter::Flatten<std::option::IntoIter<std::slice::Iter<'a, T>>>;
    type FetchItem = Self::Item;

    fn iter_prim(&self, archetype: &'a ArchetypeStorage) -> Self::It {
        archetype
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { (&mut *columns.get()).as_inner::<T>().iter() })
            .into_iter()
            .flatten()
    }

    fn fetch_prim(&self, archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::Item> {
        archetype.get_component::<T>(index)
    }
}

impl<'a, T: Component> QueryPrimitive<'a> for ArchQuery<&'a mut T> {
    type Item = &'a mut T;
    type It = std::iter::Flatten<std::option::IntoIter<std::slice::IterMut<'a, T>>>;
    type FetchItem = Self::Item;

    fn iter_prim(&self, archetype: &'a ArchetypeStorage) -> Self::It {
        archetype
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { (&mut *columns.get()).as_inner_mut::<T>().iter_mut() })
            .into_iter()
            .flatten()
    }

    fn fetch_prim(&self, archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::Item> {
        archetype.get_component_mut::<T>(index)
    }
}

impl<'a, T> QueryFragment<'a> for ArchQuery<T>
where
    ArchQuery<T>: QueryPrimitive<'a>,
{
    type Item = <Self as QueryPrimitive<'a>>::Item;
    type It = <Self as QueryPrimitive<'a>>::It;
    type FetchItem = <Self as QueryPrimitive<'a>>::FetchItem;

    fn iter(&self, archetype: &'a ArchetypeStorage) -> Self::It {
        self.iter_prim(archetype)
    }

    fn fetch(&self, archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::FetchItem> {
        self.fetch_prim(archetype, index)
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
            type FetchItem = ($(<ArchQuery<$t> as QueryPrimitive<'a>>::FetchItem),+);

            fn iter(&self, archetype: &'a ArchetypeStorage) -> Self::It
            {
                TupleIterator(($( ArchQuery::<$t>::default().iter(archetype) ),+), PhantomData)
            }

            fn fetch(&self, archetype: &'a ArchetypeStorage, index: RowIndex) -> Option<Self::FetchItem> {
                Some((
                    $(
                        ArchQuery::<$t>::default().fetch(archetype, index)?,
                    )*
                ))
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
