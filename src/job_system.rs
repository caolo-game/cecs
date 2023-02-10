use smallvec::SmallVec;
use std::{
    cell::UnsafeCell,
    num::NonZeroUsize,
    pin::Pin,
    ptr::NonNull,
    sync::{
        atomic::{AtomicIsize, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::JoinHandle,
    time::Duration,
};

use self::queue::PushError;

mod queue;

// TODO: Job Allocator
type WorkerQueue = queue::Queue<Job>;
type Sleep = Arc<(Mutex<bool>, Condvar)>;

pub struct JobPool {
    threads: Vec<JoinHandle<()>>,
    runnable_queues: Pin<Box<[WorkerQueue]>>,
    /// threads may only access their own waiting_queues
    wait_lists: Pin<Box<[UnsafeCell<Vec<Job>>]>>,
    sleep: Sleep,
}

unsafe impl Send for JobPool {}
unsafe impl Sync for JobPool {}

impl Drop for JobPool {
    fn drop(&mut self) {
        {
            *self.sleep.0.lock().unwrap() = true;
        }
        for j in self.threads.drain(..) {
            j.join().unwrap_or(());
        }
    }
}

thread_local! {
    pub static THREAD_INDEX: UnsafeCell<usize> = UnsafeCell::new(usize::MAX);
}

/// Context for a worker thread
struct Executor {
    id: usize,
    steal_id: usize,
    queues: QueueArray,
    wait_list: NonNull<Vec<Job>>,
    sleep: Sleep,
}

unsafe impl Send for Executor {}
unsafe impl Sync for Executor {}

impl Executor {
    fn new(id: usize, queues: QueueArray, wait_list: NonNull<Vec<Job>>, sleep: Sleep) -> Self {
        Self {
            id,
            steal_id: id,
            queues,
            wait_list,
            sleep,
        }
    }

    /// # Safety
    ///
    /// Caller must ensure that the thread is joined before the queues are destroyed
    unsafe fn worker_thread(&mut self) {
        THREAD_INDEX.with(move |tid| {
            *tid.get() = self.id;
            loop {
                if self.run_once().is_err() {
                    let (lock, cv) = &*self.sleep;
                    let res = cv
                        .wait_timeout(lock.lock().unwrap(), Duration::from_millis(5))
                        .unwrap();
                    if *res.0 {
                        break;
                    }
                }
            }
        });
    }

    /// # Safety
    ///
    /// Caller must ensure that the queues outlive run_once
    unsafe fn run_once(&mut self) -> Result<(), RunError> {
        let queues = &*self.queues.0;
        let id = self.id;
        if let Ok(mut job) = queues[id].pop() {
            #[cfg(feature = "tracing")]
            tracing::debug!(
                id = id,
                data = tracing::field::debug(job.data),
                "Executing job"
            );
            job.execute();
            return Ok(());
        }
        #[cfg(feature = "tracing")]
        tracing::debug!(id = id, "Pop failed");
        // if pop fails try to steal from another thread
        for _ in 0..queues.len() {
            self.steal_id = (self.steal_id + 1) % queues.len();
            if self.steal_id == id {
                continue;
            }
            match queues[id].steal(&queues[self.steal_id]) {
                Ok(_) => {
                    return Ok(());
                }
                Err(_) => {}
            }
        }
        // if stealing fails too try to promote waiting items
        let wait_list = self.wait_list.as_mut();
        let mut promoted = false;
        for i in (0..wait_list.len()).rev() {
            debug_assert!(!wait_list[i].done());
            if wait_list[i].ready() {
                let job = wait_list.swap_remove(i);
                let data = job.data;
                if let Err(err) = queues[id].push(job) {
                    match err {
                        PushError::Full(job) => {
                            wait_list.push(job);
                            break;
                        }
                    }
                }
                self.sleep.1.notify_one();
                promoted = true;

                #[cfg(feature = "tracing")]
                tracing::debug!(
                    id = id,
                    data = tracing::field::debug(data),
                    "Promoted job to runnable"
                );
            }
        }
        promoted.then_some(()).ok_or(RunError::StealFailed)
    }
}

#[derive(Debug, Clone)]
enum RunError {
    StealFailed,
}

struct QueueArray(*const [WorkerQueue]);
unsafe impl Send for QueueArray {}

impl JobPool {
    pub fn new(workers: NonZeroUsize) -> Pin<Box<Self>> {
        let capacity = NonZeroUsize::new(1 << 16).unwrap();
        let workers = workers.get();
        let mut queues = Vec::with_capacity(workers);
        for _ in 0..workers {
            queues.push(queue::Queue::new(capacity));
        }
        let queues = Pin::new(queues.into_boxed_slice());
        let q = &*queues as *const _;
        let sleep = Arc::default();
        let mut result = Box::pin(Self {
            sleep: Arc::clone(&sleep),
            runnable_queues: queues,
            threads: Vec::with_capacity(workers),
            wait_lists: Pin::new(
                (0..workers)
                    .map(|_| Default::default())
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
        });
        // the main thread is also used a worker on wait points
        // the index of the main thread is 0
        for i in 1..workers {
            let arr = QueueArray(q);
            let wait_list = NonNull::new(result.wait_lists[i].get()).unwrap();
            let mut worker = Executor::new(i, arr, wait_list, Arc::clone(&sleep));
            result.threads.push(
                std::thread::Builder::new()
                    .name(format!("cecs worker {i}"))
                    .spawn(move || unsafe { worker.worker_thread() })
                    .expect("Failed to create worker thread"),
            );
        }
        // initialize the current thread as thread 0
        THREAD_INDEX.with(|tid| unsafe {
            *tid.get() = 0;
        });
        result
    }

    /// # Safety
    ///
    /// Caller must ensure that `job` outlives the job completion
    pub unsafe fn enqueue(&self, job: &impl AsJob) -> JobHandle {
        let job = Job::new(job);
        self.enqueue_job(job)
    }

    fn enqueue_job(&self, job: Job) -> JobHandle {
        let res = job.as_handle();
        THREAD_INDEX.with(|id| unsafe {
            let id = *id.get();
            if job.ready() {
                let res = self.runnable_queues[id].push(job);
                match res {
                    Ok(_) => {}
                    Err(err) => match err {
                        PushError::Full(job) => {
                            #[cfg(feature = "tracing")]
                            tracing::debug!(
                                id = id,
                                data = tracing::field::debug(job.data),
                                "Job queue is full, pushing job into the waiting list"
                            );
                            (&mut *self.wait_lists[id].get()).push(job);
                        }
                    },
                }
                // wake up a worker
                self.sleep.1.notify_one();
            } else {
                (&mut *self.wait_lists[id].get()).push(job);
            }
            res
        })
    }

    pub fn execute_graph<T>(&self, mut graph: JobGraph<T>) {
        let root = InlineJob { inner: (|| {}) };
        let root = unsafe { Job::new(&root) };
        for job in graph.jobs.drain(..) {
            let mut job = job.into_inner();
            job.add_child(&root);
            self.enqueue_job(job);
        }
        let handle = self.enqueue_job(root);
        self.wait(handle);
        // TODO: return handle?
        // do consider that graph owns the data used by jobs and must outlive the execution
        drop(graph);
    }

    pub fn wait(&self, job: JobHandle) {
        THREAD_INDEX.with(|id| unsafe {
            let id = *id.get();
            let q = &*self.runnable_queues as *const _;
            let wait_list = NonNull::new(self.wait_lists[id].get()).unwrap();
            let mut tmp_exec = Executor::new(id, QueueArray(q), wait_list, Arc::clone(&self.sleep));
            while !job.is_done() {
                if tmp_exec.run_once().is_err() {
                    // make sure other threads keep cleaning their wait lists
                    self.sleep.1.notify_all();
                    // busy wait so `wait` returns asap
                    std::thread::yield_now();
                }
            }
        });
    }
}

impl Default for Pin<Box<JobPool>> {
    fn default() -> Self {
        unsafe {
            let conc =
                std::thread::available_parallelism().unwrap_or(NonZeroUsize::new_unchecked(1));
            JobPool::new(conc)
        }
    }
}

lazy_static::lazy_static!(
    pub static ref JOB_POOL: Pin<Box<JobPool>> = {
        Default::default()
    };
);

type Todos = Arc<AtomicIsize>;

#[derive(Debug, Clone)]
pub struct JobHandle {
    tasks_left: Todos,
}

impl JobHandle {
    pub fn is_done(&self) -> bool {
        let left = self.tasks_left.load(Ordering::Relaxed);
        debug_assert!(left >= 0, "{left}");
        left <= 0
    }
}

pub trait AsJob {
    unsafe fn execute(instance: *const ());
}

// Executor should deal in jobs
// The public API should be job graphs
#[derive(Debug)]
struct Job {
    tasks_left: Todos,
    children: SmallVec<[Todos; 4]>,
    func: unsafe fn(*const ()),
    data: *const (),
}

unsafe impl Send for Job {}

impl Job {
    /// # Safety
    ///
    /// Caller must ensure that `data` outlives the Job
    pub unsafe fn new<T: AsJob>(data: &T) -> Self {
        let data = (data as *const T).cast();
        Self {
            tasks_left: Todos::new(1.into()),
            children: SmallVec::new(),
            func: T::execute,
            data,
        }
    }

    pub fn execute(&mut self) {
        debug_assert!(self.ready());
        unsafe {
            (self.func)(self.data);
            self.data = std::ptr::null();
        }
        for dep in self.children.iter() {
            dep.fetch_sub(1, Ordering::Relaxed);
        }
        self.tasks_left.fetch_sub(1, Ordering::Release);
    }

    pub fn done(&self) -> bool {
        let left = self.tasks_left.load(Ordering::Relaxed);
        debug_assert!(left >= 0);
        left <= 0 && self.data.is_null()
    }

    pub fn ready(&self) -> bool {
        let left = self.tasks_left.load(Ordering::Relaxed);
        debug_assert!(left >= 0);
        left <= 1 && !self.data.is_null()
    }

    pub fn add_child(&mut self, child: &Job) {
        debug_assert!(!self.done());
        debug_assert!(!child.done());
        self.children.push(Arc::clone(&child.tasks_left));
        child.tasks_left.fetch_add(1, Ordering::Relaxed);
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
    unsafe fn execute(instance: *const ()) {
        let instance: *const Self = instance.cast();
        let instance = &*instance;
        (instance.inner)();
    }
}

pub struct JobGraph<T> {
    jobs: Vec<UnsafeCell<Job>>,
    _data: Vec<T>,
}

impl<T> JobGraph<T>
where
    T: AsJob,
{
    pub fn new(data: impl Into<Vec<T>>) -> Self {
        unsafe {
            let _data = data.into();
            let jobs = _data
                .iter()
                .map(|d| Job::new(d))
                .map(UnsafeCell::new)
                .collect();
            Self { _data, jobs }
        }
    }

    pub fn add_child(&mut self, parent: usize, child: usize) {
        debug_assert_ne!(parent, child);
        unsafe {
            (&mut *self.jobs[parent].get()).add_child(&*self.jobs[child].get());
        }
    }
}
