#[cfg(test)]
mod query_tests;

use crate::{db::ArchetypeStorage, Component};
use std::{any::TypeId, marker::PhantomData};

pub trait Queryable<'a, T> {
    type Item;
    type It: Iterator<Item = Self::Item>;

    fn iter(&'a self) -> Self::It;
}

impl<'a, T: Component> Queryable<'a, &'a T> for ArchetypeStorage {
    type Item = &'a T;
    type It = std::iter::Flatten<std::option::IntoIter<std::slice::Iter<'a, T>>>;

    fn iter(&'a self) -> Self::It {
        self.components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { (&mut *columns.get()).as_inner::<T>().iter() })
            .into_iter()
            .flatten()
    }
}

impl<'a, T: Component> Queryable<'a, &'a mut T> for ArchetypeStorage {
    type Item = &'a mut T;
    type It = std::iter::Flatten<std::option::IntoIter<std::slice::IterMut<'a, T>>>;

    fn iter(&'a self) -> Self::It {
        self.components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { (&mut *columns.get()).as_inner_mut::<T>().iter_mut() })
            .into_iter()
            .flatten()
    }
}

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
}

pub struct ArchQuery<T> {
    _m: PhantomData<T>,
}

pub trait QueryFragment<'a> {
    type Item;
    type It: Iterator<Item = Self::Item> + 'a;

    fn iter(&self, archetype: &'a ArchetypeStorage) -> Self::It;
}

pub trait QueryPrimitive<'a> {
    type Item;
    type It: Iterator<Item = Self::Item> + 'a;

    fn iter_prim(&self, archetype: &'a ArchetypeStorage) -> Self::It;
}

impl<T> Default for ArchQuery<T> {
    fn default() -> Self {
        Self { _m: PhantomData }
    }
}

impl<'a> QueryPrimitive<'a> for ArchQuery<crate::entity_id::EntityId> {
    type Item = crate::entity_id::EntityId;
    type It = std::iter::Copied<std::slice::Iter<'a, crate::entity_id::EntityId>>;

    fn iter_prim(&self, archetype: &'a ArchetypeStorage) -> Self::It {
        archetype.entities.iter().copied()
    }
}

impl<'a, T: Component> QueryPrimitive<'a> for ArchQuery<&'a T> {
    type Item = &'a T;
    type It = <ArchetypeStorage as Queryable<'a, &'a T>>::It;

    fn iter_prim(&self, archetype: &'a ArchetypeStorage) -> Self::It {
        <ArchetypeStorage as Queryable<'_, &'a T>>::iter(archetype)
    }
}

impl<'a, T: Component> QueryPrimitive<'a> for ArchQuery<&'a mut T> {
    type Item = &'a mut T;
    type It = <ArchetypeStorage as Queryable<'a, &'a mut T>>::It;

    fn iter_prim(&self, archetype: &'a ArchetypeStorage) -> Self::It {
        <ArchetypeStorage as Queryable<'_, &'a mut T>>::iter(archetype)
    }
}

impl<'a, T> QueryFragment<'a> for ArchQuery<T>
where
    ArchQuery<T>: QueryPrimitive<'a>,
{
    type Item = <Self as QueryPrimitive<'a>>::Item;
    type It = <Self as QueryPrimitive<'a>>::It;

    fn iter(&self, archetype: &'a ArchetypeStorage) -> Self::It {
        self.iter_prim(archetype)
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
            ArchetypeStorage: Queryable<'a, $t>,
        )+
        {
            type Item=($(<ArchQuery<$t> as QueryPrimitive<'a>>::Item),+);
            type It=TupleIterator<'a, ($(<ArchQuery<$t> as QueryPrimitive<'a>>::It,)+),($($t,)+)>;

            fn iter(&self, archetype: &'a ArchetypeStorage) -> Self::It
            {
                TupleIterator(($( ArchQuery::<$t>::default().iter(archetype) ),+), PhantomData)
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
