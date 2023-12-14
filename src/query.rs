// FIXME: yeet the bloody Box<dyn Iterator> iterators please
//
pub mod filters;
pub mod resource_query;

#[cfg(test)]
mod query_tests;

use crate::{
    entity_id::EntityId,
    systems::SystemDescriptor,
    table::{ArchetypeHash, EntityTable},
    Component, RowIndex, World,
};
use filters::Filter;
use std::{any::TypeId, collections::HashSet, marker::PhantomData, ops::RangeBounds, slice};

pub(crate) trait WorldQuery<'a> {
    fn new(db: &'a World, system_idx: usize) -> Self;

    /// List of component types this query needs exclusive access to
    fn components_mut(_set: &mut HashSet<TypeId>) {}
    /// List of component types this query needs
    fn components_const(_set: &mut HashSet<TypeId>) {}
    /// List of resource types this query needs exclusive access to
    fn resources_mut(_set: &mut HashSet<TypeId>) {}
    /// List of resource types this query needs
    fn resources_const(_set: &mut HashSet<TypeId>) {}
    /// Return wether this system should run in isolation
    fn exclusive() -> bool {
        false
    }

    fn read_only() -> bool {
        false
    }
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

    pub fn from_system<T>(desc: &SystemDescriptor<T>) -> Self {
        Self {
            exclusive: (desc.exclusive)(),
            comp_mut: (desc.components_mut)(),
            res_mut: (desc.resources_mut)(),
            comp_const: (desc.components_const)(),
            res_const: (desc.resources_const)(),
        }
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

pub struct Query<'a, T, F = ()> {
    world: std::ptr::NonNull<crate::World>,
    _m: PhantomData<(T, F)>,
    _l: PhantomData<&'a ()>,
}

unsafe impl<T, F> Send for Query<'_, T, F> {}
unsafe impl<T, F> Sync for Query<'_, T, F> {}

impl<'a, T, F> WorldQuery<'a> for Query<'a, T, F>
where
    ArchQuery<T>: QueryFragment,
    F: Filter,
{
    fn new(db: &'a World, _system_idx: usize) -> Self {
        Self::new(db)
    }

    fn components_mut(set: &mut HashSet<TypeId>) {
        <ArchQuery<T> as QueryFragment>::types_mut(set);
    }

    fn components_const(set: &mut HashSet<TypeId>) {
        <ArchQuery<T> as QueryFragment>::types_const(set);
    }

    fn read_only() -> bool {
        <ArchQuery<T> as QueryFragment>::read_only()
    }
}

impl<'a, T, F> Query<'a, T, F>
where
    ArchQuery<T>: QueryFragment,
    F: Filter,
{
    pub fn new(world: &'a crate::World) -> Self {
        Query {
            world: std::ptr::NonNull::from(world),
            _m: PhantomData,
            _l: PhantomData,
        }
    }

    /// Return a query that is a subset of this query
    ///
    /// Subset means that the child query may not have more components than the original query.
    ///
    /// Mutable references may be demoted to const references, but const references may not be
    /// promoted to mutable references.
    ///
    /// The query has to be uniquely borrowed, because the subquery _may_ mutably borrow the same data as the parent query
    ///
    /// # Panics
    ///
    /// Panics if an invariant no longer holds.
    ///
    /// TODO: Allow extending filter
    /// TODO: Can we avoid the unique borrow of the parent Query?
    ///
    /// # Safety
    ///
    /// Currently the API allows multiple mutable borrows to the same data. The caller must ensure
    /// that all mutable data access is unique.
    ///
    /// ```
    /// use cecs::prelude::*;
    /// # #[derive(Clone, Copy)]
    /// # struct A;
    /// # #[derive(Clone, Copy)]
    /// # struct B;
    /// # #[derive(Clone, Copy)]
    /// # struct C;
    ///
    /// let mut world = World::new(4);
    ///
    /// let e = world.insert_entity();
    /// world.set_bundle(e, (A, B, C)).unwrap();
    ///
    /// let mut q = Query::<(EntityId, &mut A, &mut B, &C)>::new(&world);
    ///
    /// let sub: Query<(&A, &C, EntityId)> = unsafe { q.subset() };
    ///
    /// let mut count = 0;
    /// for (_a, _c, id) in sub.iter() {
    ///     assert_eq!(id, e);
    ///     count += 1;
    /// }
    /// assert_eq!(count, 1);
    /// ```
    pub unsafe fn subset<'b, T1>(&'b mut self) -> Query<'b, T1, F>
    where
        ArchQuery<T1>: QueryFragment,
    {
        #[cfg(debug_assertions)]
        {
            let mut rhs = HashSet::new();
            let p = ensure_query_valid::<Query<T1, F>>();
            let mut lhs = p.comp_mut;

            Self::components_mut(&mut rhs);
            Query::<T1, F>::components_mut(&mut lhs);

            assert!(lhs.is_subset(&rhs));

            // T1 const components must be a subset of rhs const+mut
            Self::components_const(&mut rhs);
            let lhs = p.comp_const;

            assert!(lhs.is_subset(&rhs));
        }
        unsafe { Query::<T1, F>::new(self.world.as_ref()) }
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

    pub fn single(&self) -> Option<<ArchQuery<T> as QueryFragment>::Item<'a>> {
        self.iter().next()
    }

    pub fn single_mut(&mut self) -> Option<<ArchQuery<T> as QueryFragment>::ItemMut<'a>> {
        self.iter_mut().next()
    }

    pub fn iter(&self) -> impl Iterator<Item = <ArchQuery<T> as QueryFragment>::Item<'a>> {
        unsafe {
            self.world
                .as_ref()
                .archetypes
                .iter()
                .filter(|(_, arch)| F::filter(arch))
                .flat_map(|(_, arch)| ArchQuery::<T>::iter(arch))
        }
    }

    pub fn iter_mut(
        &mut self,
    ) -> impl Iterator<Item = <ArchQuery<T> as QueryFragment>::ItemMut<'a>> {
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
    pub unsafe fn iter_unsafe(
        &self,
    ) -> impl Iterator<Item = <ArchQuery<T> as QueryFragment>::ItemUnsafe<'a>> {
        self.world
            .as_ref()
            .archetypes
            .iter()
            .filter(|(_, arch)| F::filter(arch))
            .flat_map(|(_, arch)| ArchQuery::<T>::iter_unsafe(arch))
    }

    pub fn fetch(&self, id: EntityId) -> Option<<ArchQuery<T> as QueryFragment>::Item<'a>> {
        unsafe {
            let (arch, index) = self.world.as_ref().entity_ids().read(id).ok()?;
            if !F::filter(arch.as_ref()) {
                return None;
            }

            ArchQuery::<T>::fetch(arch.as_ref(), index)
        }
    }

    pub fn fetch_mut(
        &mut self,
        id: EntityId,
    ) -> Option<<ArchQuery<T> as QueryFragment>::ItemMut<'a>> {
        unsafe {
            let (arch, index) = self.world.as_ref().entity_ids().read(id).ok()?;
            if !F::filter(arch.as_ref()) {
                return None;
            }

            ArchQuery::<T>::fetch_mut(arch.as_ref(), index)
        }
    }

    pub unsafe fn fetch_unsafe(
        &self,
        id: EntityId,
    ) -> Option<<ArchQuery<T> as QueryFragment>::ItemUnsafe<'a>> {
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
    pub fn one(&self) -> <ArchQuery<T> as QueryFragment>::Item<'a> {
        self.iter().next().unwrap()
    }

    #[cfg(feature = "parallel")]
    pub fn par_for_each<'b>(
        &'b self,
        f: impl Fn(<ArchQuery<T> as QueryFragment>::Item<'a>) + Sync + 'b,
    ) where
        T: Send + Sync,
    {
        unsafe {
            let world = self.world.as_ref();
            let pool = &world.job_system;
            pool.scope(|s| {
                let f = &f;
                world
                    .archetypes
                    .iter()
                    // TODO: should filters run inside the jobs instead?
                    // currently I anticipate that filters are inexpensive, so it seems cheaper to
                    // filter ahead of job creation
                    .filter(|(_, arch)| !arch.is_empty() && F::filter(arch))
                    .for_each(|(_, arch)| {
                        let batch_size = arch.len() / pool.parallelism() + 1;
                        // TODO: the job allocator could probably help greatly with these jobs
                        for range in batches(arch.len(), batch_size) {
                            s.spawn(move |_s| {
                                for t in ArchQuery::<T>::iter_range(arch, range) {
                                    f(t);
                                }
                            })
                        }
                    });
            });
        }
    }

    #[cfg(feature = "parallel")]
    pub fn par_for_each_mut<'b>(
        &'b mut self,
        f: impl Fn(<ArchQuery<T> as QueryFragment>::ItemMut<'a>) + Sync + 'b,
    ) where
        T: Send + Sync,
    {
        unsafe {
            let world = self.world.as_ref();
            let pool = &world.job_system;
            pool.scope(|s| {
                let f = &f;
                world
                    .archetypes
                    .iter()
                    // TODO: should filters run inside the jobs instead?
                    // currently I anticipate that filters are inexpensive, so it seems cheaper to
                    // filter ahead of job creation
                    .filter(|(_, arch)| !arch.is_empty() && F::filter(arch))
                    .for_each(|(_, arch)| {
                        // TODO: should take size of queried types into account?
                        let batch_size = arch.len() / pool.parallelism() + 1;

                        // TODO: the job allocator could probably help greatly with these jobs
                        for range in batches(arch.len(), batch_size) {
                            s.spawn(move |_s| {
                                for t in ArchQuery::<T>::iter_range_mut(arch, range) {
                                    f(t);
                                }
                            })
                        }
                    });
            });
        }
    }

    #[cfg(not(feature = "parallel"))]
    pub fn par_for_each<'b>(
        &'b self,
        f: impl Fn(<ArchQuery<T> as QueryFragment>::Item<'a>) + Sync + 'b,
    ) where
        T: Send + Sync,
    {
        self.iter().for_each(f);
    }

    #[cfg(not(feature = "parallel"))]
    pub fn par_for_each_mut<'b>(
        &'b mut self,
        f: impl Fn(<ArchQuery<T> as QueryFragment>::ItemMut<'a>) + Sync + 'b,
    ) where
        T: Send + Sync,
    {
        self.iter_mut().for_each(f);
    }
}

