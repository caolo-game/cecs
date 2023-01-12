// FIXME: yeet the bloody Box<dyn Iterator> iterators please
//
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
    ArchQuery<T>: QueryFragment,
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

impl<T, F> Query<T, F>
where
    ArchQuery<T>: QueryFragment,
    F: Filter,
{
    pub fn new(world: &crate::World) -> Self {
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

    pub fn iter<'a>(&self) -> impl Iterator<Item = <ArchQuery<T> as QueryFragment>::Item<'a>>
    where
        Self: 'a,
    {
        unsafe {
            self.world
                .as_ref()
                .archetypes
                .iter()
                .filter(|(_, arch)| F::filter(arch))
                .flat_map(|(_, arch)| ArchQuery::<T>::iter(arch))
        }
    }

    pub fn iter_mut<'a>(
        &mut self,
    ) -> impl Iterator<Item = <ArchQuery<T> as QueryFragment>::ItemMut<'a>>
    where
        Self: 'a,
    {
        unsafe {
            self.world
                .as_ref()
                .archetypes
                .iter()
                .filter(|(_, arch)| F::filter(arch))
                .flat_map(|(_, arch)| ArchQuery::<T>::iter_mut(arch))
        }
    }

    /// Unsafe functions let you pass the same query to sub-systems recursively splitting workload
    /// on multiple threads.
    ///
    /// The top-level system still needs &mut access to the components.
    pub unsafe fn iter_unsafe<'a>(
        &self,
    ) -> impl Iterator<Item = <ArchQuery<T> as QueryFragment>::ItemUnsafe<'a>>
    where
        Self: 'a,
    {
        self.world
            .as_ref()
            .archetypes
            .iter()
            .filter(|(_, arch)| F::filter(arch))
            .flat_map(|(_, arch)| ArchQuery::<T>::iter_unsafe(arch))
    }

    pub fn fetch<'a>(&self, id: EntityId) -> Option<<ArchQuery<T> as QueryFragment>::Item<'a>>
    where
        Self: 'a,
    {
        unsafe {
            let (arch, index) = self.world.as_ref().entity_ids().read(id).ok()?;
            if !F::filter(arch.as_ref()) {
                return None;
            }

            ArchQuery::<T>::fetch(arch.as_ref(), index)
        }
    }

    pub fn fetch_mut<'a>(
        &mut self,
        id: EntityId,
    ) -> Option<<ArchQuery<T> as QueryFragment>::ItemMut<'a>>
    where
        Self: 'a,
    {
        unsafe {
            let (arch, index) = self.world.as_ref().entity_ids().read(id).ok()?;
            if !F::filter(arch.as_ref()) {
                return None;
            }

            ArchQuery::<T>::fetch_mut(arch.as_ref(), index)
        }
    }

    pub unsafe fn fetch_unsafe<'a>(
        &self,
        id: EntityId,
    ) -> Option<<ArchQuery<T> as QueryFragment>::ItemUnsafe<'a>>
    where
        Self: 'a,
    {
        unsafe {
            let (arch, index) = self.world.as_ref().entity_ids().read(id).ok()?;
            if !F::filter(arch.as_ref()) {
                return None;
            }

            ArchQuery::<T>::fetch_unsafe(arch.as_ref(), index)
        }
    }

    pub fn contains(&self, id: EntityId) -> bool {
        unsafe {
            let (arch, _index) = match self.world.as_ref().entity_ids().read(id).ok() {
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
    pub fn one<'a>(&self) -> <ArchQuery<T> as QueryFragment>::Item<'a>
    where
        Self: 'a,
    {
        self.iter().next().unwrap()
    }
}

pub struct ArchQuery<T> {
    _m: PhantomData<T>,
}

pub trait QueryFragment {
    type ItemUnsafe<'a>;
    type ItUnsafe<'a>: Iterator<Item = Self::ItemUnsafe<'a>>;
    type Item<'a>;
    type It<'a>: Iterator<Item = Self::Item<'a>>;
    type ItemMut<'a>;
    type ItMut<'a>: Iterator<Item = Self::ItemMut<'a>>;

    unsafe fn iter_unsafe(archetype: &ArchetypeStorage) -> Self::ItUnsafe<'_>;
    unsafe fn fetch_unsafe(
        archetype: &ArchetypeStorage,
        index: RowIndex,
    ) -> Option<Self::ItemUnsafe<'_>>;
    fn iter(archetype: &ArchetypeStorage) -> Self::It<'_>;
    fn iter_mut(archetype: &ArchetypeStorage) -> Self::ItMut<'_>;
    fn fetch(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::Item<'_>>;
    fn fetch_mut(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::ItemMut<'_>>;
    fn types_mut(set: &mut HashSet<TypeId>);
    fn types_const(set: &mut HashSet<TypeId>);
    fn contains(archetype: &ArchetypeStorage) -> bool;
}

pub trait QueryPrimitive {
    type ItemUnsafe<'a>;
    type ItUnsafe<'a>: Iterator<Item = Self::ItemUnsafe<'a>>;
    type Item<'a>;
    type It<'a>: Iterator<Item = Self::Item<'a>>;
    type ItemMut<'a>;
    type ItMut<'a>: Iterator<Item = Self::ItemMut<'a>>;

    unsafe fn iter_prim_unsafe(archetype: &ArchetypeStorage) -> Self::ItUnsafe<'_>;
    unsafe fn fetch_prim_unsafe(
        archetype: &ArchetypeStorage,
        index: RowIndex,
    ) -> Option<Self::ItemUnsafe<'_>>;
    fn iter_prim(archetype: &ArchetypeStorage) -> Self::It<'_>;
    fn iter_prim_mut(archetype: &ArchetypeStorage) -> Self::ItMut<'_>;
    fn fetch_prim(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::Item<'_>>;
    fn fetch_prim_mut(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::ItemMut<'_>>;
    fn contains_prim(archetype: &ArchetypeStorage) -> bool;
    fn types_mut(set: &mut HashSet<TypeId>);
    fn types_const(set: &mut HashSet<TypeId>);
}

impl QueryPrimitive for ArchQuery<EntityId> {
    type ItemUnsafe<'a> = EntityId;
    type ItUnsafe<'a> = std::iter::Copied<std::slice::Iter<'a, EntityId>>;
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

    unsafe fn iter_prim_unsafe<'a>(archetype: &'a ArchetypeStorage) -> Self::ItUnsafe<'a> {
        archetype.entities.iter().copied()
    }

    fn fetch_prim(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::Item<'_>> {
        archetype.entities.get(index as usize).copied()
    }

    fn fetch_prim_mut(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::ItemMut<'_>> {
        Self::fetch_prim(archetype, index)
    }

    unsafe fn fetch_prim_unsafe(
        archetype: &ArchetypeStorage,
        index: RowIndex,
    ) -> Option<Self::ItemUnsafe<'_>> {
        archetype.entities.get(index as usize).copied()
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
    type ItemUnsafe<'b> = Option<*mut T>;
    type ItUnsafe<'b> = Box<dyn Iterator<Item = Self::ItemUnsafe<'b>> + 'b>;
    type Item<'b> = Option<&'b T>;
    type It<'b> = Box<dyn Iterator<Item = Self::Item<'b>> + 'b>;
    type ItemMut<'b> = Option<&'b T>;
    type ItMut<'b> = Box<dyn Iterator<Item = Self::Item<'b>> + 'b>;

    fn iter_prim(archetype: &ArchetypeStorage) -> Self::It<'_> {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => Box::new(unsafe { (*columns.get()).as_slice::<T>().iter() }.map(Some)),
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }

    fn iter_prim_mut(archetype: &ArchetypeStorage) -> Self::ItMut<'_> {
        Self::iter_prim(archetype)
    }

    unsafe fn iter_prim_unsafe(archetype: &ArchetypeStorage) -> Self::ItUnsafe<'_> {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => Box::new(
                unsafe { (*columns.get()).as_slice_mut::<T>().iter_mut() }
                    .map(|x| x as *mut _)
                    .map(Some),
            ),
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }

    fn fetch_prim(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::Item<'_>> {
        Some(archetype.get_component::<T>(index))
    }

    fn fetch_prim_mut(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::ItemMut<'_>> {
        Self::fetch_prim(archetype, index)
    }

    unsafe fn fetch_prim_unsafe(
        archetype: &ArchetypeStorage,
        index: RowIndex,
    ) -> Option<Self::ItemUnsafe<'_>> {
        Some(archetype.get_component_mut::<T>(index).map(|x| x as *mut _))
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
    type ItemUnsafe<'b> = Option<*mut T>;
    type ItUnsafe<'b> = Box<dyn Iterator<Item = Self::ItemUnsafe<'b>> + 'b>;
    type Item<'b> = Option<&'b T>;
    type It<'b> = Box<dyn Iterator<Item = Self::Item<'b>> + 'b>;
    type ItemMut<'b> = Option<&'b mut T>;
    type ItMut<'b> = Box<dyn Iterator<Item = Self::ItemMut<'b>> + 'b>;

    fn iter_prim(archetype: &ArchetypeStorage) -> Self::It<'_> {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => Box::new(unsafe { (*columns.get()).as_slice::<T>().iter() }.map(Some)),
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }

    fn iter_prim_mut(archetype: &ArchetypeStorage) -> Self::ItMut<'_> {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => {
                Box::new(unsafe { (*columns.get()).as_slice_mut::<T>().iter_mut() }.map(Some))
            }
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }

    unsafe fn iter_prim_unsafe(archetype: &ArchetypeStorage) -> Self::ItUnsafe<'_> {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => Box::new(
                unsafe { (*columns.get()).as_slice_mut::<T>().iter_mut() }
                    .map(|x| x as *mut _)
                    .map(Some),
            ),
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }

    fn fetch_prim(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::Item<'_>> {
        Some(archetype.get_component::<T>(index))
    }

    fn fetch_prim_mut(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::ItemMut<'_>> {
        Some(archetype.get_component_mut::<T>(index))
    }

    unsafe fn fetch_prim_unsafe(
        archetype: &ArchetypeStorage,
        index: RowIndex,
    ) -> Option<Self::ItemUnsafe<'_>> {
        Some(archetype.get_component_mut::<T>(index).map(|x| x as *mut _))
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
    type ItemUnsafe<'b> = *mut T;
    type ItUnsafe<'b> = Box<dyn Iterator<Item = Self::ItemUnsafe<'b>>>;
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

    unsafe fn fetch_prim_unsafe(
        archetype: &ArchetypeStorage,
        index: RowIndex,
    ) -> Option<Self::ItemUnsafe<'_>> {
        archetype.get_component_mut::<T>(index).map(|x| x as *mut _)
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
            .map(|columns| unsafe { (*columns.get()).as_slice::<T>().iter() })
            .into_iter()
            .flatten()
    }

    fn iter_prim_mut(archetype: &ArchetypeStorage) -> Self::ItMut<'_> {
        Self::iter_prim(archetype)
    }

    unsafe fn iter_prim_unsafe(archetype: &ArchetypeStorage) -> Self::ItUnsafe<'_> {
        Box::new(
            archetype
                .components
                .get(&TypeId::of::<T>())
                .map(|columns| unsafe {
                    let slice = (*columns.get()).as_slice_mut::<T>();
                    let ptr = slice.as_mut_ptr();
                    let len = slice.len();
                    (0..len).map(move |i| ptr.add(i))
                })
                .into_iter()
                .flatten(),
        )
    }
}

