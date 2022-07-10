use std::{any::TypeId, cell::UnsafeCell, collections::HashMap};

pub struct ResourceStorage {
    pub(crate) resources: HashMap<TypeId, UnsafeCell<ErasedResource>>,
}

impl Clone for ResourceStorage {
    fn clone(&self) -> Self {
        Self {
            resources: self
                .resources
                .iter()
                .map(|(id, table)| (*id, UnsafeCell::new(unsafe { &*table.get() }.clone())))
                .collect(),
        }
    }
}

impl ResourceStorage {
    pub fn new() -> Self {
        Self {
            resources: Default::default(),
        }
    }

    pub fn insert<T: 'static + Clone>(&mut self, value: T) {
        match self.resources.entry(TypeId::of::<T>()) {
            std::collections::hash_map::Entry::Occupied(mut x) => {
                x.insert(UnsafeCell::new(ErasedResource::new(value)));
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(UnsafeCell::new(ErasedResource::new(value)));
            }
        }
    }

    pub fn fetch<T: 'static>(&self) -> Option<&T> {
        self.resources
            .get(&TypeId::of::<T>())
            .map(|table| unsafe { (&*table.get()).as_inner::<T>() })
    }

    pub fn fetch_mut<T: 'static>(&self) -> Option<&mut T> {
        self.resources
            .get(&TypeId::of::<T>())
            .map(|table| unsafe { (&mut *table.get()).as_inner_mut::<T>() })
    }
}

pub(crate) struct ErasedResource {
    inner: *mut u8,
    finalize: fn(&mut ErasedResource),
    clone: fn(&ErasedResource) -> ErasedResource,
}

impl Drop for ErasedResource {
    fn drop(&mut self) {
        (self.finalize)(self);
    }
}

impl Clone for ErasedResource {
    fn clone(&self) -> Self {
        (self.clone)(self)
    }
}

impl ErasedResource {
    pub fn new<T: Clone>(value: T) -> Self {
        let inner = Box::leak(Box::new(value));
        Self {
            inner: (inner as *mut T).cast(),
            finalize: |resource| unsafe {
                let _inner: Box<T> = Box::from_raw(resource.inner.cast::<T>());
            },
            clone: |resource| unsafe {
                let val = resource.as_inner::<T>().clone();
                ErasedResource::new(val)
            },
        }
    }

    /// # SAFETY
    /// Must be called with the same type as `new`
    pub unsafe fn as_inner<T>(&self) -> &T {
        &*self.inner.cast()
    }

    /// # SAFETY
    /// Must be called with the same type as `new`
    pub unsafe fn as_inner_mut<T>(&mut self) -> &mut T {
        &mut *self.inner.cast()
    }
}