#[allow(unused)]
fn batches(len: usize, batch_size: usize) -> impl Iterator<Item = impl RangeBounds<usize> + Clone> {
    (0..len / batch_size)
        .map(move |i| {
            let s = i * batch_size;
            s..s + batch_size
        })
        // last batch if len is not divisible by batch_size
        // otherwise it's an empty range
        .chain(std::iter::once((len - (len % batch_size))..len))
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

    unsafe fn iter_unsafe(archetype: &EntityTable) -> Self::ItUnsafe<'_>;
    unsafe fn fetch_unsafe(
        archetype: &EntityTable,
        index: RowIndex,
    ) -> Option<Self::ItemUnsafe<'_>>;
    fn iter(archetype: &EntityTable) -> Self::It<'_>;
    fn iter_mut(archetype: &EntityTable) -> Self::ItMut<'_>;
    fn fetch(archetype: &EntityTable, index: RowIndex) -> Option<Self::Item<'_>>;
    fn fetch_mut(archetype: &EntityTable, index: RowIndex) -> Option<Self::ItemMut<'_>>;
    fn types_mut(set: &mut HashSet<TypeId>);
    fn types_const(set: &mut HashSet<TypeId>);
    fn contains(archetype: &EntityTable) -> bool;
    fn read_only() -> bool;
    fn iter_range(archetype: &EntityTable, range: impl RangeBounds<usize> + Clone) -> Self::It<'_>;
    fn iter_range_mut(
        archetype: &EntityTable,
        range: impl RangeBounds<usize> + Clone,
    ) -> Self::ItMut<'_>;
}

