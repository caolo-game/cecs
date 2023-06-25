use std::{
    alloc::{alloc, dealloc, Layout},
    mem::{align_of, size_of},
    ptr::{self, NonNull},
};

use crate::{
    entity_id::{EntityId, ENTITY_GEN_MASK, ENTITY_INDEX_MASK},
    ArchetypeStorage, RowIndex,
};

#[derive(Debug, Clone, thiserror::Error)]
pub enum HandleTableError {
    #[error("EntityIndex has no free capacity")]
    OutOfCapacity,
    #[error("Entity not found")]
    NotFound,
    #[error("Handle was not initialized")]
    Uninitialized,
}

pub struct EntityIndex {
    entries: *mut Entry,
    cap: u32,
    /// Currently allocated entries
    count: u32,
    /// deallocated entities
    /// if empty allocate the next entity in the list
    free_list: Vec<u32>,
}

#[cfg(feature = "clone")]
impl Clone for EntityIndex {
    fn clone(&self) -> Self {
        let mut result = Self::new(self.cap);
        result.entries_mut().copy_from_slice(self.entries());
        result.free_list = self.free_list.clone();
        result.count = self.count;
        result
    }
}

unsafe impl Send for EntityIndex {}
unsafe impl Sync for EntityIndex {}

const SENTINEL: u32 = !0;

impl EntityIndex {

    pub fn new(initial_capacity: u32) -> Self {
        assert!(initial_capacity < ENTITY_INDEX_MASK);
        let entries;
        let cap = initial_capacity.max(1); // allocate at least 1 entry
        unsafe {
            entries = alloc(Layout::from_size_align_unchecked(
                size_of::<Entry>() * cap as usize,
                align_of::<Entry>(),
            )) as *mut Entry;
            assert!(!entries.is_null());
            for i in 0..cap {
                ptr::write(
                    entries.add(i as usize),
                    Entry {
                        gen: 0,
                        arch: std::ptr::null_mut(),
                        row_index: SENTINEL,
                    },
                );
            }
        };
        Self {
            entries,
            cap,
            free_list: Vec::with_capacity(cap as usize),
            count: 0,
        }
    }

    fn grow(&mut self, new_cap: u32) {
        let cap = self.cap;
        assert!(new_cap > cap);
        assert!(new_cap >= 2);
        let new_entries: *mut Entry;
        unsafe {
            new_entries = alloc(Layout::from_size_align_unchecked(
                size_of::<Entry>() * new_cap as usize,
                align_of::<Entry>(),
            ))
            .cast();

            ptr::copy_nonoverlapping(self.entries, new_entries, cap as usize);
            for i in cap..new_cap {
                ptr::write(
                    new_entries.add(i as usize),
                    Entry {
                        gen: 0,
                        arch: std::ptr::null_mut(),
                        row_index: SENTINEL,
                    },
                );
            }

            dealloc(
                self.entries.cast(),
                Layout::from_size_align_unchecked(
                    size_of::<Entry>() * cap as usize,
                    align_of::<Entry>(),
                ),
            );
        }
        self.entries = new_entries;
        self.cap = new_cap;
    }

    pub fn len(&self) -> usize {
        self.count as usize
    }

