mod serde_impl;

use std::{
    alloc::{alloc, dealloc, Layout},
    mem::{align_of, size_of},
    ptr,
};

use crate::entity_id::{EntityId, ENTITY_GEN_MASK};

#[derive(Debug, Clone, thiserror::Error)]
pub enum HandleTableError {
    #[error("HandleTable has no free capacity")]
    OutOfCapacity,
}

pub struct HandleTable {
    entries: *mut Entry,
    cap: u32,
    free_list: u32,
    /// Currently allocated entries
    count: u32,
}

impl Clone for HandleTable {
    fn clone(&self) -> Self {
        let mut result = Self::new(self.cap);
        result.entries_mut().copy_from_slice(self.entries());
        result.free_list = self.free_list;
        result.count = self.count;
        result
    }
}

unsafe impl Send for HandleTable {}
unsafe impl Sync for HandleTable {}

const SENTINEL: u32 = !0;

impl HandleTable {
    pub fn new(cap: u32) -> Self {
        let entries;
        unsafe {
            entries = alloc(Layout::from_size_align_unchecked(
                size_of::<Entry>() * (cap as usize + 1),
                align_of::<Entry>(),
            )) as *mut Entry;
            assert!(!entries.is_null());
            for i in 0..cap {
                ptr::write(
                    entries.add(i as usize),
                    Entry {
                        data: i + 1,
                        gen: 1, // 0 IDs can cause problems for clients so start at gen 1
                    },
                );
            }
            ptr::write(
                entries.add(cap as usize),
                Entry {
                    data: SENTINEL,
                    gen: SENTINEL,
                },
            );
        };
        Self {
            entries,
            cap,
            free_list: 0,
            count: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.count as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn alloc(&mut self) -> Result<EntityId, HandleTableError> {
        let entries = self.entries;
        // pop element off the free list
        //
        if self.free_list == SENTINEL {
            return Err(HandleTableError::OutOfCapacity);
        }
        self.count += 1;
        let index = self.free_list;
        let entry;
        unsafe {
            self.free_list = (*entries.add(self.free_list as usize)).data;
            // create handle
            entry = &mut *entries.add(index as usize);
            entry.data = SENTINEL;
        }
        let id = EntityId::new(index, entry.gen);
        Ok(id)
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
        entry.data = self.free_list;
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
        let entry = self.entries()[index];
        // == SENTINEL means that this entry is allocated
        entry.gen == gen && entry.data == SENTINEL
    }

    fn entries(&self) -> &[Entry] {
        unsafe { std::slice::from_raw_parts(self.entries, self.cap as usize) }
    }

    fn entries_mut(&mut self) -> &mut [Entry] {
        unsafe { std::slice::from_raw_parts_mut(self.entries, self.cap as usize) }
    }
}

impl Drop for HandleTable {
    fn drop(&mut self) {
        unsafe {
            dealloc(
                self.entries as *mut u8,
                Layout::from_size_align_unchecked(
                    size_of::<Entry>() * (self.cap as usize + 1),
                    align_of::<Entry>(),
                ),
            );
        }
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct Entry {
    /// If this entry is allocated, then data is the index of the entity
    /// if not, then data is the next link in the free_list
    data: u32,
    gen: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alloc() {
        let mut table = HandleTable::new(512);

        for _ in 0..4 {
            let e = table.alloc().unwrap();
            assert!(table.is_valid(e));
            assert_eq!(e.gen(), 1); // assert for the next step in the test
        }
        for i in 0..4 {
            let e = EntityId::new(i, 1);
            table.free(e);
            assert!(!table.is_valid(e));
        }
        for _ in 0..512 {
            let _e = table.alloc();
        }
    }

    #[test]
    fn handle_table_remove_tick_generation_test() {
        let mut table = HandleTable::new(512);

        let a = table.alloc().unwrap();

        table.free(a);

        let b = table.get_at_index(a.index());

        assert_eq!(a.index(), b.index());
        assert_eq!(a.gen() + 1, b.gen());
    }
}