pub trait QueryPrimitive {
    type ItemUnsafe<'a>;
    type ItUnsafe<'a>: Iterator<Item = Self::ItemUnsafe<'a>>;
    type Item<'a>;
    type It<'a>: Iterator<Item = Self::Item<'a>>;
    type ItemMut<'a>;
    type ItMut<'a>: Iterator<Item = Self::ItemMut<'a>>;

    unsafe fn iter_prim_unsafe(archetype: &EntityTable) -> Self::ItUnsafe<'_>;
    unsafe fn fetch_prim_unsafe(
        archetype: &EntityTable,
        index: RowIndex,
    ) -> Option<Self::ItemUnsafe<'_>>;
    fn iter_prim(archetype: &EntityTable) -> Self::It<'_>;
    fn iter_prim_mut(archetype: &EntityTable) -> Self::ItMut<'_>;
    fn fetch_prim(archetype: &EntityTable, index: RowIndex) -> Option<Self::Item<'_>>;
    fn fetch_prim_mut(archetype: &EntityTable, index: RowIndex) -> Option<Self::ItemMut<'_>>;
    fn contains_prim(archetype: &EntityTable) -> bool;
    fn types_mut(set: &mut HashSet<TypeId>);
    fn types_const(set: &mut HashSet<TypeId>);
    fn read_only() -> bool;
    fn iter_range_prim(archetype: &EntityTable, range: impl RangeBounds<usize>) -> Self::It<'_>;
    fn iter_range_prim_mut(
        archetype: &EntityTable,
        range: impl RangeBounds<usize>,
    ) -> Self::ItMut<'_>;
}

