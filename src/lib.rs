#![feature(option_get_or_insert_default)]
#![feature(const_type_id)]

use std::{any::TypeId, collections::HashMap, pin::Pin, ptr::NonNull};

use entity_id::EntityId;
use handle_table::EntityIndex;
// TODO: use dense storage instead of the PageTable because of archetypes
use page_table::PageTable;

pub mod entity_id;
pub mod handle_table;
pub mod page_table;

#[cfg(test)]
mod tests;

pub struct World {
    // TODO: world can be generic over Index
    entity_ids: EntityIndex,
    archetypes: HashMap<TypeHash, Pin<Box<ArchetypeStorage>>>,
}

type TypeHash = u64;

const fn hash_ty<T: 'static>() -> u64 {
    let ty = TypeId::of::<T>();
    // FIXME extreme curse
    //
    let ty: u64 = unsafe { std::mem::transmute(ty) };
    if ty == unsafe { std::mem::transmute(TypeId::of::<()>()) } {
        // ensure that unit type has hash=0
        0
    } else {
        ty
    }
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
pub type RowIndex = u32;

pub trait Component: 'static + Clone {}
impl<T: 'static + Clone> Component for T {}

pub trait Index {
    type Id;
    type Error;

    fn allocate(&mut self) -> Result<Self::Id, Self::Error>;
    fn update(
        &mut self,
        id: Self::Id,
        payload: (NonNull<ArchetypeStorage>, RowIndex),
    ) -> Result<(), Self::Error>;
    fn delete(&mut self, id: Self::Id) -> Result<(), Self::Error>;
    fn read(&self, id: Self::Id) -> Result<(NonNull<ArchetypeStorage>, RowIndex), Self::Error>;
}

impl World {
    pub fn new(capacity: u32) -> Self {
        let entity_ids = EntityIndex::new(capacity);

        let mut archetypes = HashMap::with_capacity(128);
        let void_store = Box::pin(ArchetypeStorage::default());
        archetypes.insert(VOID_TY, void_store);

        // FIXME: can't add assert to const fn...
        debug_assert_eq!(std::mem::size_of::<TypeId>(), std::mem::size_of::<u64>());
        Self {
            entity_ids,
            archetypes,
        }
    }

    pub fn insert_entity(&mut self) -> WorldResult<EntityId> {
        let id = self
            .entity_ids
            .allocate()
            .map_err(|_| WorldError::OutOfCapacity)?;
        let void_store = self.archetypes.get_mut(&VOID_TY).unwrap();

        let index = void_store.as_mut().insert_entity(id);
        self.entity_ids
            .update(
                id,
                (
                    NonNull::new(void_store.as_mut().get_mut() as *mut _).unwrap(),
                    index,
                ),
            )
            .unwrap();
        Ok(id)
    }

    pub fn delete_entity(&mut self, id: EntityId) -> WorldResult<()> {
        let (mut archetype, index) = self
            .entity_ids
            .read(id)
            .map_err(|_| WorldError::EntityNotFound)?;
        unsafe {
            archetype.as_mut().remove(index);
            self.entity_ids.delete(id).unwrap();
        }
        Ok(())
    }

    pub fn set_component<T: Component>(
        &mut self,
        entity_id: EntityId,
        component: T,
    ) -> WorldResult<()> {
        let (mut archetype, mut index) = self
            .entity_ids
            .read(entity_id)
            .map_err(|_| WorldError::EntityNotFound)?;
        let mut archetype = unsafe { archetype.as_mut() };
        if !archetype.contains_column::<T>() {
            let new_ty = archetype.extended_hash::<T>();
            if !self.archetypes.contains_key(&new_ty) {
                let mut res = self.insert_archetype::<T>(entity_id, archetype, index);
                archetype = unsafe { res.as_mut() };
                index = 0;
            } else {
                let new_arch = self.archetypes.get_mut(&new_ty).unwrap();
                index = archetype.move_entity(new_arch, index);
                archetype = new_arch.as_mut().get_mut();
            }
        }
        archetype.set_component(entity_id, index, component);
        unsafe {
            self.entity_ids
                .update(
                    entity_id,
                    (NonNull::new_unchecked(archetype as *mut _), index),
                )
                .unwrap();
        }
        Ok(())
    }

    #[inline(never)]
    fn insert_archetype<T: Component>(
        &mut self,
        entity_id: EntityId,
        archetype: &mut ArchetypeStorage,
        row_index: RowIndex,
    ) -> NonNull<ArchetypeStorage> {
        let mut new_arch = Box::pin(archetype.extend_with_column::<T>());
        archetype.move_entity(&mut new_arch, row_index);
        let res = unsafe { NonNull::new_unchecked(new_arch.as_mut().get_mut() as *mut _) };
        self.entity_ids.update(entity_id, (res, 0)).unwrap();
        self.archetypes.insert(new_arch.ty(), new_arch);
        res
    }
}

#[derive(Debug, Clone)]
pub struct ArchetypeStorage {
    ty: TypeHash,
    rows: u32,
    entities: Vec<EntityId>,
    components: HashMap<TypeId, ErasedPageTable>,
}

impl Default for ArchetypeStorage {
    fn default() -> Self {
        let ty = hash_ty::<()>();
        let mut components = HashMap::new();
        components.insert(
            TypeId::of::<()>(),
            ErasedPageTable::new(PageTable::<()>::default()),
        );
        Self {
            ty,
            rows: 0,
            entities: Vec::new(),
            components,
        }
    }
}

impl ArchetypeStorage {
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
    }

    pub fn insert_entity(&mut self, id: EntityId) -> RowIndex {
        let res = self.rows;
        self.rows += 1;
        self.entities.push(id);
        res
    }

    /// return the new index in `dst`
    pub fn move_entity(&mut self, dst: &mut Self, index: RowIndex) -> RowIndex {
        let entity_id = if self.entities.len() > 1 {
            self.entities.swap_remove(index as usize)
        } else {
            self.entities.remove(index as usize)
        };
        let res = dst.insert_entity(entity_id);
        for (ty, col) in self.components.iter_mut() {
            let dst = dst.components.get_mut(ty).unwrap();
            (col.move_row)(col, dst, index);
        }
        self.rows -= 1;
        res
    }

    pub fn set_component<T: 'static>(&mut self, id: EntityId, row_index: RowIndex, val: T) {
        unsafe {
            if self.entities.len() <= row_index as usize {
                self.entities
                    .resize_with(row_index as usize + 1, EntityId::default);
            }
            self.entities[row_index as usize] = id;
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
            TypeId::of::<T>(),
            ErasedPageTable::new::<T>(PageTable::default()),
        );
        result
    }

    pub fn clone_empty(&self) -> Self {
        Self {
            ty: self.ty,
            rows: 0,
            entities: Vec::new(),
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

    pub fn remove(&mut self, id: RowIndex) {
        (self.remove)(id, self);
    }
}
