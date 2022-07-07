mod pt_iter;

#[cfg(feature = "serde")]
mod serde_impl;

use crate::RowIndex;

use self::pt_iter::PTIter;
use std::{mem::MaybeUninit, ptr::drop_in_place};

const PAGE_SIZE: usize = 512;
const PAGE_FLAG_SIZE: usize = PAGE_SIZE / 64;
const PAGE_MASK: usize = PAGE_SIZE - 1; // assumes that page size is power of two

type PageEntry<T> = Option<Box<Page<T>>>;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PageTable<T> {
    num_entities: usize,
    pages: Vec<PageEntry<T>>,
}

impl<T: Clone> Clone for PageTable<T> {
    fn clone(&self) -> Self {
        Self {
            num_entities: self.num_entities,
            pages: self.pages.clone(),
        }
    }
}

impl<T> Default for PageTable<T> {
    fn default() -> Self {
        Self::new(30_000)
    }
}

impl<T> PageTable<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            num_entities: 0,
            pages: Vec::with_capacity(capacity / PAGE_SIZE),
        }
    }

    pub fn get(&self, index: RowIndex) -> Option<&T> {
        let page_index = index as usize / PAGE_SIZE;
        self.pages
            .get(page_index)
            .and_then(|p| p.as_ref())
            .and_then(|page| page.get(index as usize & PAGE_MASK))
    }

    pub fn get_mut(&mut self, index: RowIndex) -> Option<&mut T> {
        let page_index = index as usize / PAGE_SIZE;
        self.pages
            .get_mut(page_index)
            .and_then(|p| p.as_mut())
            .and_then(|page| page.get_mut(index as usize & PAGE_MASK))
    }

    pub fn remove(&mut self, index: RowIndex) -> Option<T> {
        let page_index = index as usize / PAGE_SIZE;
        let mut delete_page = false;
        let result = self
            .pages
            .get_mut(page_index)
            .and_then(|p| p.as_mut())
            .and_then(|page| {
                let result = page.remove(index as usize & PAGE_MASK);
                delete_page = page.is_empty();
                result
            })
            .map(|item| {
                // if removal succeeded
                self.num_entities -= 1;
                item
            });
        if delete_page {
            if let Some(page) = self.pages.get_mut(page_index) {
                *page = None;
            }
        }
        result
    }

    /// Returns the previous value, if any
    pub fn insert(&mut self, index: RowIndex, value: T) -> Option<T> {
        if let Some(existing) = self.get_mut(index) {
            Some(std::mem::replace(existing, value))
        } else {
            self.num_entities += 1;
            let page_ind = index as usize / PAGE_SIZE;
            if page_ind >= self.pages.len() {
                self.pages.resize_with(page_ind + 1, Default::default);
            }
            self.pages[page_ind]
                .get_or_insert_default()
                .insert(index as usize & PAGE_MASK, value);
            None
        }
    }

    pub fn len(&self) -> usize {
        self.num_entities
    }

    pub fn is_empty(&self) -> bool {
        self.num_entities == 0
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (RowIndex, &T)> {
        let it = self
            .pages
            .iter()
            .enumerate()
            .filter_map(|(page_id, page)| page.as_ref().map(|page| (page_id, page)))
            .flat_map(|(page_id, page)| {
                let offset = page_id * PAGE_SIZE;
                page.iter().map(move |(id, item)| {
                    let id = id + offset as u32;
                    (id, item)
                })
            });
        PTIter::new(it, self.num_entities)
    }

    pub fn iter_mut(&mut self) -> impl ExactSizeIterator<Item = (RowIndex, &mut T)> {
        let it = self
            .pages
            .iter_mut()
            .enumerate()
            .filter_map(|(page_id, page)| page.as_mut().map(|page| (page_id, page)))
            .flat_map(|(page_id, page)| {
                let offset = page_id * PAGE_SIZE;
                page.iter_mut().map(move |(id, item)| {
                    let id = id + offset as u32;
                    (id, item)
                })
            });
        PTIter::new(it, self.num_entities)
    }

    pub fn clear(&mut self) {
        self.num_entities = 0;
        self.pages.clear();
    }

    pub fn contains(&self, index: RowIndex) -> bool {
        let page_index = index as usize / PAGE_SIZE;
        self.pages
            .get(page_index)
            .and_then(|p| p.as_ref())
            .map(|page| page.contains(index as usize & PAGE_MASK))
            .unwrap_or(false)
    }
}