impl QueryPrimitive for ArchQuery<EntityId> {
    type ItemUnsafe<'a> = EntityId;
    type ItUnsafe<'a> = std::iter::Copied<std::slice::Iter<'a, EntityId>>;
    type Item<'a> = EntityId;
    type It<'a> = std::iter::Copied<std::slice::Iter<'a, EntityId>>;
    type ItemMut<'a> = EntityId;
    type ItMut<'a> = std::iter::Copied<std::slice::Iter<'a, EntityId>>;

    unsafe fn iter_prim_unsafe<'a>(archetype: &'a EntityTable) -> Self::ItUnsafe<'a> {
        archetype.entities.iter().copied()
    }

    unsafe fn fetch_prim_unsafe(
        archetype: &EntityTable,
        index: RowIndex,
    ) -> Option<Self::ItemUnsafe<'_>> {
        archetype.entities.get(index as usize).copied()
    }

    fn iter_prim<'a>(archetype: &'a EntityTable) -> Self::It<'a> {
        archetype.entities.iter().copied()
    }

    fn iter_prim_mut<'a>(archetype: &'a EntityTable) -> Self::ItMut<'a> {
        Self::iter_prim(archetype)
    }

    fn fetch_prim(archetype: &EntityTable, index: RowIndex) -> Option<Self::Item<'_>> {
        archetype.entities.get(index as usize).copied()
    }

    fn fetch_prim_mut(archetype: &EntityTable, index: RowIndex) -> Option<Self::ItemMut<'_>> {
        Self::fetch_prim(archetype, index)
    }

    fn contains_prim(_archetype: &EntityTable) -> bool {
        true
    }

    fn types_mut(_set: &mut HashSet<TypeId>) {
        // noop
    }

    fn types_const(_set: &mut HashSet<TypeId>) {
        // noop
        // entity_id is not considered while scheduling
    }

    fn read_only() -> bool {
        true
    }

    fn iter_range_prim(archetype: &EntityTable, range: impl RangeBounds<usize>) -> Self::It<'_> {
        let len = archetype.entities.len();
        let range = slice::range(range, ..len);
        archetype.entities[range].iter().copied()
    }

    fn iter_range_prim_mut(
        archetype: &EntityTable,
        range: impl RangeBounds<usize>,
    ) -> Self::ItMut<'_> {
        Self::iter_range_prim(archetype, range)
    }
}

impl QueryPrimitive for ArchQuery<ArchetypeHash> {
    type ItemUnsafe<'a> = ArchetypeHash;
    type Item<'a> = ArchetypeHash;
    type ItemMut<'a> = ArchetypeHash;
    type It<'a> = Box<dyn Iterator<Item = Self::Item<'a>> + 'a>;
    type ItUnsafe<'a> = Self::It<'a>;
    type ItMut<'a> = Self::It<'a>;

