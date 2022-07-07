#[cfg(test)]
mod db_tests;

use std::{any::TypeId, collections::HashMap, marker::PhantomData};

// TODO: use dense storage instead of the PageTable because of archetypes
use crate::{entity_id::EntityId, hash_ty, page_table::PageTable, Component, RowIndex, TypeHash};

#[derive(Clone)]
pub struct ArchetypeStorage {
    ty: TypeHash,
    rows: u32,
    entities: PageTable<EntityId>,
    components: HashMap<TypeId, ErasedPageTable>,
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
                    .map(|(row_index, id)| (id.to_string(), row_index))
                    .collect::<Vec<_>>(),
            )
            .field(
                "components",
                &self
                    .components
                    .iter()
                    .map(|(_, c)| c.ty_name)
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
            ErasedPageTable::new(PageTable::<()>::default()),
        );
        Self {
            ty,
            rows: 0,
            entities: PageTable::new(4),
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

    pub fn remove(&mut self, row_index: RowIndex) {
        for (_, storage) in self.components.iter_mut() {
            storage.remove(row_index);
        }
        self.entities.remove(row_index);
        self.rows -= 1;
    }

    pub fn insert_entity(&mut self, id: EntityId) -> RowIndex {
        let res = self.rows;
        self.entities.insert(res, id);
        self.rows += 1;
        res
    }

    /// return the new index in `dst`
    pub fn move_entity(&mut self, dst: &mut Self, index: RowIndex) -> RowIndex {
        self.rows -= 1;
        let entity_id = self.entities.remove(index).unwrap();
        let res = dst.insert_entity(entity_id);
        for (ty, col) in self.components.iter_mut() {
            if let Some(dst) = dst.components.get_mut(ty) {
                (col.move_row)(col, dst, index);
            }
        }
        res
    }

    pub fn set_component<T: 'static>(&mut self, id: EntityId, row_index: RowIndex, val: T) {
        unsafe {
            self.entities.insert(row_index, id);
            self.components
                .get_mut(&TypeId::of::<T>())
                .expect("set_component called on bad archetype")
                .as_inner_mut()
                .insert(row_index, val);
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
            ErasedPageTable::new::<T>(PageTable::default()),
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
            entities: PageTable::new(self.entities.len()),
            components: HashMap::from_iter(
                self.components
                    .iter()
                    .map(|(id, col)| (*id, (col.clone_empty)())),
            ),
        }
    }

    pub fn get_component<T: 'static>(&self, row: RowIndex) -> Option<&T> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|columns| unsafe { columns.as_inner().get(row) })
    }
}

/// Type erased PageTable
pub(crate) struct ErasedPageTable {
    ty_name: &'static str,
    inner: *mut std::ffi::c_void,
    finalize: fn(&mut ErasedPageTable),
    remove: fn(RowIndex, &mut ErasedPageTable),
    clone: fn(&ErasedPageTable) -> ErasedPageTable,
    clone_empty: fn() -> ErasedPageTable,
    /// src, dst
    ///
    /// if component is not in `src` then this is a noop
    move_row: fn(&mut ErasedPageTable, &mut ErasedPageTable, RowIndex),
}

impl Default for ErasedPageTable {
    fn default() -> Self {
        Self::new::<()>(PageTable::new(4))
    }
}

impl Drop for ErasedPageTable {
    fn drop(&mut self) {
        (self.finalize)(self);
    }
}

impl Clone for ErasedPageTable {
    fn clone(&self) -> Self {
        (self.clone)(&self)
    }
}

impl std::fmt::Debug for ErasedPageTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ErasedPageTable")
            .field("ty", &self.ty_name)
            .finish()
    }
}

impl ErasedPageTable {
    pub fn new<T: 'static + Clone>(table: PageTable<T>) -> Self {
        Self {
            ty_name: std::any::type_name::<T>(),
            inner: Box::into_raw(Box::new(table)).cast(),
            finalize: |erased_table: &mut ErasedPageTable| {
                // drop the inner table
                unsafe {
                    let _ = Box::from_raw(erased_table.inner.cast::<PageTable<T>>());
                }
            },
            remove: |entity_id, erased_table: &mut ErasedPageTable| unsafe {
                erased_table.as_inner_mut::<T>().remove(entity_id);
            },
            clone: |table: &ErasedPageTable| {
                let inner = unsafe { table.as_inner::<T>() };
                let res: PageTable<T> = inner.clone();
                ErasedPageTable::new(res)
            },
            clone_empty: || ErasedPageTable::new::<T>(PageTable::default()),
            move_row: |src, dst, entity_id| unsafe {
                let src = src.as_inner_mut::<T>();
                let dst = dst.as_inner_mut::<T>();
                if let Some(src) = src.remove(entity_id) {
                    dst.insert(entity_id, src);
                }
            },
        }
    }

    /// # SAFETY
    /// Must be called with the same type as `new`
    pub unsafe fn as_inner<T>(&self) -> &PageTable<T> {
        &*self.inner.cast()
    }

    /// # SAFETY
    /// Must be called with the same type as `new`
    pub unsafe fn as_inner_mut<T>(&mut self) -> &mut PageTable<T> {
        &mut *self.inner.cast()
    }

    pub fn remove(&mut self, id: RowIndex) {
        (self.remove)(id, self);
    }
}

#[derive(Clone, Copy)]
pub struct Ref<'a, T: 'static> {
    inner: &'static T,
    _m: PhantomData<&'a ()>,
}

impl<'a, T: 'static> AsRef<T> for Ref<'a, T> {
    fn as_ref(&self) -> &T {
        self.inner
    }
}

impl<'a, T: 'static> std::ops::Deref for Ref<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner
    }
}

pub struct QueryIt<'a, T> {
    inner: Option<Box<dyn Iterator<Item = (u32, &'a T)> + 'a>>,
    _m: PhantomData<&'a ()>,
}

impl<'a, T: 'static> Iterator for QueryIt<'a, T> {
    type Item = Ref<'a, T>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.as_mut().and_then(|it| it.next()).map(|(_, x)| {
            let x: &'static T = unsafe { std::mem::transmute(x) };
            Ref {
                inner: x,
                _m: PhantomData,
            }
        })
    }
}

pub trait Queryable<'a, T: 'static> {
    type It: Iterator<Item = Ref<'a, T>>;

    fn iter(&'a self) -> Self::It;
}

impl<'a, T: Component> Queryable<'a, T> for ArchetypeStorage {
    type It = QueryIt<'a, T>;

    fn iter(&'a self) -> Self::It {
        let inner = self
            .components
            .get(&TypeId::of::<T>())
            .map(|columns| unsafe { columns.as_inner::<T>().iter() });
        let inner = inner.map(|fos| {
            let res: Box<dyn Iterator<Item = (u32, &'a T)>> = Box::new(fos);
            res
        });
        QueryIt {
            inner,
            _m: PhantomData,
        }
    }
}

#[derive(Default)]
pub struct ComponentQuery<T> {
    _m: PhantomData<T>,
}

impl<'a, T: 'static> ComponentQuery<T>
where
    ArchetypeStorage: Queryable<'a, T>,
{
    pub fn iter(
        &self,
        archetype: &'a ArchetypeStorage,
    ) -> <ArchetypeStorage as Queryable<'a, T>>::It {
        archetype.iter()
    }
}
