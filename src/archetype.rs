use crate::{entity_id::EntityId, hash_ty, hash_type_id, Component, RowIndex, TypeHash};
use std::{alloc::Layout, any::TypeId, cell::UnsafeCell, collections::BTreeMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArchetypeHash(pub TypeHash);

pub struct ArchetypeStorage {
    pub(crate) ty: TypeHash,
    pub(crate) rows: u32,
    pub(crate) entities: Vec<EntityId>,
    pub(crate) components: BTreeMap<TypeId, UnsafeCell<ErasedTable>>,
}

unsafe impl Send for ArchetypeStorage {}
unsafe impl Sync for ArchetypeStorage {}

#[cfg(feature = "clone")]
impl Clone for ArchetypeStorage {
    fn clone(&self) -> Self {
        Self {
            ty: self.ty,
            rows: self.rows,
            entities: self.entities.clone(),
            components: self
                .components
                .iter()
                .map(|(ty, col)| unsafe { (*ty, UnsafeCell::new((*col.get()).clone())) })
                .collect(),
        }
    }
}

impl std::fmt::Debug for ArchetypeStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchetypeStorage")
            .field("rows", &self.rows)
            .field(
                "entities",
                &self
                    .entities
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>(),
            )
            .field(
                "components",
                &self
                    .components
                    .iter()
                    .map(|(_, c)| unsafe { &*c.get() }.ty_name)
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl ArchetypeStorage {
    pub fn empty() -> Self {
        let ty = hash_ty::<()>();
        let mut components = BTreeMap::new();
        components.insert(
            TypeId::of::<()>(),
            UnsafeCell::new(ErasedTable::new::<()>(0)),
        );
        Self {
            ty,
            rows: 0,
            entities: Vec::default(),
            components,
        }
    }

    /// Get the archetype storage's ty.
    pub fn ty(&self) -> TypeHash {
        self.ty
    }

    pub fn len(&self) -> usize {
        self.rows as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// return the updated entityid, if any
    #[must_use]
    pub fn remove(&mut self, row_index: RowIndex) -> Option<EntityId> {
        for (_, storage) in self.components.iter_mut() {
            storage.get_mut().remove(row_index);
        }
        self.entities.swap_remove(row_index as usize);
        self.rows -= 1;
        // if we have remaining entities, and the removed entity was not the last
        (self.rows > 0 && row_index < self.rows).then(|| self.entities[row_index as usize])
    }

    pub fn insert_entity(&mut self, id: EntityId) -> RowIndex {
        let res = self.rows;
        self.entities.push(id);
        self.rows += 1;
        res
    }

    /// return the new index in `dst` and the entity that has been moved to this one's position, if
    /// any
    #[must_use]
    pub fn move_entity(&mut self, dst: &mut Self, index: RowIndex) -> (RowIndex, Option<EntityId>) {
        let entity_id = self.entities.swap_remove(index as usize);
        let res = dst.insert_entity(entity_id);
        let moved = unsafe { self.move_entity_noinsert(dst, index) };
        (res, moved)
    }

    /// return the moved entity in `self`, if any
    #[must_use]
    pub fn move_entity_into(
        &mut self,
        src_index: RowIndex,
        dst: &mut Self,
        dst_index: RowIndex,
    ) -> Option<EntityId> {
        self.entities.swap_remove(src_index as usize);
        self.rows -= 1;
        let mut moved = None;
        if self.rows > 0 && src_index < self.rows {
            // if we have remaining rows, and the removed row was not the last
            moved = Some(self.entities[src_index as usize]);
        }
        for (ty, col) in self.components.iter_mut() {
            if let Some(dst) = dst.components.get_mut(ty) {
                (col.get_mut().move_row_into)(col.get_mut(), src_index, dst.get_mut(), dst_index);
            } else {
                // destination does not have this column
                col.get_mut().remove(src_index);
            }
        }
        moved
    }

    /// Move components into the last entity of `dst`
    ///
    /// # SAFETY
    ///
    /// Caller must ensure that an entity is inserted
    #[must_use]
    pub unsafe fn move_entity_noinsert(
        &mut self,
        dst: &mut Self,
        src_index: RowIndex,
    ) -> Option<EntityId> {
        debug_assert!(self.rows > 0);
        debug_assert!(src_index < self.rows);
        self.rows -= 1;
        let mut moved = None;
        if self.rows > 0 && src_index < self.rows {
            // if we have remaining rows, and the removed row was not the last
            moved = Some(self.entities[src_index as usize]);
        }
        for (ty, col) in self.components.iter_mut() {
            if let Some(dst) = dst.components.get_mut(ty) {
                (col.get_mut().move_row)(col.get_mut(), dst.get_mut(), src_index);
            } else {
                // destination does not have this column
                col.get_mut().remove(src_index);
            }
        }
        moved
    }

    pub fn set_component<T: 'static>(&mut self, row_index: RowIndex, val: T) {
        unsafe {
            let table = self
                .components
                .get_mut(&TypeId::of::<T>())
                .expect("set_component called on bad archetype")
                .get_mut();

            let v = table.as_slice_mut();
            let row_index = row_index as usize;
            assert!(row_index <= v.len());
            if row_index == v.len() {
                table.push(val);
            } else {
                v[row_index as usize] = val;
            }
        }
    }

    pub fn contains_column<T: 'static>(&self) -> bool {
        let hash = TypeId::of::<T>();
        self.contains_column_ty(hash)
    }

    pub fn contains_column_ty(&self, ty: TypeId) -> bool {
        self.components.contains_key(&ty)
    }

    pub fn extended_hash<T: Component>(&self) -> TypeHash {
        self.extended_hash_ty(hash_ty::<T>())
    }

    pub fn extended_hash_ty(&self, ty: TypeHash) -> TypeHash {
        self.ty ^ ty
    }

    pub fn extend_with_column<T: Component>(mut self) -> Self {
        if !self.contains_column::<T>() {
            let new_ty = self.extended_hash::<T>();
            self.ty = new_ty;
            self.components
                .insert(TypeId::of::<T>(), UnsafeCell::new(ErasedTable::new::<T>(0)));
        }
        self
    }

    /// Creates a new archetype that holds tables with both `self` and `rhs` columns
    pub fn merged(&self, rhs: &Self) -> Self {
        let mut result = self.clone_empty();
        for (col, table) in rhs.components.iter() {
            if !self.contains_column_ty(*col) {
                let table = unsafe { &*table.get() };
                result.ty = result.extended_hash_ty(hash_type_id(*col));
                result
                    .components
                    .insert(*col, UnsafeCell::new((table.clone_empty)()));
            }
        }
        result
    }

    /// Swap all components of two entities
    pub fn swap_components(&mut self, a: RowIndex, b: RowIndex) {
        for table in self.components.values_mut() {
            let table = table.get_mut();
            (table.swap_rows)(table, a, b);
        }
    }

    pub fn reduce_with_column<T: Component>(&self) -> Self {
        assert!(self.contains_column::<T>());

        let mut result = self.clone_empty();
        let new_ty = self.extended_hash::<T>();
        result.ty = new_ty;
        result.components.remove(&TypeId::of::<T>()).unwrap();
        result
    }

    pub fn clone_empty(&self) -> Self {
        Self {
            ty: self.ty,
            rows: 0,
            entities: Vec::with_capacity(self.entities.len()),
            components: BTreeMap::from_iter(
                self.components
                    .iter()
                    .map(|(id, col)| (*id, (unsafe { &*col.get() }.clone_empty)()))
                    .map(|(id, col)| (id, UnsafeCell::new(col))),
            ),
        }
    }

    pub fn get_component<T: 'static>(&self, row: RowIndex) -> Option<&T> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|columns| unsafe { (*columns.get()).as_slice().get(row as usize) })
    }

    pub fn get_component_mut<T: 'static>(&self, row: RowIndex) -> Option<&mut T> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|columns| unsafe { (*columns.get()).as_slice_mut().get_mut(row as usize) })
    }
}

