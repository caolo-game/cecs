use std::{
    alloc::Layout,
    num::NonZeroUsize,
    ptr::{self, NonNull},
    sync::atomic::{self, AtomicBool, AtomicIsize, Ordering},
};

// assume 64 byte cachelines for the vast majority of deployments
#[repr(align(64))]
pub struct Queue<T> {
    items: NonNull<T>,
    head: AtomicIsize,
    tail: AtomicIsize,
    // stealers can lock this queue so at most 1 thread is stealing at a time
    steal_lock: AtomicBool,
    capacity: usize,
    capacity_mask: usize,
}

unsafe impl<T> Send for Queue<T> where T: Send {}
unsafe impl<T> Sync for Queue<T> where Queue<T>: Send {}

impl<T> Drop for Queue<T> {
    fn drop(&mut self) {
        unsafe {
            let mut tail = self.tail.load(Ordering::Relaxed);
            let head = self.head.load(Ordering::Relaxed);

            while head > tail {
                std::ptr::drop_in_place(self.items.as_ptr().add(self.index(tail)));
                tail += 1;
            }

            std::alloc::dealloc(
                self.items.cast().as_ptr(),
                Layout::array::<T>(self.capacity).unwrap(),
            );
        }
    }
}

#[derive(Debug)]
pub enum PushError<T> {
    /// Returns the pushed item
    Full(T),
}

type PushResult<T> = Result<(), PushError<T>>;

#[derive(Debug)]
pub enum PopError {
    Empty,
}

type PopResult<T> = Result<T, PopError>;

impl<T> Queue<T> {
    pub fn new(capacity: NonZeroUsize) -> Self {
        let capacity = capacity.get().max(2).next_power_of_two();
        unsafe {
            let jobs = std::alloc::alloc(Layout::array::<T>(capacity).unwrap());
            Self {
                items: NonNull::new_unchecked(jobs.cast()),
                head: AtomicIsize::new(0),
                tail: AtomicIsize::new(0),
                steal_lock: AtomicBool::new(false),
                capacity,
                capacity_mask: capacity - 1,
            }
        }
    }

    /// # Safety
    /// Only the owning thread may push
    pub unsafe fn push(&self, val: T) -> PushResult<T> {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Relaxed);
        atomic::fence(Ordering::Acquire);
        if head - tail >= self.capacity as isize {
            return Err(PushError::Full(val));
        }