impl<'a, T: Component> QueryPrimitive for ArchQuery<&'a mut T> {
    type ItemUnsafe<'b> = *mut T;
    type ItUnsafe<'b> = Box<dyn Iterator<Item = *mut T>>;
    type Item<'b> = &'b T;
    type It<'b> = std::iter::Flatten<std::option::IntoIter<std::slice::Iter<'b, T>>>;
    type ItemMut<'b> = &'b mut T;
    type ItMut<'b> = std::iter::Flatten<std::option::IntoIter<std::slice::IterMut<'b, T>>>;

    fn iter_prim(archetype: &ArchetypeStorage) -> Self::It<'_> {
        archetype
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { (*columns.get()).as_slice::<T>().iter() })
            .into_iter()
            .flatten()
    }

    fn iter_prim_mut(archetype: &ArchetypeStorage) -> Self::ItMut<'_> {
        archetype
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { (*columns.get()).as_slice_mut::<T>().iter_mut() })
            .into_iter()
            .flatten()
    }

    unsafe fn iter_prim_unsafe(archetype: &ArchetypeStorage) -> Self::ItUnsafe<'_> {
        Box::new(
            archetype
                .components
                .get(&TypeId::of::<T>())
                .map(|columns| unsafe {
                    let slice = (*columns.get()).as_slice_mut::<T>();
                    let ptr = slice.as_mut_ptr();
                    let len = slice.len();
                    (0..len).map(move |i| ptr.add(i))
                })
                .into_iter()
                .flatten(),
        )
    }

    fn fetch_prim(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::Item<'_>> {
        archetype.get_component::<T>(index)
    }

    fn fetch_prim_mut(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::ItemMut<'_>> {
        archetype.get_component_mut::<T>(index)
    }

    unsafe fn fetch_prim_unsafe(
        archetype: &ArchetypeStorage,
        index: RowIndex,
    ) -> Option<Self::ItemUnsafe<'_>> {
        archetype.get_component_mut::<T>(index).map(|x| x as *mut _)
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