/// Type erased Vec
pub(crate) struct ErasedTable {
    pub(crate) ty_name: &'static str,
    // Vec //
    data: *mut u8,
    end: usize,
    capacity: usize,
    layout: Layout,
    // Type Erased Methods //
    pub(crate) finalize: fn(&mut ErasedTable),
    /// remove is always swap_remove
    pub(crate) remove: fn(RowIndex, &mut ErasedTable),
    #[cfg(feature = "clone")]
    pub(crate) clone: fn(&ErasedTable) -> ErasedTable,
    pub(crate) clone_empty: fn() -> ErasedTable,
    /// src, dst
    ///
    /// if component is not in `src` then this is a noop
    /// Caller must ensure that both tables have the same underlying type
    pub(crate) move_row: fn(&mut ErasedTable, &mut ErasedTable, RowIndex),
    /// src, dst
    /// Move the row from src to the specified slow in dst
    ///
    /// Caller must ensure that dst is initialized and both tables have the same underlying type
    pub(crate) move_row_into: fn(&mut ErasedTable, RowIndex, &mut ErasedTable, RowIndex),
    /// Swap rows in an entity
    pub(crate) swap_rows: fn(&mut ErasedTable, RowIndex, RowIndex),
}

impl Default for ErasedTable {
    fn default() -> Self {
        Self::new::<()>(0)
    }
}