    unsafe fn iter_prim_unsafe<'a>(archetype: &'a EntityTable) -> Self::ItUnsafe<'a> {
        Self::iter_prim(archetype)
    }

    unsafe fn fetch_prim_unsafe(
        archetype: &EntityTable,
        index: RowIndex,
    ) -> Option<Self::ItemUnsafe<'_>> {
        Self::fetch_prim(archetype, index)
    }

    fn iter_prim<'a>(archetype: &'a EntityTable) -> Self::It<'a> {
        let hash = archetype.ty;
        Box::new((0..archetype.rows).map(move |_| ArchetypeHash(hash)))
    }

    fn iter_prim_mut<'a>(archetype: &'a EntityTable) -> Self::ItMut<'a> {
        Self::iter_prim(archetype)
    }

    fn fetch_prim(archetype: &EntityTable, index: RowIndex) -> Option<Self::Item<'_>> {
        archetype
            .entities
            .get(index as usize)
            .map(|_| ArchetypeHash(archetype.ty))
    }

    fn fetch_prim_mut(archetype: &EntityTable, index: RowIndex) -> Option<Self::ItemMut<'_>> {
        Self::fetch_prim(archetype, index)
    }

    fn contains_prim(_archetype: &EntityTable) -> bool {
        true
    }

    fn types_mut(_set: &mut HashSet<TypeId>) {
        // noop
    }

    fn types_const(_set: &mut HashSet<TypeId>) {
        // noop
    }

    fn read_only() -> bool {
        true
    }

    fn iter_range_prim(archetype: &EntityTable, range: impl RangeBounds<usize>) -> Self::It<'_> {
        let len = archetype.entities.len();
        let range = slice::range(range, ..len);
        let hash = archetype.ty;
        Box::new(
            archetype.entities[range]
                .iter()
                .map(move |_| ArchetypeHash(hash)),
        )
    }

    fn iter_range_prim_mut(
        archetype: &EntityTable,
        range: impl RangeBounds<usize>,
    ) -> Self::ItMut<'_> {
        Self::iter_range_prim(archetype, range)
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

    fn iter_prim(archetype: &EntityTable) -> Self::It<'_> {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => {
                Box::new(unsafe { (&*columns.get()).as_slice::<T>().iter() }.map(Some))
            }
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }

    fn iter_prim_mut(archetype: &EntityTable) -> Self::ItMut<'_> {
        Self::iter_prim(archetype)
    }

    unsafe fn iter_prim_unsafe(archetype: &EntityTable) -> Self::ItUnsafe<'_> {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => Box::new(
                unsafe { (&mut *columns.get()).as_slice_mut::<T>().iter_mut() }
                    .map(|x| x as *mut _)
                    .map(Some),
            ),
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }

    fn fetch_prim(archetype: &EntityTable, index: RowIndex) -> Option<Self::Item<'_>> {
        Some(archetype.get_component::<T>(index))
    }

    fn fetch_prim_mut(archetype: &EntityTable, index: RowIndex) -> Option<Self::ItemMut<'_>> {
        Self::fetch_prim(archetype, index)
    }

    unsafe fn fetch_prim_unsafe(
        archetype: &EntityTable,
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

    fn contains_prim(_archetype: &EntityTable) -> bool {
        true
    }

    fn read_only() -> bool {
        true
    }

    fn iter_range_prim(archetype: &EntityTable, range: impl RangeBounds<usize>) -> Self::It<'_> {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => unsafe {
                let col = (&*columns.get()).as_slice::<T>();
                let len = col.len();
                let range = slice::range(range, ..len);
                Box::new(col[range].iter().map(Some))
            },
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }

    fn iter_range_prim_mut(
        archetype: &EntityTable,
        range: impl RangeBounds<usize>,
    ) -> Self::ItMut<'_> {
        Self::iter_range_prim(archetype, range)
    }
}

impl<'a, T: Component> QueryPrimitive for ArchQuery<Option<&'a mut T>> {
    type ItemUnsafe<'b> = Option<*mut T>;
    type ItUnsafe<'b> = Box<dyn Iterator<Item = Self::ItemUnsafe<'b>> + 'b>;
    type Item<'b> = Option<&'b T>;
    type It<'b> = Box<dyn Iterator<Item = Self::Item<'b>> + 'b>;
    type ItemMut<'b> = Option<&'b mut T>;
    type ItMut<'b> = Box<dyn Iterator<Item = Self::ItemMut<'b>> + 'b>;

