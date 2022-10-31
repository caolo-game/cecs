use std::{alloc::Layout, any::TypeId, cell::UnsafeCell, collections::BTreeMap};

// TODO: use dense storage instead of the Vec because of archetypes
use crate::{entity_id::EntityId, hash_ty, Component, RowIndex, TypeHash};

// TODO: hide from public interface, because it's fairly unsafe
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
        debug_assert!(self.rows > 0);
        debug_assert!(index < self.rows);
        self.rows -= 1;
        let entity_id = self.entities.swap_remove(index as usize);
        let mut moved = None;
        if self.rows > 0 && index < self.rows {
            // if we have remaining rows, and the removed row was not the last
            moved = Some(self.entities[index as usize]);
        }
        let res = dst.insert_entity(entity_id);
        for (ty, col) in self.components.iter_mut() {
            if let Some(dst) = dst.components.get_mut(ty) {
                (col.get_mut().move_row)(col.get_mut(), dst.get_mut(), index);
            } else {
                // destination does not have this column
                col.get_mut().remove(index);
            }
        }
        (res, moved)
    }

    pub fn set_component<T: 'static>(&mut self, row_index: RowIndex, val: T) {
        unsafe {
            let table = self
                .components
                .get_mut(&TypeId::of::<T>())
                .expect("set_component called on bad archetype")
                .get_mut();

            let v = table.as_inner_mut();
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
        self.components.contains_key(&hash)
    }

    pub fn extended_hash<T: Component>(&self) -> TypeHash {
        self.ty ^ hash_ty::<T>()
    }

    pub fn extend_with_column<T: Component>(&self) -> Self {
        assert!(!self.contains_column::<T>());

        let mut result = self.clone_empty();
        let new_ty = self.extended_hash::<T>();
        result.ty = new_ty;
        result
            .components
            .insert(TypeId::of::<T>(), UnsafeCell::new(ErasedTable::new::<T>(0)));
        result
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
            .and_then(|columns| unsafe { (*columns.get()).as_inner().get(row as usize) })
    }

    pub fn get_component_mut<T: 'static>(&self, row: RowIndex) -> Option<&mut T> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|columns| unsafe { (*columns.get()).as_inner_mut().get_mut(row as usize) })
    }
}

/// Type erased Vec
pub(crate) struct ErasedTable {
    ty_name: &'static str,
    // Vec //
    data: *mut u8,
    end: usize,
    capacity: usize,
    layout: Layout,
    // Type Erased Methods //
    finalize: fn(&mut ErasedTable),
    /// remove is always swap_remove
    remove: fn(RowIndex, &mut ErasedTable),
    #[cfg(feature = "clone")]
    clone: fn(&ErasedTable) -> ErasedTable,
    clone_empty: fn() -> ErasedTable,
    /// src, dst
    ///
    /// if component is not in `src` then this is a noop
    move_row: fn(&mut ErasedTable, &mut ErasedTable, RowIndex),
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
        let layout = Layout::array::<T>(capacity).unwrap();
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
        }
    }

    /// # SAFETY
    /// Must be called with the same type as `new`
    pub unsafe fn as_inner<T>(&self) -> &[T] {
        std::slice::from_raw_parts(self.data.cast::<T>(), self.end)
    }

    /// # SAFETY
    /// Must be called with the same type as `new`
    pub unsafe fn as_inner_mut<T>(&mut self) -> &mut [T] {
        std::slice::from_raw_parts_mut(self.data.cast::<T>(), self.end)
    }

    /// # SAFETY
    /// Must be called with the same type as `new`
    pub unsafe fn push<T>(&mut self, val: T) {
        debug_assert!(self.end <= self.capacity);
        if self.end == self.capacity {
            // full, have to reallocate
            let new_cap = (self.capacity * 2).max(2);
            let new_layout = Layout::array::<T>(new_cap).unwrap();
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
