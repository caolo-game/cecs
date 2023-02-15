use parking_lot::{Condvar, Mutex, ReentrantMutex};
use std::{
    cell::UnsafeCell,
    marker::PhantomData,
    num::NonZeroUsize,
    pin::Pin,
    ptr::NonNull,
    sync::{
        atomic::{AtomicIsize, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::Duration,
};

// TODO: Job Allocator
type WorkerQueue = crossbeam_deque::Worker<Job>;
type Sleep = Arc<(Mutex<bool>, Condvar)>;

#[derive(Clone)]
pub struct JobPool {
    inner: Arc<Inner>,
}

impl JobPool {
    pub fn join(&self, a: impl FnOnce() + Send, b: impl FnOnce() + Send) {
        let c = InlineJob::new(|| {});
        let a = InlineJob::new(a);
        let b = InlineJob::new(b);

        unsafe {
            let c = c.as_job();
            let mut a = a.as_job();
            a.add_child(&c);
            self.enqueue_job(a);
            let mut b = b.as_job();
            b.add_child(&c);
            self.enqueue_job(b);
            let handle = self.enqueue_job(c);
            self.wait(handle);
        }
    }

    pub fn scope(&self, f: impl FnOnce(Scope)) {
        let scope = Scope {
            pool: self,
            root: Default::default(),
        };
        f(scope);
    }

    pub(crate) fn enqueue_job(&self, job: Job) -> JobHandle {
        unsafe {
            let res = job.as_handle();
            with_thread_index(|id| {
                #[cfg(feature = "tracing")]
                tracing::trace!(
                    id = id,
                    data = tracing::field::debug(job.data),
                    "Enqueueing job"
                );
                if job.ready() {
                    self.inner.runnable_queues[id].push(job);
                    // wake up a worker
                    self.inner.sleep.1.notify_one();
                } else {
                    (&mut *self.inner.wait_lists[id].get()).push(job);
                }
                // wake up a worker
                self.inner.sleep.1.notify_one();
                res
            })
        }
    }

    pub fn enqueue_graph<T: Send>(&self, mut graph: JobGraph<T>) -> JobHandle {
        let data = graph._data;
        let root = BoxedJob::new(move || {
            // take ownership of the data
            // dropping it when the final, root, job is executed
            let _d = data;
        });
        let root = unsafe { root.into_job() };
        for job in graph.jobs.drain(..) {
            let mut job = job.into_inner();
            job.add_child(&root);
            self.enqueue_job(job);
        }
        self.enqueue_job(root)
    }

    pub fn wait(&self, job: JobHandle) {
        unsafe {
            with_thread_index(|id| {
                let q = &*self.inner.runnable_queues as *const _;
                let wait_list = NonNull::new(self.inner.wait_lists[id].get()).unwrap();
                let mut tmp_exec =
                    Executor::new(id, QueueArray(q), wait_list, Arc::clone(&self.inner.sleep));
                while !job.done() {
                    if tmp_exec.run_once().is_err() {
                        // make sure other threads keep cleaning their wait lists
                        self.inner.sleep.1.notify_all();
                        // busy wait so `wait` returns asap
                        std::thread::yield_now();
                    }
                }
            });
        }
    }
}

impl Default for JobPool {
    fn default() -> Self {
        unsafe {
            let conc =
                std::thread::available_parallelism().unwrap_or(NonZeroUsize::new_unchecked(1));
            let inner = Arc::new(Inner::new(conc));
            Self { inner }
        }
    }
}

struct Inner {
    threads: Vec<JoinHandle<()>>,
    runnable_queues: Pin<Box<[WorkerQueue]>>,
    /// threads may only access their own waiting_queues
    wait_lists: Pin<Box<[UnsafeCell<Vec<Job>>]>>,
    sleep: Sleep,
    /// JobPool must not be `Send` because the owning thread is initialized as thread 0
    _m: PhantomData<*mut ()>,
}

unsafe impl Send for Inner {}
unsafe impl Sync for Inner {}

impl Drop for Inner {
    fn drop(&mut self) {
        {
            *self.sleep.0.lock() = true;
        }
        for j in self.threads.drain(..) {
            j.join().unwrap_or(());
        }
    }
}

thread_local! {
    static THREAD_INDEX: UnsafeCell<usize> = UnsafeCell::new(0);
}

static ZERO_LOCK: ReentrantMutex<()> = ReentrantMutex::new(());

fn with_thread_index<R>(f: impl FnOnce(usize) -> R) -> R {
    let mut id = unsafe { THREAD_INDEX.with(|id| *id.get()) };
    // unitinialized threads can use the main thread's queue
    let _lock = if id == 0 {
        id = 0;
        Some(ZERO_LOCK.lock())
    } else {
        None
    };
    f(id)
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
                    let mut l = lock.lock();
                    cv.wait_for(&mut l, Duration::from_millis(5));
                    if *l {
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
        #[cfg(feature = "tracing")]
        tracing::trace!(id = id, "Running executor");
        if let Some(mut job) = queues[id].pop() {
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
        let qlen = queues.len();
        let next_id = move |i: &mut Self| {
            i.steal_id = (i.steal_id + 1) % qlen;
        };
        loop {
            let mut retry = false;
            for _ in 0..queues.len() {
                if self.steal_id == id {
                    next_id(self);
                    continue;
                }
                // TODO: retain the stealer?
                let stealer = queues[self.steal_id].stealer();
                match stealer.steal_batch(&queues[self.id]) {
                    crossbeam_deque::Steal::Success(_) => {
                        // do not increment the id if steal succeeds
                        // next time this executor is out of jobs try stealing from the same queue
                        // first
                        return Ok(());
                    }
                    crossbeam_deque::Steal::Empty => {}
                    crossbeam_deque::Steal::Retry => {
                        retry = true;
                    }
                }
                next_id(self);
            }
            if !retry {
                break;
            }
        }
        // if stealing fails too try to promote waiting items
        let wait_list = self.wait_list.as_mut();
        let mut promoted = false;
        for i in (0..wait_list.len()).rev() {
            debug_assert!(!wait_list[i].done());
            if wait_list[i].ready() {
                let job = wait_list.swap_remove(i);
                #[cfg(feature = "tracing")]
                let data = job.data;
                queues[id].push(job);
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

impl Inner {
    pub fn new(workers: NonZeroUsize) -> Self {
        let workers = workers.get();
        let mut queues = Vec::with_capacity(workers);
        for _ in 0..workers {
            queues.push(WorkerQueue::new_fifo());
        }
        let queues = Pin::new(queues.into_boxed_slice());
        let q = &*queues as *const _;
        let sleep = Arc::default();
        let mut result = Self {
            sleep: Arc::clone(&sleep),
            runnable_queues: queues,
            threads: Vec::with_capacity(workers),
            wait_lists: Pin::new(
                (0..workers)
                    .map(|_| Default::default())
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
            _m: PhantomData,
        };
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
        result
    }
}

lazy_static::lazy_static!(
    pub static ref JOB_POOL: JobPool = {
        Default::default()
    };
);

type Todos = Arc<AtomicIsize>;

#[derive(Default, Debug, Clone)]
pub struct JobHandle {
    tasks_left: Todos,
}

impl JobHandle {
    pub fn done(&self) -> bool {
        let left = self.tasks_left.load(Ordering::Relaxed);
        debug_assert!(left >= 0, "{left}");
        left <= 0
    }
}

pub trait AsJob: Send {
    unsafe fn execute(instance: *const ());
}

// Executor should deal in jobs
// The public API should be job graphs
#[derive(Debug)]
pub(crate) struct Job {
    tasks_left: Todos,
    children: Vec<Todos>,
    func: unsafe fn(*const ()),
    data: *const (),
}

unsafe impl Send for Job {}

impl Job {
    /// # Safety
    ///
    /// Caller must ensure that `data` outlives the Job
    unsafe fn new<T: AsJob>(data: *const T) -> Self {
        Self {
            tasks_left: Todos::new(1.into()),
            children: Vec::new(),
            func: T::execute,
            data: data.cast(),
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

    pub fn add_child_handle(&mut self, child: &JobHandle) {
        debug_assert!(!self.done());
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
    inner: UnsafeCell<Option<F>>,
}

impl<F> InlineJob<F> {
    pub fn new(inner: F) -> Self {
        Self {
            inner: UnsafeCell::new(Some(inner)),
        }
    }

    /// # Safety caller must ensure that the instance outlives the job
    pub(crate) unsafe fn as_job(&self) -> Job
    where
        F: FnOnce() + Send,
    {
        Job::new(self)
    }
}

impl<F: FnOnce() + Send> AsJob for InlineJob<F> {
    unsafe fn execute(instance: *const ()) {
        let instance: *const Self = instance.cast();
        let instance = &*instance;
        let inner = (&mut *instance.inner.get()).take();
        (inner.unwrap())();
    }
}

pub struct BoxedJob<F> {
    inner: F,
}

impl<F> AsJob for BoxedJob<F>
where
    F: FnOnce() + Send,
{
    unsafe fn execute(instance: *const ()) {
        let instance: Box<Self> = Box::from_raw(instance.cast_mut().cast());
        (instance.inner)();
    }
}

impl<F> BoxedJob<F> {
    pub fn new(inner: F) -> Box<Self> {
        Box::new(Self { inner })
    }

    /// # Safety caller must ensure that the job is executed exactly once
    /// The job takes ownership of self
    pub(crate) unsafe fn into_job(self: Box<Self>) -> Job
    where
        F: FnOnce() + Send,
    {
        Job::new(Box::into_raw(self))
    }
}

pub struct Scope<'a> {
    pool: &'a JobPool,
    root: JobHandle,
}

impl<'a> Drop for Scope<'a> {
    fn drop(&mut self) {
        if !self.root.done() {
            self.pool.wait(self.root.clone());
        }
    }
}

impl<'a> Scope<'a> {
    pub fn new(pool: &'a JobPool) -> Self {
        Self {
            pool,
            root: Default::default(),
        }
    }

    pub fn spawn(&self, task: impl FnOnce(Scope<'a>) + Send) {
        let child_scope = Scope {
            pool: self.pool,
            root: Default::default(),
        };
        let job = BoxedJob::new(move || {
            task(child_scope);
        });
        unsafe {
            let mut job = job.into_job();
            job.add_child_handle(&self.root);
            self.pool.enqueue_job(job);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg_attr(feature = "tracing", tracing_test::traced_test)]
    fn join_test() {
        let pool = JobPool::default();

        let mut a = 0;
        let mut b = 0;

        pool.join(
            || {
                a += 1;
            },
            || {
                b += 1;
            },
        );

        assert_eq!(a, 1);
        assert_eq!(b, 1);
    }

    #[test]
    #[cfg_attr(feature = "tracing", tracing_test::traced_test)]
    fn scope_test() {
        let pool = JobPool::default();

        let a = AtomicIsize::new(0);
        let b = AtomicIsize::new(0);

        pool.scope(|s| {
            s.spawn(|_s| {
                a.fetch_add(1, Ordering::Relaxed);
            });
            s.spawn(|_s| {
                b.fetch_add(1, Ordering::Relaxed);
            });
        });

        assert_eq!(a.load(Ordering::Relaxed), 1);
        assert_eq!(b.load(Ordering::Relaxed), 1);
    }
}
