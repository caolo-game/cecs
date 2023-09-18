#[cfg(test)]
mod test;

use std::{any::TypeId, collections::HashSet, marker::PhantomData};

use crate::{
    prelude::{Filter, Query},
    query::{ArchQuery, QueryFragment, WorldQuery},
};

/// A QuerySet can be used when a system needs multiple, coupled queries.
///
/// ## Limitations
///
/// Currently if a sub-query needs mutable access to a component, then all sub-queries to the same
/// component must also be mutable.
///
/// ```
/// use cecs::prelude::*;
/// #[derive(Default, Clone)]
/// struct Foo {
///     value: i32,
/// }
///
/// #[derive(Default, Clone)]
/// struct Bar;
///
/// fn sys(
///     mut q: QuerySet<(
///         Query<(&mut Foo, &Bar)>,
///         Query<&mut Foo>,
///     )>,
/// ) {
///     for foo in q.q1_mut().iter_mut() {
///         assert_eq!(foo.value, 0);
///         foo.value = 42;
///     }
///
///     // notice how foo is not changed in this loop, but q0 still has to request mutable access
///     for (foo, _bar) in q.q0().iter() {
///         assert_eq!(foo.value, 42);
///     }
/// }
/// let mut world = World::new(4);
/// world.run_system(sys).unwrap();
/// ```
pub struct QuerySet<Inner> {
    inner: Inner,
    _m: PhantomData<Inner>,
}

macro_rules! impl_tuple {
    ($($idx: tt , $t: ident , $f: ident , $q: ident , $q_mut: ident , $set: ident);+ $(;)?) => {
        impl<'a, $($t, $f),*> QuerySet<($(Query<$t, $f>),*)>
        where
            $(
            ArchQuery<$t>: QueryFragment,
            $f: Filter,
            )*
        {
            $(
            pub fn $q(&self) -> &Query<$t, $f> {
                &self.inner.$idx
            }

            pub fn $q_mut(&mut self) -> &mut Query<$t, $f> {
                &mut self.inner.$idx
            }

            )*
        }

        impl<'a, $($t, $f),*> WorldQuery<'a> for QuerySet<($(Query<$t, $f>),*)>
        where
            $(
            ArchQuery<$t>: QueryFragment,
            $f: Filter,
            )*
        {
            fn new(db: &'a crate::World, _commands_index: usize) -> Self {
                Self {
                    inner: ($(Query::<$t, $f>::new(db)),*),
                    _m: PhantomData,
                }
            }

            fn exclusive() -> bool {false}

            fn read_only() -> bool {
                true
                $(
                    &&
                    <ArchQuery<$t> as QueryFragment>::read_only()
                )*
            }

            fn components_mut(set: &mut HashSet<TypeId>) {
                // sub queries may have overlapping type (that's the point of the QuerySet)
                // types_mut will panic in this case, so we'll try all in isolation, then
                // add the types to the output
                $(
                    let mut $set = set.clone();
                    <ArchQuery<$t> as QueryFragment>::types_mut(&mut $set);
                )*

                $(
                    set.extend($set.into_iter());
                )*
            }

            fn components_const(set: &mut HashSet<TypeId>) {
                $(
                    <ArchQuery<$t> as QueryFragment>::types_const(set);
                )*
            }
        }
    };
}

impl_tuple!(0 , T0 , F0 , q0 , q0_mut , set0; 1 , T1 , F1 , q1 , q1_mut , set1;);
impl_tuple!(0 , T0 , F0 , q0 , q0_mut , set0; 1 , T1 , F1 , q1 , q1_mut , set1; 2 , T2 , F2 , q2 , q2_mut , set2;);
impl_tuple!(0 , T0 , F0 , q0 , q0_mut , set0; 1 , T1 , F1 , q1 , q1_mut , set1; 2 , T2 , F2 , q2 , q2_mut , set2; 3 , T3 , F3 , q3 , q3_mut , set3;);
impl_tuple!(0 , T0 , F0 , q0 , q0_mut , set0; 1 , T1 , F1 , q1 , q1_mut , set1; 2 , T2 , F2 , q2 , q2_mut , set2; 3 , T3 , F3 , q3 , q3_mut , set3; 4 , T4 , F4 , q4 , q4_mut , set4;);
impl_tuple!(0 , T0 , F0 , q0 , q0_mut , set0; 1 , T1 , F1 , q1 , q1_mut , set1; 2 , T2 , F2 , q2 , q2_mut , set2; 3 , T3 , F3 , q3 , q3_mut , set3; 4 , T4 , F4 , q4 , q4_mut , set4; 5 , T5 , F5 , q5 , q5_mut , set5;);
impl_tuple!(0 , T0 , F0 , q0 , q0_mut , set0; 1 , T1 , F1 , q1 , q1_mut , set1; 2 , T2 , F2 , q2 , q2_mut , set2; 3 , T3 , F3 , q3 , q3_mut , set3; 4 , T4 , F4 , q4 , q4_mut , set4; 5 , T5 , F5 , q5 , q5_mut , set5; 6 , T6 , F6 , q6 , q6_mut , set6;);
impl_tuple!(0 , T0 , F0 , q0 , q0_mut , set0; 1 , T1 , F1 , q1 , q1_mut , set1; 2 , T2 , F2 , q2 , q2_mut , set2; 3 , T3 , F3 , q3 , q3_mut , set3; 4 , T4 , F4 , q4 , q4_mut , set4; 5 , T5 , F5 , q5 , q5_mut , set5; 6 , T6 , F6 , q6 , q6_mut , set6; 7 , T7 , F7 , q7 , q7_mut , set7;);
impl_tuple!(0 , T0 , F0 , q0 , q0_mut , set0; 1 , T1 , F1 , q1 , q1_mut , set1; 2 , T2 , F2 , q2 , q2_mut , set2; 3 , T3 , F3 , q3 , q3_mut , set3; 4 , T4 , F4 , q4 , q4_mut , set4; 5 , T5 , F5 , q5 , q5_mut , set5; 6 , T6 , F6 , q6 , q6_mut , set6; 7 , T7 , F7 , q7 , q7_mut , set7; 8 , T8 , F8 , q8 , q8_mut , set8;);
impl_tuple!(0 , T0 , F0 , q0 , q0_mut , set0; 1 , T1 , F1 , q1 , q1_mut , set1; 2 , T2 , F2 , q2 , q2_mut , set2; 3 , T3 , F3 , q3 , q3_mut , set3; 4 , T4 , F4 , q4 , q4_mut , set4; 5 , T5 , F5 , q5 , q5_mut , set5; 6 , T6 , F6 , q6 , q6_mut , set6; 7 , T7 , F7 , q7 , q7_mut , set7; 8 , T8 , F8 , q8 , q8_mut , set8; 9 , T9 , F9 , q9 , q9_mut , set9;);