impl Drop for ErasedTable {
    fn drop(&mut self) {
        (self.finalize)(self);
    }
}

#[cfg(feature = "clone")]
impl Clone for ErasedTable {
    fn clone(&self) -> Self {
        (self.clone)(self)
    }
}

impl std::fmt::Debug for ErasedTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ErasedVec")
            .field("ty", &self.ty_name)
            .finish()
    }
}

impl ErasedTable {
    pub fn new<T: crate::Component>(capacity: usize) -> Self {
        let layout = Self::layout::<T>(capacity);
        Self {
            ty_name: std::any::type_name::<T>(),
            capacity,
            end: 0,
            data: unsafe { std::alloc::alloc(layout) },
            layout,
            finalize: |erased_table: &mut ErasedTable| {
                // drop the inner table
                unsafe {
                    let data: *mut T = erased_table.data.cast();
                    for i in 0..erased_table.end {
                        std::ptr::drop_in_place(data.add(i));
                    }
                    std::alloc::dealloc(erased_table.data, erased_table.layout);
                }
            },
            remove: |entity_id, erased_table: &mut ErasedTable| unsafe {
                erased_table.swap_remove::<T>(entity_id as usize);
            },
            #[cfg(feature = "clone")]
            clone: |table: &ErasedTable| {
                let mut res = ErasedTable::new::<T>(table.capacity);
                res.end = table.end;
                for i in 0..table.end {
                    unsafe {
                        let val = (&*table.data.cast::<T>().add(i)).clone();
                        std::ptr::write(res.data.cast::<T>().add(i), val);
                    }
                }
                res
            },
            clone_empty: || ErasedTable::new::<T>(1),
            move_row: |src, dst, index| unsafe {
                let src = src.swap_remove::<T>(index as usize);
                dst.push::<T>(src);
            },
            move_row_into: |src_t, src, dst_t, dst| unsafe {
                let src = src_t.swap_remove::<T>(src as usize);
                dst_t.as_slice_mut::<T>()[dst as usize] = src;
            },
            swap_rows: |this, src, dst| unsafe {
                this.as_slice_mut::<T>().swap(src as usize, dst as usize);
            },
        }
    }

    /// # SAFETY
    /// Must be called with the same type as `new`
    pub unsafe fn as_slice<T>(&self) -> &[T] {
        std::slice::from_raw_parts(self.data.cast::<T>(), self.end)
    }

    /// # SAFETY
    /// Must be called with the same type as `new`
    pub unsafe fn as_slice_mut<T>(&mut self) -> &mut [T] {
        std::slice::from_raw_parts_mut(self.data.cast::<T>(), self.end)
    }

    fn layout<T>(capacity: usize) -> Layout {
        let layout = Layout::array::<T>(capacity).unwrap();
        // ensure non-zero layout
        if layout.size() != 0 {
            layout
        } else {
            Layout::from_size_align(1, 1).unwrap()
        }
    }

    /// # SAFETY
    /// Must be called with the same type as `new`
    pub unsafe fn push<T>(&mut self, val: T) {
        debug_assert!(self.end <= self.capacity);
        if self.end == self.capacity {
            // full, have to reallocate
            let new_cap = (self.capacity * 2).max(2);
            let new_layout = Self::layout::<T>(new_cap);
            let new_data = std::alloc::alloc(new_layout);
            for i in 0..self.end {
                let t: T = std::ptr::read(self.data.cast::<T>().add(i));
                std::ptr::write(new_data.cast::<T>().add(i), t);
            }
            std::alloc::dealloc(self.data, self.layout);
            self.capacity = new_cap;
            self.data = new_data;
            self.layout = new_layout;
        }
        std::ptr::write(self.data.cast::<T>().add(self.end), val);
        self.end += 1;
    }

    /// # SAFETY
    /// Must be called with the same type as `new`
    pub unsafe fn swap_remove<T>(&mut self, i: usize) -> T {
        debug_assert!(i < self.end);
        let res;
        if i + 1 == self.end {
            // last item
            res = std::ptr::read(self.data.cast::<T>().add(i));
        } else {
            res = std::ptr::read(self.data.cast::<T>().add(i));
            let last: T = std::ptr::read(self.data.cast::<T>().add(self.end - 1));
            std::ptr::write(self.data.cast::<T>().add(i), last);
        }
        self.end -= 1;
        res
    }

    pub fn remove(&mut self, id: RowIndex) {
        (self.remove)(id, self);
    }
}