struct Page<T> {
    filled: [u64; PAGE_FLAG_SIZE],
    // TODO:
    // pack existing entities tightly, also store their ids
    // iterate on tight arrays...
    // eliminate the existance check during iteration
    data: [MaybeUninit<T>; PAGE_SIZE],
    len: usize,
}

impl<T: Clone> Clone for Page<T> {
    fn clone(&self) -> Self {
        let mut result = Self {
            data: [Self::D; PAGE_SIZE],
            len: self.len,
            filled: self.filled,
        };

        for (id, t) in self.iter() {
            unsafe {
                std::ptr::write(result.data[id as usize].as_mut_ptr(), t.clone());
            }
        }

        result
    }
}

impl<T> Default for Page<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Page<T> {
    const D: MaybeUninit<T> = MaybeUninit::uninit();
    pub fn new() -> Self {
        Self {
            filled: [0; PAGE_FLAG_SIZE],
            data: [Self::D; PAGE_SIZE],
            len: 0,
        }
    }

    pub fn contains(&self, id: usize) -> bool {
        let flags = self.filled[id / 64];
        ((flags >> (id & 63)) & 1) != 0
    }

    pub fn iter(&self) -> impl Iterator<Item = (RowIndex, &T)> {
        (0..self.data.len()).filter_map(move |i| {
            let flag_idx = i / 64;
            unsafe {
                let flags = self.filled.get_unchecked(flag_idx);
                if (*flags & (1 << (i as u64 & 63))) != 0 {
                    Some((i as u32, &*self.data.get_unchecked(i).as_ptr()))
                } else {
                    None
                }
            }
        })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (RowIndex, &mut T)> + '_ {
        (0..self.data.len()).filter_map(move |i| {
            let flag_idx = i / 64;
            unsafe {
                let flags = self.filled.get_unchecked(flag_idx);
                if (*flags & (1 << (i as u64 & 63))) != 0 {
                    Some((i as u32, &mut *self.data.get_unchecked_mut(i).as_mut_ptr()))
                } else {
                    None
                }
            }
        })
    }

    pub fn get(&self, i: usize) -> Option<&T> {
        debug_assert!(i / 64 < self.filled.len());
        unsafe {
            let flags = self.filled.get_unchecked(i / 64);
            ((flags >> (i & 63)) & 1 == 1).then(|| &*self.data.get_unchecked(i).as_ptr())
        }
    }

    pub fn get_mut(&mut self, i: usize) -> Option<&mut T> {
        debug_assert!(i / 64 < self.filled.len());
        unsafe {
            let flags = self.filled.get_unchecked(i / 64);
            ((flags >> (i & 63)) & 1 == 1)
                .then(|| &mut *self.data.get_unchecked_mut(i).as_mut_ptr())
        }
    }

    pub fn insert(&mut self, i: usize, value: T) {
        assert!(
            self.filled
                .get(i / 64)
                .copied()
                .map(|flags| flags >> (i & 63) & 1 == 0)
                .unwrap(),
            "RowIndex {} is invalid or occupied",
            i
        );
        let flags = self.filled[i / 64];
        self.filled[i / 64] = flags | (1 << (i & 63));
        if flags == self.filled[i / 64] {
            // we already had an entry in this position
            unsafe {
                drop_in_place(self.data[i].as_mut_ptr());
            }
        } else {
            self.len += 1;
        }
        unsafe {
            std::ptr::write(self.data[i].as_mut_ptr(), value);
        }
    }

