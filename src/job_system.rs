use smallvec::SmallVec;
use std::{
    cell::UnsafeCell,
    marker::PhantomData,
    num::NonZeroUsize,
    panic::{catch_unwind, resume_unwind},
    pin::Pin,
    ptr::NonNull,
    sync::{
        atomic::{AtomicIsize, Ordering},
        Arc,
    },
    thread::JoinHandle,
};

use crate::{systems::InnerSystem, World};

use self::queue::PushError;

mod queue;

// TODO: Job Allocator
type WorkerQueue<'a> = Pin<Box<queue::Queue<Job>>>;

pub struct JobPool {
    threads: Vec<JoinHandle<()>>,
    queues: Pin<Box<[WorkerQueue<'static>]>>,
}

impl Drop for JobPool {
    fn drop(&mut self) {
        let threads = std::mem::take(&mut self.threads);
        for j in threads.into_iter() {
            j.join().unwrap_or(());
        }
    }
}

thread_local! {
    pub static THREAD_INDEX: UnsafeCell<usize> = UnsafeCell::new(0);
}

/// Context for a worker thread
struct Executor {
    id: usize,
    queues: QueueArray,
}

impl Executor {
    fn new(id: usize, queues: QueueArray) -> Self {
        Self { id, queues }
    }

    /// # Safety
    ///
    /// Caller must ensure that the thread is joined before the queues are destroyed
    unsafe fn worker_thread(&self) {
        // TODO: exit condition
        // TODO: sleep
        let queues = &*self.queues.0;
        let id = self.id;
        THREAD_INDEX.with(move |tid| std::ptr::write(tid.get(), id));
        let mut steal_id = id;
        'main: loop {
            if let Ok(mut job) = queues[id].pop() {
                job.execute();
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
}

struct QueueArray(*const [WorkerQueue<'static>]);
unsafe impl Send for QueueArray {}

impl JobPool {
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
        // the main thread is also used a worker on wait points
        // the index of the main thread is 0
        for i in 1..workers {
            let arr = QueueArray(q);
            let mut worker = Executor::new(i, arr);
            result.threads.push(std::thread::spawn(move || unsafe {
                worker.worker_thread()
            }));
        }
        result
    }

    // FIXME: lifetime guarantees
    // Somehow tell the type system that this job must live until completion
    pub fn enqueue(&self, job: &impl AsJob) -> Result<JobHandle, PushError<()>> {
        let job = Job::new(job);
        self.enqueue_job(job)
    }

    fn enqueue_job(&self, job: Job) -> Result<JobHandle, PushError<()>> {
        let res = job.as_handle();
        THREAD_INDEX
            .with(|id| unsafe {
                let id = *id.get();
                self.queues[id].push(job)
            })
            .map(move |_| res)
            .map_err(|err| match err {
                PushError::Full(_) => PushError::Full(()),
            })
    }

    pub fn enqueue_graph(&self, graph: JobGraph) -> Result<(), PushError<()>> {
        for job in graph.build() {
            self.enqueue_job(job)?;
        }
        // TODO: return a handle
        Ok(())
    }

    pub fn wait(&self, job: JobHandle) {
        while !job.is_done() {
            // TODO: use the [id] executor
            THREAD_INDEX.with(|id| unsafe {
                let id = *id.get();
                match self.queues[id].pop() {
                    Ok(mut j) => {
                        j.execute();
                    }
                    Err(_) => {
                        todo!()
                    }
                }
            });
        }
    }
}

lazy_static::lazy_static!(
    static ref JOB_POOL: Pin<Box<JobPool>> = unsafe {
        let conc = std::thread::available_parallelism().unwrap_or(NonZeroUsize::new_unchecked(1));
        JobPool::new(conc)
    };
);

type Todos = Arc<AtomicIsize>;

pub struct JobHandle {
    tasks_left: Todos,
}

impl JobHandle {
    pub fn is_done(&self) -> bool {
        let left = self.tasks_left.load(Ordering::Relaxed);
        debug_assert!(left >= 0);
        left <= 0
    }
}

pub trait AsJob {
    unsafe fn execute(this: *const ());
}

// Executor should deal in jobs
// The public API should be job graphs
struct Job {
    tasks_left: Todos,
    children: SmallVec<[Todos; 4]>,
    func: unsafe fn(*const ()),
    data: *const (),
}

unsafe impl Send for Job {}

impl Job {
    pub fn new<T: AsJob>(data: &T) -> Self {
        Self {
            tasks_left: Todos::new(1.into()),
            children: SmallVec::new(),
            func: T::execute,
            data: (data as *const T).cast(),
        }
    }

    pub fn execute(&mut self) {
        unsafe {
            debug_assert!(!self.data.is_null());
            (self.func)(self.data);
            self.data = std::ptr::null();
        }
        for mut dep in self.children.drain(..) {
            dep.fetch_sub(1, Ordering::Relaxed);
        }
    }

    pub fn done(&self) -> bool {
        let left = self.tasks_left.load(Ordering::Relaxed);
        debug_assert!(left >= 0);
        left <= 0
    }

    pub fn add_child(&mut self, child: &Job) {
        self.children.push(Arc::clone(&child.tasks_left));
        if self.done() {
            child.tasks_left.fetch_sub(1, Ordering::Relaxed);
        }
    }

    fn as_handle(&self) -> JobHandle {
        JobHandle {
            tasks_left: Arc::clone(&self.tasks_left),
        }
    }
}

pub struct InlineJob<F> {
    inner: F,
}

impl<F: Fn()> AsJob for InlineJob<F> {
    unsafe fn execute(this: *const ()) {
        let this: *const Self = this.cast();
        let this = &*this;
        (this.inner)();
    }
}

/// FIXME: should have some empty job that depends on all other jobs in the graph
/// Return a handle to this job when enqueue_graph is called
pub struct JobGraph {
    jobs: Vec<Job>,
}

impl JobGraph {
    pub fn new() -> Self {
        Self {
            jobs: Vec::with_capacity(128),
        }
    }

    pub fn add_job(&mut self, j: &impl AsJob) -> usize {
        let i = self.jobs.len();
        self.jobs.push(Job::new(j));
        i
    }

    pub fn add_child(&mut self, parent: usize, child: usize) {
        debug_assert_ne!(parent, child);
        let child = Arc::clone(&self.jobs[child].tasks_left);
        self.jobs[parent].children.push(child);
    }

    fn build(self) -> impl Iterator<Item = Job> {
        self.jobs.into_iter()
    }
}
