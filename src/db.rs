use std::{any::TypeId, cell::UnsafeCell, collections::HashMap};

// TODO: use dense storage instead of the Vec because of archetypes
use crate::{entity_id::EntityId, hash_ty, RowIndex, TypeHash};

// TODO: hide from public interface, because it's fairly unsafe
pub struct ArchetypeStorage {
    pub(crate) ty: TypeHash,
    pub(crate) rows: u32,
    pub(crate) entities: Vec<EntityId>,
    pub(crate) components: HashMap<TypeId, UnsafeCell<ErasedTable>>,
}

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
        let mut components = HashMap::new();
        components.insert(
            TypeId::of::<()>(),
            UnsafeCell::new(ErasedTable::new(Vec::<()>::default())),
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
            let v = self
                .components
                .get_mut(&TypeId::of::<T>())
                .expect("set_component called on bad archetype")
                .get_mut()
                .as_inner_mut();
            let row_index = row_index as usize;
            assert!(row_index <= v.len());
            if row_index == v.len() {
                v.push(val);
            } else {
                v[row_index as usize] = val;
            }
        }
    }

    pub fn contains_column<T: 'static>(&self) -> bool {
        let hash = TypeId::of::<T>();
        self.components.contains_key(&hash)
    }

    pub fn extended_hash<T: 'static + Clone>(&self) -> TypeHash {
        self.ty ^ hash_ty::<T>()
    }

    pub fn extend_with_column<T: 'static + Clone>(&self) -> Self {
        assert!(!self.contains_column::<T>());

        let mut result = self.clone_empty();
        let new_ty = self.extended_hash::<T>();
        result.ty = new_ty;
        result.components.insert(
            TypeId::of::<T>(),
            UnsafeCell::new(ErasedTable::new::<T>(Vec::default())),
        );
        result
    }

    pub fn reduce_with_column<T: 'static + Clone>(&self) -> Self {
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
            components: HashMap::from_iter(
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
            .and_then(|columns| unsafe { (&*columns.get()).as_inner().get(row as usize) })
    }
}

/// Type erased Vec
pub(crate) struct ErasedTable {
    ty_name: &'static str,
    inner: *mut std::ffi::c_void,
    finalize: fn(&mut ErasedTable),
    /// remove is always swap_remove
    remove: fn(RowIndex, &mut ErasedTable),
    clone: fn(&ErasedTable) -> ErasedTable,
    clone_empty: fn() -> ErasedTable,
    /// src, dst
    ///
    /// if component is not in `src` then this is a noop
    move_row: fn(&mut ErasedTable, &mut ErasedTable, RowIndex),
}

impl Default for ErasedTable {
    fn default() -> Self {
        Self::new::<()>(Vec::new())
    }
}

impl Drop for ErasedTable {
    fn drop(&mut self) {
        (self.finalize)(self);
    }
}

impl Clone for ErasedTable {
    fn clone(&self) -> Self {
        (self.clone)(&self)
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
    pub fn new<T: 'static + Clone>(table: Vec<T>) -> Self {
        Self {
            ty_name: std::any::type_name::<T>(),
            inner: Box::into_raw(Box::new(table)).cast(),
            finalize: |erased_table: &mut ErasedTable| {
                // drop the inner table
                unsafe {
                    let _ = Box::from_raw(erased_table.inner.cast::<Vec<T>>());
                }
            },
            remove: |entity_id, erased_table: &mut ErasedTable| unsafe {
                let v = erased_table.as_inner_mut::<T>();
                v.swap_remove(entity_id as usize);
            },
            clone: |table: &ErasedTable| {
                let inner = unsafe { table.as_inner::<T>() };
                let res: Vec<T> = inner.clone();
                ErasedTable::new(res)
            },
            clone_empty: || ErasedTable::new::<T>(Vec::default()),
            move_row: |src, dst, index| unsafe {
                let src = src.as_inner_mut::<T>();
                let dst = dst.as_inner_mut::<T>();
                let src = src.swap_remove(index as usize);
                dst.push(src);
            },
        }
    }

    /// # SAFETY
    /// Must be called with the same type as `new`
    pub unsafe fn as_inner<T>(&self) -> &Vec<T> {
        &*self.inner.cast()
    }

    /// # SAFETY
    /// Must be called with the same type as `new`
    pub unsafe fn as_inner_mut<T>(&mut self) -> &mut Vec<T> {
        &mut *self.inner.cast()
    }

    pub fn remove(&mut self, id: RowIndex) {
        (self.remove)(id, self);
    }
}
