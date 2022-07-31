#[cfg(feature = "serde")]
mod serde_impl;

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
    #[error("HandleTable has no free capacity")]
    OutOfCapacity,
    #[error("Entity not found")]
    NotFound,
    #[error("Handle was not initialized")]
    Uninitialized,
}

pub struct HandleTable {
    entries: *mut Entry,
    cap: u32,
    free_list: u32,
    /// Currently allocated entries
    count: u32,
}

#[cfg(feature = "clone")]
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

#[cfg(feature = "serde")]
type SerializedMetadata = Vec<(crate::TypeHash, u32, EntityId)>;

impl HandleTable {
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
                        data: i + 1,
                        gen: 1, // 0 IDs can cause problems for clients so start at gen 1
                    },
                );
            }
            ptr::write(
                entries.add(cap as usize - 1),
                Entry {
                    data: SENTINEL,
                    gen: 1,
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

    fn grow(&mut self, new_cap: u32) {
        let cap = self.cap;
        assert!(new_cap > cap);
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
                        data: i + 1,
                        gen: 1, // 0 IDs can cause problems for clients so start at gen 1
                    },
                );
            }
            ptr::write(
                new_entries.add(new_cap as usize - 1),
                Entry {
                    data: SENTINEL,
                    gen: 1,
                },
            );
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

    pub fn alloc(&mut self) -> Result<EntityId, HandleTableError> {
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
            self.free_list = (*entries.add(self.free_list as usize)).data;
            // create handle
            entry = &mut *entries.add(index as usize);
            entry.data = SENTINEL;
        }
        let id = EntityId::new(index, entry.gen);
        Ok(id)
    }

    pub(crate) fn update(&mut self, id: EntityId, data: u32) {
        debug_assert!(self.is_valid(id));
        let index = id.index();
        unsafe {
            let entry = &mut *self.entries.add(index as usize);
            entry.data = data;
        }
    }

    pub(crate) fn get(&self, id: EntityId) -> Option<u32> {
        let index = id.index();
        self.is_valid(id).then(|| unsafe {
            let entry = &*self.entries.add(index as usize);
            entry.data
        })
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

impl Drop for HandleTable {
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
struct Entry {
    /// If this entry is allocated, then data is the index of the entity
    /// if not, then data is the next link in the free_list
    data: u32,
    gen: u32,
}

#[cfg_attr(feature = "clone", derive(Clone))]
pub struct EntityIndex {
    pub(crate) handles: HandleTable,
    pub(crate) metadata: Vec<(*mut ArchetypeStorage, RowIndex, EntityId)>,
}

unsafe impl Send for EntityIndex {}
unsafe impl Sync for EntityIndex {}

impl EntityIndex {
    pub fn new(capacity: u32) -> Self {
        let handles = HandleTable::new(capacity);
        Self {
            handles,
            metadata: Vec::new(),
        }
    }

    pub fn is_valid(&self, id: EntityId) -> bool {
        self.handles.is_valid(id)
    }

    #[cfg(feature = "serde")]
    pub fn load<'a, D: serde::Deserializer<'a>>(
        d: D,
        world: &mut crate::World,
    ) -> Result<Self, D::Error> {
        use serde::de::{self, MapAccess, Visitor};

        use crate::World;

        struct Vis<'a>(&'a mut World);

        impl<'a> Vis<'a> {
            fn rows_to_metadata(
                self,
                rows: SerializedMetadata,
            ) -> Vec<(*mut ArchetypeStorage, RowIndex, EntityId)> {
                rows.into_iter()
                    .map(move |(_type_id, _row_index, id)| {
                        let default_archetype = self.0.archetypes.get_mut(&0).unwrap();

                        let index = default_archetype.insert_entity(id);
                        default_archetype.set_component(index, ());
                        let ptr = &mut *default_archetype.as_mut() as *mut ArchetypeStorage;
                        (ptr, index, id)
                    })
                    .collect()
            }
        }

        impl<'a, 'de> Visitor<'de> for Vis<'a> {
            type Value = EntityIndex;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("EntityIndex")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let handles = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;

                let meta: SerializedMetadata = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;

                Ok(EntityIndex {
                    handles,
                    metadata: self.rows_to_metadata(meta),
                })
            }

            fn visit_map<V>(self, mut map: V) -> Result<EntityIndex, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut handles: Option<HandleTable> = None;
                let mut rows: Option<SerializedMetadata> = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        "handles" => {
                            handles = Some(map.next_value()?);
                        }
                        "rows" => {
                            rows = Some(map.next_value()?);
                        }
                        _ => {}
                    }
                }
                let handles = handles.ok_or_else(|| de::Error::missing_field("handles"))?;
                let rows = rows.ok_or_else(|| de::Error::missing_field("handles"))?;
                Ok(EntityIndex {
                    handles,
                    metadata: self.rows_to_metadata(rows),
                })
            }
        }

        d.deserialize_struct("EntityIndex", &["handles", "rows"], Vis(world))
    }

    pub fn allocate(&mut self) -> Result<EntityId, HandleTableError> {
        let id = self.handles.alloc()?;
        let index = self.metadata.len() as u32;
        self.metadata.push((std::ptr::null_mut(), 0, id));
        self.handles.update(id, index);
        #[cfg(feature = "tracing")]
        tracing::trace!(id = tracing::field::display(id), "Allocated entity");
        Ok(id)
    }

    pub fn update(
        &mut self,
        id: EntityId,
        payload: (NonNull<ArchetypeStorage>, RowIndex),
    ) -> Result<(), HandleTableError> {
        let index = self.handles.get(id).ok_or(HandleTableError::NotFound)? as usize;
        debug_assert_eq!(
            self.metadata[index].2, id,
            "Metadata corruption! Found: {} Expected: {}",
            self.metadata[index].2, id
        );
        self.metadata[index] = (payload.0.as_ptr(), payload.1, id);
        Ok(())
    }

    pub fn delete(&mut self, id: EntityId) -> Result<(), HandleTableError> {
        #[cfg(feature = "tracing")]
        tracing::trace!(id = tracing::field::display(id), "Deleting entity");

        if !self.handles.is_valid(id) {
            return Err(HandleTableError::NotFound);
        }
        if self.metadata.len() > 1 {
            let last_index = self.metadata.len() as u32 - 1;
            let last_id = self.metadata[last_index as usize].2;

            let index = self.handles.get(id).unwrap();
            self.metadata.swap_remove(index as usize);
            self.handles.update(last_id, index);
        } else {
            // last item
            self.metadata.clear();
        }
        self.handles.free(id);
        Ok(())
    }

    pub fn read(
        &self,
        id: EntityId,
    ) -> Result<(NonNull<ArchetypeStorage>, RowIndex), HandleTableError> {
        let index = self.handles.get(id).ok_or(HandleTableError::NotFound)? as usize;
        let res = self.metadata[index];
        if res.0.is_null() {
            return Err(HandleTableError::Uninitialized);
        }

        Ok((unsafe { NonNull::new_unchecked(res.0) }, res.1))
    }

    pub fn len(&self) -> usize {
        self.metadata.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for EntityIndex {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        let mut s = s.serialize_struct("EntityIndex", 2)?;
        s.serialize_field("handles", &self.handles)?;
        // serialize archetype type, row_index, entity_id tuples
        s.serialize_field(
            "rows",
            &self
                .metadata
                .iter()
                .map(|(ptr, row, id)| unsafe {
                    (ptr.as_ref().map(|x| x.ty).unwrap_or(0), *row, *id)
                })
                .collect::<SerializedMetadata>(),
        )?;

        s.end()
    }
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

    #[test]
    fn can_grow_handles_test() {
        let mut table = HandleTable::new(4);

        for _ in 0..128 {
            table.alloc().unwrap();
        }
    }
}