    fn iter_prim(archetype: &EntityTable) -> Self::It<'_> {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => {
                Box::new(unsafe { (&*columns.get()).as_slice::<T>().iter() }.map(Some))
            }
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }

    fn iter_prim_mut(archetype: &EntityTable) -> Self::ItMut<'_> {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => {
                Box::new(unsafe { (&mut *columns.get()).as_slice_mut::<T>().iter_mut() }.map(Some))
            }
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }

    unsafe fn iter_prim_unsafe(archetype: &EntityTable) -> Self::ItUnsafe<'_> {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => Box::new(
                unsafe { (&mut *columns.get()).as_slice_mut::<T>().iter_mut() }
                    .map(|x| x as *mut _)
                    .map(Some),
            ),
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }

    fn fetch_prim(archetype: &EntityTable, index: RowIndex) -> Option<Self::Item<'_>> {
        Some(archetype.get_component::<T>(index))
    }

    fn fetch_prim_mut(archetype: &EntityTable, index: RowIndex) -> Option<Self::ItemMut<'_>> {
        Some(archetype.get_component_mut::<T>(index))
    }

    unsafe fn fetch_prim_unsafe(
        archetype: &EntityTable,
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

    fn contains_prim(_archetype: &EntityTable) -> bool {
        true
    }

    fn read_only() -> bool {
        false
    }

    fn iter_range_prim(archetype: &EntityTable, range: impl RangeBounds<usize>) -> Self::It<'_> {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => unsafe {
                let col = (&mut *columns.get()).as_slice::<T>();
                let len = col.len();
                let range = slice::range(range, ..len);
                Box::new(col[range].iter().map(Some))
            },
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }

    fn iter_range_prim_mut(
        archetype: &EntityTable,
        range: impl RangeBounds<usize>,
    ) -> Self::ItMut<'_> {
        match archetype.components.get(&TypeId::of::<T>()) {
            Some(columns) => unsafe {
                let col = (&mut *columns.get()).as_slice_mut::<T>();
                let len = col.len();
                let range = slice::range(range, ..len);
                Box::new(col[range].iter_mut().map(Some))
            },
            None => Box::new((0..archetype.rows).map(|_| None)),
        }
    }
}

impl<'a, T: Component> QueryPrimitive for ArchQuery<&'a T> {
    type ItemUnsafe<'b> = *mut T;
    type ItUnsafe<'b> = Box<dyn Iterator<Item = Self::ItemUnsafe<'b>>>;
    type Item<'b> = &'b T;
    type It<'b> = std::iter::Flatten<std::option::IntoIter<std::slice::Iter<'b, T>>>;
    type ItemMut<'b> = &'b T;
    type ItMut<'b> = std::iter::Flatten<std::option::IntoIter<std::slice::Iter<'b, T>>>;

