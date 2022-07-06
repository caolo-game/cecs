#![feature(option_get_or_insert_default)]
#![feature(const_type_id)]

use std::{any::TypeId, collections::HashMap};

use entity_id::EntityId;
use page_table::PageTable;

use crate::handle_table::HandleTable;

pub mod entity_id;
pub mod handle_table;
pub mod page_table;

pub struct World {
    entity_ids: HandleTable,
    archetypes: HashMap<TypeId, ArchetypeStorage>,
}

const VOID_TY: TypeId = TypeId::of::<()>();

impl World {
    pub fn new(capacity: u32) -> Self {
        let entity_ids = HandleTable::new(capacity);

        let mut archetypes = HashMap::with_capacity(128);
        let void_store = ArchetypeStorage::new::<()>(128);
        archetypes.insert(void_store.ty(), void_store);

        Self {
            entity_ids,
            archetypes,
        }
    }

    // TODO: errors
    pub fn insert_entity(&mut self) -> Result<EntityId, ()> {
        let id = self.entity_ids.alloc().unwrap(); // FIXME: return error
        let void_store = self.archetypes.get_mut(&VOID_TY).unwrap();
        void_store.components_mut::<()>().insert(id, ());
        Ok(id)
    }
}

pub struct ArchetypeStorage {
    ty: TypeId,
    components: ErasedPageTable,
    finalizer: Box<dyn Fn(&mut ErasedPageTable)>,
}

impl Drop for ArchetypeStorage {
    fn drop(&mut self) {
        (self.finalizer)(&mut self.components);
    }
}

impl ArchetypeStorage {
    pub fn new<T: 'static>(capacity: usize) -> Self {
        let ty = TypeId::of::<T>();
        Self {
            ty,
            components: ErasedPageTable::new(PageTable::<T>::new(capacity)),
            finalizer: Box::new(|erased_table: &mut ErasedPageTable| {
                // drop the inner table
                unsafe {
                    std::ptr::drop_in_place(erased_table);
                }
            }),
        }
    }

    pub fn components<T: 'static>(&self) -> &PageTable<T> {
        assert_eq!(self.ty, TypeId::of::<T>());
        unsafe { self.components.as_inner() }
    }

    pub fn components_mut<T: 'static>(&mut self) -> &mut PageTable<T> {
        assert_eq!(self.ty, TypeId::of::<T>());
        unsafe { self.components.as_inner_mut() }
    }

    /// Get the archetype storage's ty.
    pub fn ty(&self) -> TypeId {
        self.ty
    }
}

/// Type erased PageTable
pub struct ErasedPageTable {
    ty: TypeId,
    inner: *mut std::ffi::c_void,
}

impl Default for ErasedPageTable {
    fn default() -> Self {
        Self::new::<()>(PageTable::new(4))
    }
}

impl Drop for ErasedPageTable {
    fn drop(&mut self) {
        assert!(
            self.inner.is_null(),
            "You have to clean up the ErasedPageTable by casting to the underlying value"
        );
    }
}

impl ErasedPageTable {
    /// Get the archetype storage's ty.
    pub fn ty(&self) -> TypeId {
        self.ty
    }

    pub fn new<T: 'static>(table: PageTable<T>) -> Self {
        Self {
            ty: TypeId::of::<T>(),
            inner: Box::into_raw(Box::new(table)).cast(),
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

    /// # SAFETY
    /// Must be called with the same type as `new`
    pub unsafe fn into_inner<T>(mut self) -> PageTable<T> {
        let result = std::ptr::read(self.inner.cast());
        self.inner = std::ptr::null_mut();
        result
    }
}
