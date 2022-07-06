#![feature(option_get_or_insert_default)]
#![feature(const_type_id)]

use std::{any::TypeId, collections::HashMap};

use entity_id::EntityId;
use page_table::PageTable;

use crate::handle_table::HandleTable;

pub mod entity_id;
pub mod handle_table;
pub mod page_table;

#[cfg(test)]
mod tests;

pub struct World {
    entity_ids: HandleTable,
    entity_archetype: HashMap<EntityId, TypeHash>,
    archetypes: HashMap<TypeHash, ArchetypeStorage>,
}

type TypeHash = u64;

const fn hash_ty<T: 'static>() -> u64 {
    let ty = TypeId::of::<T>();
    // FIXME extreme curse
    unsafe { std::mem::transmute(ty) }
}

const VOID_TY: TypeHash = hash_ty::<()>();

#[derive(Clone, Debug, thiserror::Error)]
pub enum WorldError {
    #[error("World is full and can not take more entities")]
    OutOfCapacity,
    #[error("Entity was not found")]
    EntityNotFound,
}

pub type WorldResult<T> = Result<T, WorldError>;

impl World {
    pub fn new(capacity: u32) -> Self {
        let entity_ids = HandleTable::new(capacity);

        let mut archetypes = HashMap::with_capacity(128);
        let void_store = ArchetypeStorage::new::<()>(128);
        archetypes.insert(void_store.ty(), void_store);

        // FIXME: can't add assert to const fn...
        debug_assert_eq!(std::mem::size_of::<TypeId>(), std::mem::size_of::<u64>());
        Self {
            entity_ids,
            entity_archetype: Default::default(),
            archetypes,
        }
    }

    pub fn insert_entity(&mut self) -> WorldResult<EntityId> {
        let id = self
            .entity_ids
            .alloc()
            .map_err(|_| WorldError::OutOfCapacity)?;
        let void_store = self.archetypes.get_mut(&VOID_TY).unwrap();
        void_store.components_mut::<()>().insert(id, ());
        self.entity_archetype.insert(id, VOID_TY);
        Ok(id)
    }

    pub fn delete_entity(&mut self, id: EntityId) -> WorldResult<()> {
        if !self.entity_ids.is_valid(id) {
            todo!()
        }
        let ty = self.entity_archetype[&id];
        let archetype = self.archetypes.get_mut(&ty).unwrap();
        archetype.delete(id);
        self.entity_ids.free(id);
        Ok(())
    }

    pub fn set_component<T>(&mut self, entity_id: EntityId, component: T) -> WorldResult<()> {
        if !self.entity_ids.is_valid(entity_id) {
            return Err(WorldError::EntityNotFound);
        }
        let archetype_hash = self.entity_archetype[&entity_id];
        let archetype = &self.archetypes[&archetype_hash];

        let old_hash = archetype.ty();

        // let have_already =
        todo!()
    }
}

#[derive(Clone)]
pub struct ArchetypeStorage {
    ty: TypeHash,
    components: HashMap<TypeId, ErasedPageTable>,
}

impl ArchetypeStorage {
    pub fn new<T: 'static + Clone>(capacity: usize) -> Self {
        let ty = hash_ty::<T>();
        Self {
            ty,
            components: HashMap::new(),
        }
    }

    /// Get the archetype storage's ty.
    pub fn ty(&self) -> TypeHash {
        self.ty
    }

    pub fn remove(&mut self, id: EntityId) {
        for (_, storage) in self.components.iter_mut() {
            storage.remove(id);
        }
    }
}

/// Type erased PageTable
pub struct ErasedPageTable {
    ty: TypeHash,
    inner: *mut std::ffi::c_void,
    finalize: fn(&mut ErasedPageTable),
    remove: fn(EntityId, &mut ErasedPageTable),
    clone: fn(&ErasedPageTable) -> ErasedPageTable,
}

impl Default for ErasedPageTable {
    fn default() -> Self {
        Self::new::<()>(PageTable::new(4))
    }
}

impl Drop for ErasedPageTable {
    fn drop(&mut self) {
        (self.finalize)(&mut self);
    }
}

impl Clone for ErasedPageTable {
    fn clone(&self) -> Self {
        (self.clone)(&self)
    }
}

impl ErasedPageTable {
    /// Get the archetype storage's ty.
    pub fn ty(&self) -> TypeHash {
        self.ty
    }

    pub fn new<T: 'static + Clone>(table: PageTable<T>) -> Self {
        Self {
            ty: hash_ty::<T>(),
            inner: Box::into_raw(Box::new(table)).cast(),
            finalize: |erased_table: &mut ErasedPageTable| {
                // drop the inner table
                unsafe {
                    std::ptr::drop_in_place(erased_table.as_inner_mut::<T>());
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
    pub unsafe fn into_inner<T>(self) -> PageTable<T> {
        std::ptr::read(self.inner.cast())
    }

    pub fn remove(&mut self, id: EntityId) {
        (self.remove)(id, &mut self);
    }
}
