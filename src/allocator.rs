use std::alloc::Layout;

pub struct ComponentAllocator {
    blocks: Vec<*mut u8>,
    block_layout: Layout,
}

#[derive(Debug, Clone, Copy)]
pub struct Allocation {
    pub begin: *mut u8,
    pub block_idx: usize,
}

impl ComponentAllocator {
    pub fn new<T: Sized>() -> Self {
        // TODO: configure size?
        let layout = Layout::array::<T>(4096).unwrap();
        Self {
            blocks: Vec::default(),
            block_layout: layout,
        }
    }

    pub fn alloc(&mut self, n: usize) -> Allocation {
        todo!()
    }

    pub fn free(&mut self, alloc: Allocation) {
        let block = self.blocks[alloc.block_idx];
        todo!()
    }

    pub fn clear(&mut self) {
        for ptr in self.blocks.drain(..) {
            unsafe {
                std::alloc::dealloc(ptr, self.block_layout);
            }
        }
    }
}

impl Drop for ComponentAllocator {
    fn drop(&mut self) {
        self.clear();
    }
}
