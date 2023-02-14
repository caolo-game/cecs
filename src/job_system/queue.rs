use std::{
    alloc::Layout,
    mem::MaybeUninit,
    num::NonZeroUsize,
    ptr::{self, NonNull},
    sync::atomic::{AtomicBool, AtomicIsize, Ordering},
};

use cache_padded::CachePadded;

pub struct Queue<T> {
    items: CachePadded<NonNull<MaybeUninit<T>>>,
    head: CachePadded<AtomicIsize>,
    tail: CachePadded<AtomicIsize>,
    // stealers can lock this queue so at most 1 thread is stealing at a time
    // since concurrent steals would fail anyway this seems like an OK compromise
    steal_lock: CachePadded<AtomicBool>,
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
                self.items
                    .as_ptr()
                    .add(self.index(tail))
                    .as_mut()
                    .unwrap()
                    .assume_init_drop();
                tail += 1;
            }

            std::alloc::dealloc(
                self.items.cast().as_ptr(),
                Layout::array::<MaybeUninit<T>>(self.capacity).unwrap(),
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

#[derive(Debug, Clone)]
pub enum PopError {
    Empty,
}

#[derive(Debug, Clone)]
pub enum StealError {
    Empty,
    Busy,
}

type PopResult<T> = Result<T, PopError>;

impl<T> Queue<T> {
    pub fn new(capacity: NonZeroUsize) -> Self {
        let capacity = capacity.get().max(2).next_power_of_two();
        unsafe {
            let jobs = std::alloc::alloc(Layout::array::<MaybeUninit<T>>(capacity).unwrap());
            Self {
                items: NonNull::new_unchecked(jobs.cast()).into(),
                head: AtomicIsize::new(0).into(),
                tail: AtomicIsize::new(0).into(),
                steal_lock: AtomicBool::new(false).into(),
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
        if head - tail >= self.capacity_mask as isize {
            return Err(PushError::Full(val));
        }

        let slot = self.items.as_ptr().add(self.index(head));
        ptr::write(slot, MaybeUninit::new(val));
        self.head.fetch_add(1, Ordering::Release);
        Ok(())
    }

    /// # Safety
    /// Only the owning thread may pop
    pub unsafe fn pop(&self) -> PopResult<T> {
        let head = self.head.fetch_sub(1, Ordering::AcqRel) - 1;
        let tail = self.tail.load(Ordering::Relaxed);
        if tail > head {
            // queue is empty
            // reset head
            self.head.store(tail, Ordering::Release);
            return Err(PopError::Empty);
        }

        let value = ptr::read(self.items.as_ptr().add(self.index(head)));

        if head != tail {
            // done
            return Ok(value.assume_init());
        }

        // `value` is the last item in the queue
        // ensure no other thread takes the value
        // reset queue to 0
        let new_tail = tail + 1;
        match self
            .tail
            .compare_exchange(tail, new_tail, Ordering::Release, Ordering::Relaxed)
        {
            Ok(_current_tail) => {
                self.head.store(new_tail, Ordering::Release);
                Ok(value.assume_init())
            }
            Err(tail) => {
                self.head.store(tail, Ordering::Release);
                Err(PopError::Empty)
            }
        }
    }

    /// Stealing from itself is a noop
    ///
    /// `self` will try to steal half of `other`'s items
    pub fn steal(&self, victim: &Self) -> Result<(), StealError> {
        if self.items == victim.items {
            return Ok(());
        }

        // capacity may increase during the steal, due to other threads stealing from `self`
        // but this should be infrequent enough that we'll ignore it
        let head = self.head.load(Ordering::Relaxed);
        let capacity = self.capacity_mask - self.len();
        'retry: loop {
            let desired = (victim.len() / 2).min(capacity);
            std::sync::atomic::fence(Ordering::Acquire);
            if desired == 0 {
                return Err(StealError::Empty);
            }
            if victim
                .steal_lock
                .compare_exchange_weak(false, true, Ordering::AcqRel, Ordering::Relaxed)
                .is_err()
            {
                // victim is locked by a third thread
                return Err(StealError::Busy);
            }
            let victim_tail = victim.tail.load(Ordering::Acquire);
            let target_tail = victim_tail + desired as isize;

            // copy the items
            //
            // another thread may steal from self while we're stealing
            // and steal can fail in which case we undo
            // so self.head must not be modified at this point
            let mut h = head;
            for t in victim_tail..target_tail {
                unsafe {
                    // TSAN complains about the read if it's not marked volatile
                    let stolen_goods =
                        ptr::read_volatile(victim.items.as_ptr().add(victim.index(t)));
                    let slot = self.items.as_ptr().add(self.index(h));
                    ptr::write(slot, stolen_goods);
                    h += 1;
                }
            }
            let victim_head = victim.head.load(Ordering::Acquire);
            // unlikely, but the producer thread might pop into the part of the queue
            // we're stealing
            if victim_head <= target_tail {
                // undo the insertion
                self.head.store(head, Ordering::Relaxed);
                victim.steal_lock.store(false, Ordering::Release);
                std::sync::atomic::fence(Ordering::Release);
                std::thread::yield_now();
                continue 'retry;
            }
            // commit changes
            self.head.store(h, Ordering::Relaxed);
            victim.tail.store(target_tail, Ordering::Release);
            victim.steal_lock.store(false, Ordering::Relaxed);
            std::sync::atomic::fence(Ordering::Release);
            return Ok(());
        }
    }

    #[inline]
    fn index(&self, i: isize) -> usize {
        i.max(0) as usize & self.capacity_mask
    }

    pub fn len(&self) -> usize {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Relaxed);

        (head - tail).max(0) as usize
    }

    #[allow(unused)]
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
            for _ in 0..7 {
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
        assert_eq!(q0.len(), 0);
        assert_eq!(q1.len(), 0);
    }

    /// Intended to be run via tsan
    #[test]
    fn steal_thread_test() {
        let q0 = Queue::new(NonZeroUsize::new(1000).unwrap());

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
                    let bar = Arc::clone(&bar);
                    thread::spawn(move || {
                        let q1 = Queue::new(NonZeroUsize::new(1000).unwrap());
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
