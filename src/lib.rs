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

pub trait Component: 'static + Clone {}
impl<T: 'static + Clone> Component for T {}

impl World {
    pub fn new(capacity: u32) -> Self {
        let entity_ids = HandleTable::new(capacity);

        let mut archetypes = HashMap::with_capacity(128);
        let void_store = ArchetypeStorage::default();
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
        void_store.set_component(id, ());
        self.entity_archetype.insert(id, VOID_TY);
        Ok(id)
    }

    pub fn delete_entity(&mut self, id: EntityId) -> WorldResult<()> {
        if !self.entity_ids.is_valid(id) {
            todo!()
        }
        let ty = self.entity_archetype[&id];
        let archetype = self.archetypes.get_mut(&ty).unwrap();
        archetype.remove(id);
        self.entity_ids.free(id);
        Ok(())
    }

    pub fn set_component<T: Component>(
        &mut self,
        entity_id: EntityId,
        component: T,
    ) -> WorldResult<()> {
        if !self.entity_ids.is_valid(entity_id) {
            return Err(WorldError::EntityNotFound);
        }
        let mut archetype_hash = self.entity_archetype[&entity_id];
        let archetype = self.archetypes.get_mut(&archetype_hash).unwrap();
        if !archetype.contains_column::<T>() {
            let new_ty = archetype.extended_hash::<T>();
            if !self.archetypes.contains_key(&new_ty) {
                self.insert_archetype::<T>(entity_id, archetype_hash)
            }
            archetype_hash = new_ty;
        }
        let archetype = self.archetypes.get_mut(&archetype_hash).unwrap();
        archetype.set_component(entity_id, component);
        Ok(())
    }

    #[inline(never)]
    fn insert_archetype<T: Component>(&mut self, entity_id: EntityId, archetype_hash: TypeHash) {
        let archetype = self.archetypes.get_mut(&archetype_hash).unwrap();
        let mut new_arch = archetype.extend_with_column::<T>();
        // move all components from old archetype to new
        for (ty, col) in archetype.components.iter_mut() {
            let dst = new_arch.components.get_mut(ty).unwrap();
            (col.move_row)(col, dst, entity_id);
        }
        let ty = new_arch.ty();
        self.entity_archetype.insert(entity_id, ty);
        self.archetypes.insert(ty, new_arch);
    }
}

#[derive(Clone)]
pub(crate) struct ArchetypeStorage {
    ty: TypeHash,
    components: HashMap<TypeHash, ErasedPageTable>,
}

impl Default for ArchetypeStorage {
    fn default() -> Self {
        let ty = hash_ty::<()>();
        let mut components = HashMap::new();
        components.insert(ty, ErasedPageTable::new(PageTable::<()>::default()));
        Self { ty, components }
    }
}

impl ArchetypeStorage {
    /// Get the archetype storage's ty.
    pub fn ty(&self) -> TypeHash {
        self.ty
    }

    pub fn remove(&mut self, id: EntityId) {
        for (_, storage) in self.components.iter_mut() {
            storage.remove(id);
        }
    }

    pub fn set_component<T: 'static>(&mut self, id: EntityId, val: T) {
        unsafe {
            self.components
                .get_mut(&hash_ty::<T>())
                .expect("set_component called on bad archetype")
                .as_inner_mut()
                .insert(id, val);
        }
    }

    pub fn contains_column<T: 'static>(&self) -> bool {
        let hash = hash_ty::<T>();
        self.components.contains_key(&hash)
    }

    pub fn extended_hash<T: 'static + Clone>(&self) -> TypeHash {
        if self.contains_column::<T>() {
            self.ty
        } else {
            self.ty ^ hash_ty::<T>()
        }
    }

    pub fn extend_with_column<T: 'static + Clone>(&self) -> Self {
        assert!(!self.contains_column::<T>());

        let mut result = self.clone_empty();
        let new_ty = self.extended_hash::<T>();
        result.ty = new_ty;
        result.components.insert(
            hash_ty::<T>(),
            ErasedPageTable::new::<T>(PageTable::default()),
        );
        result
    }

    pub fn clone_empty(&self) -> Self {
        Self {
            ty: self.ty,
            components: HashMap::from_iter(
                self.components
                    .iter()
                    .map(|(id, col)| (*id, (col.clone_empty)())),
            ),
        }
    }
}

/// Type erased PageTable
pub(crate) struct ErasedPageTable {
    ty: TypeHash,
    inner: *mut std::ffi::c_void,
    finalize: fn(&mut ErasedPageTable),
    remove: fn(EntityId, &mut ErasedPageTable),
    clone: fn(&ErasedPageTable) -> ErasedPageTable,
    clone_empty: fn() -> ErasedPageTable,
    /// src, dst
    ///
    /// if component is not in `src` then this is a noop
    move_row: fn(&mut ErasedPageTable, &mut ErasedPageTable, EntityId),
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

    /// # SAFETY
    /// Must be called with the same type as `new`
    pub unsafe fn into_inner<T>(self) -> PageTable<T> {
        std::ptr::read(self.inner.cast())
    }

    pub fn remove(&mut self, id: EntityId) {
        (self.remove)(id, self);
    }
}
