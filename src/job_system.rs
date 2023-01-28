use smallvec::SmallVec;
use std::{
    marker::PhantomData,
    num::NonZeroUsize,
    panic::{catch_unwind, resume_unwind},
    pin::Pin,
    ptr::NonNull,
    sync::atomic::{AtomicIsize, Ordering},
    thread::JoinHandle,
};

use crate::{systems::InnerSystem, World};

mod queue;

// TODO: Job Allocator
type WorkerQueue<'a> = Pin<Box<queue::Queue<Pin<Box<Job>>>>>;

pub struct Executor {
    threads: Vec<JoinHandle<()>>,
    queues: Pin<Box<[WorkerQueue<'static>]>>,
}

impl Drop for Executor {
    fn drop(&mut self) {
        let threads = std::mem::take(&mut self.threads);
        for j in threads.into_iter() {
            j.join().unwrap_or(());
        }
    }
}

/// # Safety
///
/// Caller must ensure that the thread is joined before the queues are destroyed
unsafe fn worker_thread(id: usize, queues: QueueArray) {
    // TODO: exit condition
    // TODO: sleep
    let queues = &*queues.0;
    let mut steal_id = id;
    'main: loop {
        if let Ok(job) = queues[id].pop() {
            Pin::into_inner(job).execute();
            continue;
        }
        // if pop fails try to steal from another thread
        for _ in 0..queues.len() {
            steal_id = (steal_id + 1) % queues.len();
            if steal_id == id {
                continue;
            }
            match queues[id].steal(&queues[steal_id]) {
                Ok(_) => {
                    continue 'main;
                }
                Err(_) => {}
            }
        }
        // steal failed, go to sleep
    }
}

struct QueueArray(*const [WorkerQueue<'static>]);
unsafe impl Send for QueueArray {}

impl Executor {
    pub fn new(capacity: NonZeroUsize) -> Pin<Box<Self>> {
        let workers = std::thread::available_parallelism()
            .map(|x| x.get())
            .unwrap_or(1);
        let mut queues = Vec::with_capacity(workers);
        for _ in 0..workers {
            queues.push(Box::pin(queue::Queue::new(capacity)));
        }
        let queues = Pin::new(queues.into_boxed_slice());
        let q = &*queues as *const _;
        let mut result = Box::pin(Self {
            queues,
            threads: Vec::with_capacity(workers),
        });
        for i in 0..workers {
            let arr = QueueArray(q);
            result
                .threads
                .push(std::thread::spawn(move || unsafe { worker_thread(i, arr) }));
        }
        result
    }
}

pub struct Job {
    tasks_left: AtomicIsize,
    children: SmallVec<[NonNull<Job>; 1]>,
    func: fn(*const ()),
    data: *const (),
}

unsafe impl Send for Job {}

impl Job {
    pub fn execute(&mut self) {
        // TODO: catch_unwind
        debug_assert!(!self.data.is_null());
        (self.func)(self.data);
        self.data = std::ptr::null();
        for mut dep in self.children.drain(..) {
            unsafe {
                dep.as_mut().tasks_left.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }

    pub fn add_child(&mut self, child: &Job) {
        self.children.push(NonNull::from(child));
    }
}