impl<T> QueryFragment for ArchQuery<T>
where
    ArchQuery<T>: QueryPrimitive,
{
    type ItemUnsafe<'a> = <Self as QueryPrimitive>::ItemUnsafe<'a>;
    type ItUnsafe<'a> = <Self as QueryPrimitive>::ItUnsafe<'a>;
    type Item<'a> = <Self as QueryPrimitive>::Item<'a>;
    type It<'a> = <Self as QueryPrimitive>::It<'a>;
    type ItemMut<'a> = <Self as QueryPrimitive>::ItemMut<'a>;
    type ItMut<'a> = <Self as QueryPrimitive>::ItMut<'a>;

    fn iter(archetype: &ArchetypeStorage) -> Self::It<'_> {
        Self::iter_prim(archetype)
    }

    fn iter_mut(archetype: &ArchetypeStorage) -> Self::ItMut<'_> {
        Self::iter_prim_mut(archetype)
    }

    fn fetch(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::Item<'_>> {
        Self::fetch_prim(archetype, index)
    }

    fn fetch_mut(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::ItemMut<'_>> {
        Self::fetch_prim_mut(archetype, index)
    }

    fn contains(archetype: &ArchetypeStorage) -> bool {
        Self::contains_prim(archetype)
    }

    fn types_mut(set: &mut HashSet<TypeId>) {
        <Self as QueryPrimitive>::types_mut(set);
    }

    fn types_const(set: &mut HashSet<TypeId>) {
        <Self as QueryPrimitive>::types_const(set);
    }

    unsafe fn fetch_unsafe(
        archetype: &ArchetypeStorage,
        index: RowIndex,
    ) -> Option<Self::ItemUnsafe<'_>> {
        Self::fetch_prim_unsafe(archetype, index)
    }

    unsafe fn iter_unsafe(archetype: &ArchetypeStorage) -> Self::ItUnsafe<'_> {
        Self::iter_prim_unsafe(archetype)
    }
}