    pub fn capacity(&self) -> usize {
        self.cap as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Can resize the buffer, if out of capacity
    pub fn allocate_with_resize(&mut self) -> EntityId {
        if self.free_list.is_empty() && self.count == self.cap {
            self.grow((self.cap as f32 * 3.0 / 2.0).ceil() as u32);
        }
        self.allocate().unwrap()
    }

    /// Allocate will not grow the buffer, caller must ensure that sufficient capacity is reserved
    pub fn allocate(&mut self) -> Result<EntityId, HandleTableError> {
        // pop element off the free list
        //
        let index = match self.free_list.pop() {
            Some(i) => i,
            None => {
                if self.count == self.cap {
                    return Err(HandleTableError::OutOfCapacity);
                }
                self.count
            }
        };
        let entries = self.entries;
        self.count += 1;
        let entry;
        unsafe {
            entry = &mut *entries.add(index as usize);
            entry.arch = std::ptr::null_mut();
            entry.row_index = 0;
        }
        let id = EntityId::new(index as u32, entry.gen);
        Ok(id)
    }

    pub fn reserve(&mut self, additional: u32) {
        let new_cap = self.count + additional;
        if new_cap > self.cap {
            self.grow(new_cap);
        }
    }

    /// # Safety
    ///
    /// Caller must ensure that the id is valid
    pub(crate) unsafe fn update(
        &mut self,
        id: EntityId,
        arch: *mut ArchetypeStorage,
        row: RowIndex,
    ) {
        let index = id.index();
        debug_assert!(index < self.cap);
        let entry = &mut *self.entries.add(index as usize);
        debug_assert_eq!(id.gen(), entry.gen);
        entry.arch = arch;
        entry.row_index = row;
    }

    /// # Safety
    ///
    /// Caller must ensure that the id is valid
    pub(crate) unsafe fn update_row_index(&mut self, id: EntityId, row: RowIndex) {
        let index = id.index();
        debug_assert!(index < self.cap);
        let entry = &mut *self.entries.add(index as usize);
        debug_assert_eq!(id.gen(), entry.gen);
        entry.row_index = row;
    }

    /// # Safety
    ///
    /// Caller must ensure that the id is unused
    #[allow(unused)]
    pub(crate) unsafe fn set_gen(&mut self, index: usize, gen: u32) {
        while index >= self.cap as usize {
            self.grow((self.cap as f32 * 3.0 / 2.0).ceil() as u32);
        }
        let entry = &mut self.entries_mut()[index];
        entry.gen = gen;
    }

    /// # Safety
    ///
    /// Caller must ensure that the entity is not in the free list, nor is is allocated.
    #[allow(unused)]
    pub(crate) unsafe fn force_insert_entity(&mut self, id: EntityId) {
        debug_assert!(!self.is_valid(id));

        let index = id.index();
        while index >= self.cap {
            self.grow((self.cap as f32 * 3.0 / 2.0).ceil() as u32);
        }
        let entry = &mut *self.entries.add(index as usize);

        entry.gen = id.gen();
        self.update(id, std::ptr::null_mut(), SENTINEL);
        self.count += 1;
        debug_assert!(self.count <= self.cap);
        debug_assert!(self.count != self.cap || self.free_list.is_empty());
    }

    pub(crate) fn get(&self, id: EntityId) -> Option<&Entry> {
        let index = id.index();
        self.is_valid(id)
            .then(|| unsafe { &*self.entries.add(index as usize) })
    }

    pub fn read(
        &self,
        id: EntityId,
    ) -> Result<(NonNull<ArchetypeStorage>, RowIndex), HandleTableError> {
        let res = self.get(id).ok_or(HandleTableError::NotFound)?;
        if res.arch.is_null() {
            return Err(HandleTableError::Uninitialized);
        }

        Ok((unsafe { NonNull::new_unchecked(res.arch) }, res.row_index))
    }

    pub fn free(&mut self, id: EntityId) {
        self.count -= 1;
        let index = id.index();
        let entry: &mut Entry;
        unsafe {
            let entries = self.entries;
            entry = &mut *entries.add(index as usize);
        }
        debug_assert_eq!(id.gen(), entry.gen);
        entry.arch = std::ptr::null_mut();
        entry.row_index = SENTINEL;
        entry.gen = (entry.gen + 1) & ENTITY_GEN_MASK;
        self.free_list.push(index);
    }

    pub fn get_at_index(&self, ind: u32) -> EntityId {
        let entry = self.entries()[ind as usize];
        EntityId::new(ind, entry.gen)
    }

    pub fn is_valid(&self, id: EntityId) -> bool {
        let index = id.index() as usize;
        let gen = id.gen();
        if index >= self.cap as usize {
            return false;
        }
        let entry = &self.entries()[index];
        entry.gen == gen && entry.arch != std::ptr::null_mut()
    }

    pub(crate) fn entries(&self) -> &[Entry] {
        unsafe { std::slice::from_raw_parts(self.entries, self.cap as usize) }
    }

    #[allow(unused)]
    pub(crate) fn entries_mut(&mut self) -> &mut [Entry] {
        unsafe { std::slice::from_raw_parts_mut(self.entries, self.cap as usize) }
    }
}

impl Drop for EntityIndex {
    fn drop(&mut self) {
        unsafe {
            dealloc(
                self.entries.cast(),
                Layout::from_size_align_unchecked(
                    size_of::<Entry>() * (self.cap as usize),
                    align_of::<Entry>(),
                ),
            );
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Entry {
    pub gen: u32,
    pub arch: *mut ArchetypeStorage,
    pub row_index: RowIndex,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alloc() {
        let mut table = EntityIndex::new(512);

        for _ in 0..4 {
            let e = table.allocate().unwrap();
            assert_eq!(e.gen(), 0); // assert for the next step in the test
        }
        for i in 0..4 {
            let e = EntityId::new(i, 0);
            table.free(e);
            assert!(!table.is_valid(e));
        }
        for _ in 0..512 {
            let _e = table.allocate();
        }
    }

    #[test]
    fn handle_table_remove_tick_generation_test() {
        let mut table = EntityIndex::new(512);

        let a = table.allocate().unwrap();

        table.free(a);

        let b = table.get_at_index(a.index());

        assert_eq!(a.index(), b.index());
        assert_eq!(a.gen() + 1, b.gen());
    }

    #[test]
    fn can_grow_handles_test() {
        let mut table = EntityIndex::new(0);

        table.reserve(128);
        for _ in 0..128 {
            table.allocate().unwrap();
        }
    }
}