    fn fetch_prim(archetype: &EntityTable, index: RowIndex) -> Option<Self::Item<'_>> {
        archetype.get_component::<T>(index)
    }

    fn fetch_prim_mut(archetype: &EntityTable, index: RowIndex) -> Option<Self::ItemMut<'_>> {
        Self::fetch_prim(archetype, index)
    }

    unsafe fn fetch_prim_unsafe(
        archetype: &EntityTable,
        index: RowIndex,
    ) -> Option<Self::ItemUnsafe<'_>> {
        archetype.get_component_mut::<T>(index).map(|x| x as *mut _)
    }

    fn contains_prim(archetype: &EntityTable) -> bool {
        archetype.contains_column::<T>()
    }

    fn types_mut(_set: &mut HashSet<TypeId>) {
        // noop
    }

    fn types_const(set: &mut HashSet<TypeId>) {
        set.insert(TypeId::of::<T>());
    }

    fn iter_prim(archetype: &EntityTable) -> Self::It<'_> {
        archetype
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { (&*columns.get()).as_slice::<T>().iter() })
            .into_iter()
            .flatten()
    }

    fn iter_prim_mut(archetype: &EntityTable) -> Self::ItMut<'_> {
        Self::iter_prim(archetype)
    }

    unsafe fn iter_prim_unsafe(archetype: &EntityTable) -> Self::ItUnsafe<'_> {
        Box::new(
            archetype
                .components
                .get(&TypeId::of::<T>())
                .map(|columns| unsafe {
                    let slice = (&mut *columns.get()).as_slice_mut::<T>();
                    let ptr = slice.as_mut_ptr();
                    let len = slice.len();
                    (0..len).map(move |i| ptr.add(i))
                })
                .into_iter()
                .flatten(),
        )
    }

    fn read_only() -> bool {
        true
    }

    fn iter_range_prim(archetype: &EntityTable, range: impl RangeBounds<usize>) -> Self::ItMut<'_> {
        archetype
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe {
                let col = (&*columns.get()).as_slice::<T>();
                let len = col.len();
                let range = slice::range(range, ..len);
                col[range].iter()
            })
            .into_iter()
            .flatten()
    }

    fn iter_range_prim_mut(
        archetype: &EntityTable,
        range: impl RangeBounds<usize>,
    ) -> Self::ItMut<'_> {
        Self::iter_range_prim(archetype, range)
    }
}

impl<'a, T: Component> QueryPrimitive for ArchQuery<&'a mut T> {
    type ItemUnsafe<'b> = *mut T;
    type ItUnsafe<'b> = Box<dyn Iterator<Item = *mut T>>;
    type Item<'b> = &'b T;
    type It<'b> = std::iter::Flatten<std::option::IntoIter<std::slice::Iter<'b, T>>>;
    type ItemMut<'b> = &'b mut T;
    type ItMut<'b> = std::iter::Flatten<std::option::IntoIter<std::slice::IterMut<'b, T>>>;