// macro implementing more combinations
//

pub struct TupleIterator<Inner>(Inner);

unsafe impl<Inner> Send for TupleIterator<Inner> {}
unsafe impl<Inner> Sync for TupleIterator<Inner> {}

macro_rules! impl_tuple {
    ($($idx: tt : $t: ident),+ $(,)?) => {
        impl<$($t,)+> Iterator for TupleIterator<
            ($($t),+)
        >
        where
            $( $t: Iterator ),+
        {
            type Item = ( $( $t::Item ),* );

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

        impl<$($t,)+> QueryFragment for ArchQuery<($($t,)+)>
        where
        $(
            ArchQuery<$t>: QueryPrimitive,
        )+
        {
            type ItemUnsafe<'a> = ($(<ArchQuery<$t> as QueryPrimitive>::ItemUnsafe<'a>),+);
            type ItUnsafe<'a> = TupleIterator<($(<ArchQuery<$t> as QueryPrimitive>::ItUnsafe<'a>,)+)>;
            type Item<'a> = ($(<ArchQuery<$t> as QueryPrimitive>::Item<'a>),+);
            type It<'a> = TupleIterator<($(<ArchQuery<$t> as QueryPrimitive>::It<'a>,)+)>;
            type ItemMut<'a> = ($(<ArchQuery<$t> as QueryPrimitive>::ItemMut<'a>),+);
            type ItMut<'a> = TupleIterator<($(<ArchQuery<$t> as QueryPrimitive>::ItMut<'a>,)+)>;

            fn iter(archetype: &ArchetypeStorage) -> Self::It<'_>
            {
                TupleIterator(($( ArchQuery::<$t>::iter(archetype) ),+))
            }

            fn iter_mut(archetype: &ArchetypeStorage) -> Self::ItMut<'_>
            {
                TupleIterator(($( ArchQuery::<$t>::iter_mut(archetype) ),+))
            }

            unsafe fn iter_unsafe(archetype: &ArchetypeStorage) -> Self::ItUnsafe<'_>
            {
                TupleIterator(($( ArchQuery::<$t>::iter_unsafe(archetype) ),+))
            }

            fn fetch(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::Item<'_>> {
                Some((
                    $(
                        ArchQuery::<$t>::fetch(archetype, index)?,
                    )*
                ))
            }

            fn fetch_mut(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::ItemMut<'_>> {
                Some((
                    $(
                        ArchQuery::<$t>::fetch_mut(archetype, index)?,
                    )*
                ))
            }

            unsafe fn fetch_unsafe(archetype: &ArchetypeStorage, index: RowIndex) -> Option<Self::ItemUnsafe<'_>> {
                Some((
                    $(
                        ArchQuery::<$t>::fetch_unsafe(archetype, index)?,
                    )*
                ))
            }

            fn contains(archetype: &ArchetypeStorage) -> bool {
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
