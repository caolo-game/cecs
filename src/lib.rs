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
mod world_tests;

pub struct World {
    // TODO: world can be generic over Index
    entity_ids: Pin<Box<EntityIndex>>,
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
    #[error("Entity doesn't have specified component")]
    ComponentNotFound,
}

pub type WorldResult<T> = Result<T, WorldError>;
pub type RowIndex = u32;

/// The end goal is to have a clonable ECS, that's why we have the Clone restriction.
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
        let entity_ids = Box::pin(EntityIndex::new(capacity));

        let mut archetypes = HashMap::with_capacity(128);
        let void_store = Box::pin(ArchetypeStorage::empty());
        archetypes.insert(VOID_TY, void_store);

        // FIXME: can't add assert to const fn...
        // the `hash_ty` function assumes that TypeId is a u64 under the hood
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
                let mut res = self.insert_archetype::<T>(
                    archetype,
                    index,
                    archetype.extend_with_column::<T>(),
                );
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

    pub fn get_component<T: Component>(&self, entity_id: EntityId) -> Option<&T> {
        let (arch, idx) = self.entity_ids.read(entity_id).ok()?;
        unsafe { arch.as_ref().get_component(idx) }
    }

    pub fn remove_component<T: Component>(&mut self, entity_id: EntityId) -> WorldResult<()> {
        let (mut archetype, mut index) = self
            .entity_ids
            .read(entity_id)
            .map_err(|_| WorldError::EntityNotFound)?;
        let mut archetype = unsafe { archetype.as_mut() };
        if !archetype.contains_column::<T>() {
            return Err(WorldError::ComponentNotFound);
        }
        let new_ty = archetype.extended_hash::<T>();
        if !self.archetypes.contains_key(&new_ty) {
            let mut res =
                self.insert_archetype::<T>(archetype, index, archetype.reduce_with_column::<T>());
            archetype = unsafe { res.as_mut() };
            index = 0;
        } else {
            let new_arch = self.archetypes.get_mut(&new_ty).unwrap();
            index = archetype.move_entity(new_arch, index);
            archetype = new_arch.as_mut().get_mut();
        }
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
        archetype: &mut ArchetypeStorage,
        row_index: RowIndex,
        new_arch: ArchetypeStorage,
    ) -> NonNull<ArchetypeStorage> {
        let mut new_arch = Box::pin(new_arch);
        let index = archetype.move_entity(&mut new_arch, row_index);
        debug_assert_eq!(index, 0);
        let res = unsafe { NonNull::new_unchecked(new_arch.as_mut().get_mut() as *mut _) };
        self.archetypes.insert(new_arch.ty(), new_arch);
        res
    }
}

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
            if let Some(entry) = self.entities.get_mut(row_index) {
                *entry = id;
            } else {
                self.entities.insert(row_index, id);
            }
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