    fn iter_prim(archetype: &EntityTable) -> Self::It<'_> {
        archetype
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { (&*columns.get()).as_slice::<T>().iter() })
            .into_iter()
            .flatten()
    }

    fn iter_prim_mut(archetype: &EntityTable) -> Self::ItMut<'_> {
        archetype
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { (&mut *columns.get()).as_slice_mut::<T>().iter_mut() })
            .into_iter()
            .flatten()
    }

    unsafe fn iter_prim_unsafe(archetype: &EntityTable) -> Self::ItUnsafe<'_> {
        Box::new(
            archetype
                .components
                .get(&TypeId::of::<T>())
                .map(|columns| unsafe {
                    let slice = (&mut *columns.get()).as_slice_mut::<T>();
                    let ptr = slice.as_mut_ptr();
                    let len = slice.len();
                    (0..len).map(move |i| ptr.add(i))
                })
                .into_iter()
                .flatten(),
        )
    }

    fn fetch_prim(archetype: &EntityTable, index: RowIndex) -> Option<Self::Item<'_>> {
        archetype.get_component::<T>(index)
    }

    fn fetch_prim_mut(archetype: &EntityTable, index: RowIndex) -> Option<Self::ItemMut<'_>> {
        archetype.get_component_mut::<T>(index)
    }

    unsafe fn fetch_prim_unsafe(
        archetype: &EntityTable,
        index: RowIndex,
    ) -> Option<Self::ItemUnsafe<'_>> {
        archetype.get_component_mut::<T>(index).map(|x| x as *mut _)
    }

    fn contains_prim(archetype: &EntityTable) -> bool {
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

    fn read_only() -> bool {
        false
    }

    fn iter_range_prim(archetype: &EntityTable, range: impl RangeBounds<usize>) -> Self::It<'_> {
        archetype
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe {
                let col = (&mut *columns.get()).as_slice::<T>();
                let len = col.len();
                let range = slice::range(range, ..len);
                col[range].iter()
            })
            .into_iter()
            .flatten()
    }

    fn iter_range_prim_mut(
        archetype: &EntityTable,
        range: impl RangeBounds<usize>,
    ) -> Self::ItMut<'_> {
        archetype
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe {
                let col = (&mut *columns.get()).as_slice_mut::<T>();
                let len = col.len();
                let range = slice::range(range, ..len);
                col[range].iter_mut()
            })
            .into_iter()
            .flatten()
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

    fn iter(archetype: &EntityTable) -> Self::It<'_> {
        Self::iter_prim(archetype)
    }

    fn iter_mut(archetype: &EntityTable) -> Self::ItMut<'_> {
        Self::iter_prim_mut(archetype)
    }

    fn fetch(archetype: &EntityTable, index: RowIndex) -> Option<Self::Item<'_>> {
        Self::fetch_prim(archetype, index)
    }

    fn fetch_mut(archetype: &EntityTable, index: RowIndex) -> Option<Self::ItemMut<'_>> {
        Self::fetch_prim_mut(archetype, index)
    }

    fn contains(archetype: &EntityTable) -> bool {
        Self::contains_prim(archetype)
    }

    fn types_mut(set: &mut HashSet<TypeId>) {
        <Self as QueryPrimitive>::types_mut(set);
    }

    fn types_const(set: &mut HashSet<TypeId>) {
        <Self as QueryPrimitive>::types_const(set);
    }

    unsafe fn fetch_unsafe(
        archetype: &EntityTable,
        index: RowIndex,
    ) -> Option<Self::ItemUnsafe<'_>> {
        Self::fetch_prim_unsafe(archetype, index)
    }

    unsafe fn iter_unsafe(archetype: &EntityTable) -> Self::ItUnsafe<'_> {
        Self::iter_prim_unsafe(archetype)
    }

    fn read_only() -> bool {
        <Self as QueryPrimitive>::read_only()
    }

    fn iter_range(archetype: &EntityTable, range: impl RangeBounds<usize> + Clone) -> Self::It<'_> {
        <Self as QueryPrimitive>::iter_range_prim(archetype, range)
    }

    fn iter_range_mut(
        archetype: &EntityTable,
        range: impl RangeBounds<usize> + Clone,
    ) -> Self::ItMut<'_> {
        <Self as QueryPrimitive>::iter_range_prim_mut(archetype, range)
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

            fn iter(archetype: &EntityTable) -> Self::It<'_>
            {
                TupleIterator(($( ArchQuery::<$t>::iter(archetype) ),+))
            }

            fn iter_mut(archetype: &EntityTable) -> Self::ItMut<'_>
            {
                TupleIterator(($( ArchQuery::<$t>::iter_mut(archetype) ),+))
            }

            fn iter_range(
                archetype: &EntityTable,
                range: impl RangeBounds<usize> + Clone,
            ) -> Self::It<'_> {
                TupleIterator(($( ArchQuery::<$t>::iter_range(archetype, range.clone()) ),+))
            }

            fn iter_range_mut(
                archetype: &EntityTable,
                range: impl RangeBounds<usize> + Clone,
            ) -> Self::ItMut<'_> {
                TupleIterator(($( ArchQuery::<$t>::iter_range_mut(archetype, range.clone()) ),+))
            }

            unsafe fn iter_unsafe(archetype: &EntityTable) -> Self::ItUnsafe<'_>
            {
                TupleIterator(($( ArchQuery::<$t>::iter_unsafe(archetype) ),+))
            }

            fn fetch(archetype: &EntityTable, index: RowIndex) -> Option<Self::Item<'_>> {
                Some((
                    $(
                        ArchQuery::<$t>::fetch(archetype, index)?,
                    )*
                ))
            }

            fn fetch_mut(archetype: &EntityTable, index: RowIndex) -> Option<Self::ItemMut<'_>> {
                Some((
                    $(
                        ArchQuery::<$t>::fetch_mut(archetype, index)?,
                    )*
                ))
            }

            unsafe fn fetch_unsafe(archetype: &EntityTable, index: RowIndex) -> Option<Self::ItemUnsafe<'_>> {
                Some((
                    $(
                        ArchQuery::<$t>::fetch_unsafe(archetype, index)?,
                    )*
                ))
            }

            fn contains(archetype: &EntityTable) -> bool {
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

            fn read_only() -> bool {
                false $(|| <ArchQuery<$t> as QueryPrimitive>::read_only())+
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
