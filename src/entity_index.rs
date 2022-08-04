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
    free_list: u32,
    /// Currently allocated entries
    count: u32,
}

#[cfg(feature = "clone")]
impl Clone for EntityIndex {
    fn clone(&self) -> Self {
        let mut result = Self::new(self.cap);
        result.entries_mut().copy_from_slice(self.entries());
        result.free_list = self.free_list;
        result.count = self.count;
        result
    }
}

unsafe impl Send for EntityIndex {}
unsafe impl Sync for EntityIndex {}

const SENTINEL: u32 = !0;

#[allow(unused)]
struct FreeListWalker<'a> {
    entries: &'a [Entry],
    i: u32,
}

impl<'a> Iterator for FreeListWalker<'a> {
    type Item = (u32, u32);

    fn next(&mut self) -> Option<Self::Item> {
        if self.i == SENTINEL {
            return None;
        }
        let i = self.i;
        let gen;
        unsafe {
            gen = self.entries[self.i as usize].gen;
            self.i = self.entries[self.i as usize].data.free_list.next;
        }
        Some((gen, i))
    }
}

impl EntityIndex {
    // return (gen, index) tuples
    #[allow(unused)]
    pub(crate) fn walk_free_list(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        FreeListWalker {
            entries: self.entries(),
            i: self.free_list,
        }
    }

    /// Takes (gen, index) tuples
    ///
    /// # Safety
    ///
    /// Caller must ensure that no index in the free list has been allocated
    #[allow(unused)]
    pub(crate) unsafe fn restore_free_list(&mut self, mut list: impl Iterator<Item = (u32, u32)>) {
        let mut last = 0;
        if let Some((gen, i)) = list.next() {
            self.free_list = i;
            self.entries_mut()[i as usize].gen = gen;
            last = i;
        }
        for (gen, i) in list {
            if self.cap <= i {
                self.grow(i + 100);
            }
            self.entries_mut()[i as usize].gen = gen;
            self.entries_mut()[last as usize].data.free_list.next = i;
            last = i;
        }
        self.entries_mut()[last as usize].data.free_list.next = SENTINEL;
    }

    /// # Safety
    /// Creates a new uninitialized table, clients must call `initialize_free_list` after their
    /// manual initialization
    #[allow(unused)]
    pub(crate) unsafe fn new_uninit(initial_capacity: u32) -> Self {
        let mut result = Self::new(initial_capacity);
        for entry in result.entries_mut() {
            entry.data.free_list.next = SENTINEL;
            entry.gen = 0;
        }
        result.free_list = SENTINEL;
        result.cap = initial_capacity;
        result
    }

    /// # Safety
    /// Inserts the ID into this table
    ///
    /// Clients must ensure that this ID have not been allocated and that the free list is rebuilt
    /// before calling `allocate`
    pub(crate) unsafe fn force_insert(&mut self, id: EntityId) {
        debug_assert!(!self.is_valid(id));

        let index = id.index();
        if index >= self.cap {
            self.grow(index + 2);
        }

        let entry = &mut self.entries_mut()[id.index() as usize];
        entry.gen = id.gen();
        self.count += 1;
        debug_assert!(self.count <= self.cap);
        self.update(id, std::ptr::null_mut(), 0);
    }

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
            for i in 1..cap {
                ptr::write(
                    entries.add(i as usize),
                    Entry {
                        data: EntryData {
                            free_list: FreeList { next: i + 1 },
                        },
                        gen: 1, // 0 IDs can cause problems for clients so start at gen 1
                    },
                );
            }
            ptr::write(
                entries,
                Entry {
                    data: EntryData {
                        free_list: FreeList { next: 1 },
                    },
                    gen: 1,
                },
            );
            (&mut *entries.add(cap as usize - 1)).data.free_list.next = SENTINEL;
        };
        Self {
            entries,
            cap,
            free_list: 0,
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
                        data: EntryData {
                            free_list: FreeList { next: i + 1 },
                        },
                        gen: 1, // 0 IDs can cause problems for clients so start at gen 1
                    },
                );
            }
            (&mut *new_entries.add(new_cap as usize - 1))
                .data
                .free_list
                .next = SENTINEL;

            // update the free_list if empty
            dealloc(
                self.entries.cast(),
                Layout::from_size_align_unchecked(
                    size_of::<Entry>() * cap as usize,
                    align_of::<Entry>(),
                ),
            );
        }
        if self.free_list == SENTINEL {
            self.free_list = cap;
        }
        self.entries = new_entries;
        self.cap = new_cap;
    }

    pub fn len(&self) -> usize {
        self.count as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn allocate(&mut self) -> Result<EntityId, HandleTableError> {
        // pop element off the free list
        //
        if self.count == self.cap {
            self.grow((self.cap as f32 * 3.0 / 2.0).ceil() as u32);
        }
        let entries = self.entries;
        self.count += 1;
        let index = self.free_list;
        let entry;
        unsafe {
            // unlink this node from the free list
            // and create handle
            self.free_list = (*entries.add(index as usize)).data.free_list.next;
            entry = &mut *entries.add(index as usize);
            entry.data.meta.arch = std::ptr::null_mut();
            entry.data.meta.row_index = 0;
        }
        let id = EntityId::new(index as u32, entry.gen);
        Ok(id)
    }

    pub(crate) fn update(&mut self, id: EntityId, arch: *mut ArchetypeStorage, row: RowIndex) {
        debug_assert!(self.is_valid(id));
        let index = id.index();
        unsafe {
            let entry = &mut *self.entries.add(index as usize);
            entry.data.meta.arch = arch;
            entry.data.meta.row_index = row;
        }
    }

    pub(crate) fn get(&self, id: EntityId) -> Option<&Metadata> {
        let index = id.index();
        self.is_valid(id).then(|| unsafe {
            let entry = &*self.entries.add(index as usize);
            &entry.data.meta
        })
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
        debug_assert!(self.is_valid(id));
        self.count -= 1;
        let index = id.index();
        let entry: &mut Entry;
        unsafe {
            let entries = self.entries;
            entry = &mut *entries.add(index as usize);
        }
        entry.data.free_list.next = self.free_list;
        // 0 IDs can cause problems for clients so start at gen 1
        entry.gen = ((entry.gen + 1) & ENTITY_GEN_MASK).max(1);
        self.free_list = index;
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
        entry.gen == gen
    }

    fn entries(&self) -> &[Entry] {
        unsafe { std::slice::from_raw_parts(self.entries, self.cap as usize) }
    }

    #[allow(unused)]
    fn entries_mut(&mut self) -> &mut [Entry] {
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
struct Entry {
    gen: u32,
    /// If this entry is allocated, then data is the index of the entity
    /// if not, then data is the next link in the free_list
    data: EntryData,
}

#[derive(Clone, Copy)]
union EntryData {
    free_list: FreeList,
    meta: Metadata,
}

#[derive(Clone, Copy)]
pub(crate) struct FreeList {
    pub next: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct Metadata {
    arch: *mut ArchetypeStorage,
    row_index: RowIndex,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alloc() {
        let mut table = EntityIndex::new(512);

        for _ in 0..4 {
            let e = table.allocate().unwrap();
            assert!(table.is_valid(e));
            assert_eq!(e.gen(), 1); // assert for the next step in the test
        }
        for i in 0..4 {
            let e = EntityId::new(i, 1);
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
        let mut table = EntityIndex::new(4);

        for _ in 0..128 {
            table.allocate().unwrap();
        }
    }
}