        let slot = self.items.as_ptr().add(self.index(head));
        ptr::write(slot, val);
        self.head.fetch_add(1, Ordering::Release);
        Ok(())
    }

    /// # Safety
    /// Only the owning thread may pop
    pub unsafe fn pop(&self) -> PopResult<T> {
        let head = self.head.fetch_sub(1, Ordering::AcqRel) - 1;
        let tail = self.tail.load(Ordering::Relaxed);
        atomic::fence(Ordering::Acquire);
        if tail > head {
            // queue is empty
            //reset head
            self.head.store(tail, Ordering::Release);
            return Err(PopError::Empty);
        }

        let value = ptr::read(self.items.as_ptr().add(self.index(head)));

        if head != tail {
            // done
            return Ok(value);
        }

        // `value` is the last item in the queue
        let new_tail = tail + 1;
        match self
            .tail
            .compare_exchange_weak(tail, new_tail, Ordering::Release, Ordering::Relaxed)
        {
            Ok(_) => Ok(value),
            Err(tail) => {
                // another thread stole this item
                std::mem::forget(value);
                // reset queue to 0
                self.head.store(tail, Ordering::Release);
                Err(PopError::Empty)
            }
        }
    }

    /// Stealing from itself is a noop
    ///
    /// `self` will try to steal half of `other`'s items
    pub fn steal(&self, victim: &Self) -> Result<(), PopError> {
        if self.items == victim.items {
            return Ok(());
        }

        let mut victim_tail = victim.tail.load(Ordering::Relaxed);
        'retry: loop {
            let head = self.head.load(Ordering::Relaxed);
            let target = (victim.len() / 2).min(self.capacity - self.len());
            let target_tail = victim_tail + target as isize;
            if target == 0 {
                return Err(PopError::Empty);
            }
            if victim
                .steal_lock
                .compare_exchange_weak(false, true, Ordering::Release, Ordering::Relaxed)
                .is_err()
            {
                // victim is locked by a third thread
                std::thread::yield_now();
                continue 'retry;
            }

            // copy the items
            for t in victim_tail..target_tail {
                unsafe {
                    if self
                        .push(ptr::read(victim.items.as_ptr().add(victim.index(t))))
                        .is_err()
                    {
                        panic!("Insertion failed");
                    }
                }
            }
            let victim_head = victim.head.load(Ordering::Acquire);
            // unlikely, but the producer thread might pop into the part of the queue
            // we're stealing
            if victim_head <= target_tail {
                // undo the insertion
                self.head.store(head, Ordering::Relaxed);
                victim.steal_lock.store(false, Ordering::Release);
                std::thread::yield_now();
                continue 'retry;
            }
            victim.tail.store(target_tail, Ordering::Relaxed);
            victim.steal_lock.store(false, Ordering::Release);
            atomic::fence(Ordering::Release);
            return Ok(());
        }
    }

    fn index(&self, i: isize) -> usize {
        i.max(0) as usize & self.capacity_mask
    }

    pub fn len(&self) -> usize {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Relaxed);

        (head - tail).max(0) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Barrier},
        thread,
    };

    use super::*;

    #[test]
    fn basic_full_test() {
        let q = Queue::new(NonZeroUsize::new(8).unwrap());

        assert_eq!(q.capacity, 8);

        unsafe {
            for _ in 0..8 {
                q.push(42i32).unwrap();
            }
            let res = q.push(69);
            assert!(res.is_err());
        }
    }

    #[test]
    fn basic_empty_test() {
        let q = Queue::<i32>::new(NonZeroUsize::new(8).unwrap());

        unsafe {
            let res = q.pop();
            assert!(res.is_err());
        }
    }

    #[test]
    fn basic_producer_test() {
        let q = Queue::new(NonZeroUsize::new(16).unwrap());

        unsafe {
            q.push(42i32).unwrap();
            q.push(69i32).unwrap();
            let last = q.pop().unwrap();

            assert_eq!(last, 69);
        }
    }

    #[test]
    fn basic_steal_test() {
        let q0 = Queue::new(NonZeroUsize::new(16).unwrap());
        let q1 = Queue::new(NonZeroUsize::new(16).unwrap());

        for i in 0..4i32 {
            unsafe {
                q0.push(i).unwrap();
            }
        }

        assert_eq!(q0.len(), 4);
        assert_eq!(q1.len(), 0);

        q1.steal(&q0).unwrap();

        assert_eq!(q0.len(), 2);
        assert_eq!(q1.len(), 2);

        unsafe {
            assert_eq!(q1.pop().unwrap(), 1);
            assert_eq!(q1.pop().unwrap(), 0);
            assert_eq!(q0.pop().unwrap(), 3);
            assert_eq!(q0.pop().unwrap(), 2);
        }
    }

    /// Intended to be run via tsan
    #[test]
    fn steal_thread_test() {
        let q0 = Queue::new(NonZeroUsize::new(1000).unwrap());
        let q1 = Queue::new(NonZeroUsize::new(1000).unwrap());

        let n = thread::available_parallelism()
            .map(|x| x.get())
            .unwrap_or(1);
        let bar = Arc::new(Barrier::new(n + 1));

        for i in 0..1000i32 {
            unsafe {
                q0.push(i).unwrap();
            }
        }

        unsafe {
            let threads = (0..n)
                .map(|_| {
                    let q0 = std::mem::transmute::<&Queue<i32>, &'static Queue<i32>>(&q0);
                    let q1 = std::mem::transmute::<&Queue<i32>, &'static Queue<i32>>(&q1);
                    let bar = Arc::clone(&bar);
                    thread::spawn(move || {
                        bar.wait();
                        q1.steal(q0).unwrap_or_default();
                    })
                })
                .collect::<Vec<_>>();

            bar.wait();
            while q0.pop().is_ok() {}
            for j in threads {
                j.join().unwrap();
            }
        }
    }
}