    pub fn remove(&mut self, i: usize) -> Option<T> {
        if let Some(flags) = self
            .filled
            .get_mut(i / 64)
            .filter(|flags| (**flags >> (i as u64 & 63)) & 1 == 1)
        {
            *flags ^= 1 << (i & 63);
            let res = unsafe { std::ptr::read(self.data.get_unchecked_mut(i).as_mut_ptr()) };
            self.len -= 1;
            Some(res)
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    #[allow(unused)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T> Drop for Page<T> {
    fn drop(&mut self) {
        for (i, flags) in self.filled.iter().enumerate() {
            for j in 0..64 {
                if (flags >> j) & 1 == 1 {
                    unsafe {
                        drop_in_place(self.data[i * 64 + j].as_mut_ptr());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_retrieve() {
        let mut table = PageTable::<i64>::new(1024);
        assert!(table.is_empty());
        assert_eq!(table.len(), 0);

        let id = 512;
        table.insert(id, 42);

        assert!(!table.is_empty());
        assert_eq!(table.len(), 1);

        // get and get_mut should be consistent
        assert_eq!(table.contains(id), true);
        assert_eq!(table.get(id).copied(), Some(42));
        assert_eq!(table.get_mut(id).copied(), Some(42));

        assert_eq!(table.get(128), None);
        assert_eq!(table.contains(128), false);
    }

    #[test]
    fn test_remove() {
        let mut table = PageTable::<i64>::new(1024);
        assert!(table.is_empty());
        assert_eq!(table.len(), 0);

        let id = 21;
        table.insert(id, 42);
        assert_eq!(table.get(id).copied(), Some(42));
        let removed = table.remove(id);
        assert_eq!(removed, Some(42));
        assert_eq!(table.get(id).copied(), None);
        assert!(table.is_empty());
        assert_eq!(table.len(), 0);
    }

    #[test]
    fn test_iter() {
        let mut table = PageTable::<i64>::new(0);

        table.insert(12, 12);
        table.insert(521, 521);
        table.insert(333, 333);
        table.insert(666, 666);

        let it = table.iter();
        assert_eq!(it.len(), table.len());
        assert_eq!(it.len(), 4);
        let actual: Vec<_> = it.collect();

        dbg!(&actual);

        let expected = [12, 333, 521, 666];

        assert_eq!(expected.len(), actual.len());

        for (exp, actual) in expected.iter().copied().zip(actual.iter()) {
            assert_eq!(exp, actual.0);
            assert_eq!(exp as i64, *actual.1);
        }
    }

    #[test]
    fn test_iter_mut() {
        let mut table = PageTable::<i64>::new(0);

        table.insert(12, 12);
        table.insert(521, 521);
        table.insert(333, 333);
        table.insert(666, 666);

        let iter_res: Vec<_> = table.iter().map(|(_x, y)| *y).collect();

        let iter_mut: Vec<_> = table.iter_mut().map(|(_x, y)| *y).collect();
        dbg!(&iter_res, &iter_mut);

        assert_eq!(iter_res.len(), iter_mut.len());

        for (exp, actual) in iter_res.iter().zip(iter_mut.iter()) {
            assert_eq!(exp, actual);
        }
    }

    #[test]
    fn remove_test() {
        let mut table = PageTable::<i64>::new(0);

        table.insert(12, 12);
        table.insert(521, 521);
        table.insert(333, 333);
        table.insert(666, 666);

        let iter_res_a: Vec<_> = table.iter().map(|(_x, y)| *y).collect();

        table.remove(12);

        let iter_res_b: Vec<_> = table.iter().map(|(_x, y)| *y).collect();

        assert_eq!(&iter_res_a[1..], &iter_res_b[..]);
    }
}
