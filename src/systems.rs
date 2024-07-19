use std::{any::TypeId, collections::HashSet, ptr::NonNull, rc::Rc};

use cfg_if::cfg_if;

#[cfg(feature = "parallel")]
use crate::job_system::{AsJob, ExecutionState};

use crate::{query::WorldQuery, World};

pub type InnerSystem<'a, R> = dyn Fn(&'a World, usize) -> R + 'a;
pub type ShouldRunSystem<'a> = InnerSystem<'a, bool>;

type SystemStorage<T> = smallvec::SmallVec<[T; 4]>;

pub fn sort_systems<T>(sys: &mut [ErasedSystem<T>]) {
    sys.sort_unstable_by(|a, b| {
        let aid = a.descriptor.id;
        let bid = b.descriptor.id;
        assert!(
            !(a.descriptor.after.contains(&bid) && b.descriptor.after.contains(&aid)),
            "ambigous ordering between {} {}",
            a.descriptor.name,
            b.descriptor.name
        );
        if a.descriptor.after.contains(&bid) {
            std::cmp::Ordering::Greater
        } else if b.descriptor.after.contains(&aid) {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    });
}

#[derive(Clone)]
pub struct SystemStage<'a> {
    pub name: String,
    pub should_run: SystemStorage<ErasedSystem<'a, bool>>,
    pub systems: SystemStorage<ErasedSystem<'a, ()>>,
}

impl<'a> SystemStage<'a> {
    pub fn sort(&mut self) {
        sort_systems(&mut self.systems);
        sort_systems(&mut self.should_run);
    }

    pub fn new<N: Into<String>>(name: N) -> Self {
        Self {
            name: name.into(),
            should_run: SystemStorage::with_capacity(1),
            systems: SystemStorage::with_capacity(4),
        }
    }

    /// Multiple should_runs will be executed serially, and "and'ed" together in the same order as
    /// they were registered.
    pub fn with_should_run<S, P>(mut self, system: S) -> Self
    where
        S: IntoSystem<'a, P, bool>,
    {
        self.add_should_run(system);
        self
    }

    pub fn add_should_run<S, P>(&mut self, system: S) -> &mut Self
    where
        S: IntoSystem<'a, P, bool>,
    {
        let system = system.descriptor().into();
        self.should_run.push(system);
        self
    }

    pub fn add_system<S, P>(&mut self, system: S) -> &mut Self
    where
        S: IntoSystem<'a, P, ()>,
    {
        let system_idx;

        cfg_if!(
            if #[cfg(feature = "parallel")] {
                system_idx = self.systems.len();
            }
            else {
                system_idx = 0;
            }
        );

        let descriptor = Rc::new(system.descriptor());
        let system = ErasedSystem {
            execute: (descriptor.factory)(),
            system_idx,
            descriptor,
        };
        self.systems.push(system);
        self
    }

    pub fn with_system<S, P>(mut self, system: S) -> Self
    where
        S: IntoSystem<'a, P, ()>,
    {
        self.add_system(system);
        self
    }
}

#[allow(unused)] // with no feature=parallel most of this struct is unused
pub struct SystemDescriptor<'a, R> {
    pub name: String,
    pub id: TypeId,
    pub components_mut: Box<dyn 'a + Fn() -> HashSet<TypeId>>,
    pub resources_mut: Box<dyn 'a + Fn() -> HashSet<TypeId>>,
    pub components_const: Box<dyn 'a + Fn() -> HashSet<TypeId>>,
    pub resources_const: Box<dyn 'a + Fn() -> HashSet<TypeId>>,
    pub exclusive: Box<dyn 'a + Fn() -> bool>,
    /// produce a system
    pub factory: Box<dyn 'a + Fn() -> Box<InnerSystem<'a, R>>>,
    pub read_only: Box<dyn 'a + Fn() -> bool>,
    pub after: HashSet<TypeId>,
}

pub struct ErasedSystem<'a, R> {
    pub(crate) system_idx: usize,
    pub(crate) execute: Box<InnerSystem<'a, R>>,
    pub(crate) descriptor: Rc<SystemDescriptor<'a, R>>,
}

impl<'a, R> From<SystemDescriptor<'a, R>> for ErasedSystem<'a, R> {
    fn from(system: SystemDescriptor<'a, R>) -> Self {
        let descriptor = Rc::new(system);
        ErasedSystem {
            execute: (descriptor.factory)(),
            system_idx: 0,
            descriptor,
        }
    }
}

unsafe impl<R> Send for ErasedSystem<'_, R> {}
unsafe impl<R> Sync for ErasedSystem<'_, R> {}

impl<'a, R> Clone for ErasedSystem<'a, R> {
    fn clone(&self) -> Self {
        Self {
            system_idx: self.system_idx,
            execute: (self.descriptor.factory)(),
            descriptor: self.descriptor.clone(),
        }
    }
}

pub trait IntoSystem<'a, Param, R> {
    fn descriptor(self) -> SystemDescriptor<'a, R>;
    /// Order this system after another system
    fn after<'b, P, R2>(self, rhs: impl IntoSystem<'b, P, R2>) -> SystemDescriptor<'a, R>
    where
        Self: Sized,
    {
        let mut res = self.descriptor();
        let desc = rhs.descriptor();
        let id = desc.id;
        res.after.insert(id);
        res
    }
}

pub trait IntoOnceSystem<'a, Param, R> {
    fn into_once_system(self) -> impl FnOnce(&World, usize) -> R;
    fn descriptor() -> SystemDescriptor<'a, R>;
}

// helps chaining methods
impl<'a, R> IntoSystem<'a, (), R> for SystemDescriptor<'a, R> {
    fn descriptor(self) -> SystemDescriptor<'a, R> {
        self
    }
}

pub struct SystemJob<'a, R> {
    pub world: NonNull<World>,
    pub sys: NonNull<ErasedSystem<'a, R>>,
}

impl<'a, R> std::fmt::Debug for SystemJob<'a, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sys = unsafe { self.sys.as_ref().descriptor.name.as_str() };
        f.debug_struct("SystemJob").field("sys", &sys).finish()
    }
}

unsafe impl<'a, R> Send for SystemJob<'a, R> {}

#[cfg(feature = "parallel")]
impl<'a, R> AsJob for SystemJob<'a, R> {
    unsafe fn execute(this: *const ()) -> ExecutionState {
        let job: *const Self = this.cast();
        let job = &*job;
        let sys = job.sys.as_ref();
        let world = job.world.as_ref();

        #[cfg(feature = "tracing")]
        let _e = tracing::trace_span!("system", name = sys.descriptor.name.as_str()).entered();

        (sys.execute)(world, sys.system_idx);
        ExecutionState::Done
    }
}

macro_rules! impl_intosys_fn {
    ($($t: ident),* $(,)*) => {
        #[allow(unused_parens)]
        #[allow(unused_mut)]
        impl<'a, R, F, $($t: WorldQuery<'a> + 'static,)*>
            IntoSystem<'a, ($($t),*), R> for F
        where
            F: Fn($($t),*) -> R + 'static + Copy,
        {
            fn descriptor(self) -> SystemDescriptor<'a, R> {
                #[cfg(debug_assertions)]
                {
                    let mut _props = crate::query::QueryProperties::default();
                    // assert queries
                    $(
                        let p = crate::query::ensure_query_valid::<$t>();
                        assert!(p.is_disjoint(&_props) || (p.exclusive && _props.is_empty())
                                , "system {} has incompatible queries!", std::any::type_name::<F>());
                        _props.extend(p);
                    )*
                }
                let factory: Box<dyn Fn()-> Box<InnerSystem<'a, R>>>
                    = Box::new(move || {
                        Box::new(move |_world: &'a World, _system_idx| {
                            (self)(
                                $(<$t>::new(_world, _system_idx),)*
                            )
                        })
                    });
                SystemDescriptor {
                    id: TypeId::of::<F>(),
                    name: std::any::type_name::<F>().into(),
                    components_mut:Box::new( || {
                        let mut res = HashSet::new();
                        $(<$t>::components_mut(&mut res);)*
                        res
                    }),
                    resources_mut:Box::new( || {
                        let mut res = HashSet::new();
                        $(<$t>::resources_mut(&mut res);)*
                        res
                    }),
                    components_const:Box::new( || {
                        let mut res = HashSet::new();
                        $(<$t>::components_const(&mut res);)*
                        res
                    }),
                    resources_const:Box::new( || {
                        let mut res = HashSet::new();
                        $(<$t>::resources_const(&mut res);)*
                        res
                    }),
                    exclusive:Box::new( || {
                        // empty system is not exclusive
                        false $(|| <$t>::exclusive())*
                    }),
                    read_only:Box::new( || {
                        // empty system is read_only
                        true $(&& <$t>::read_only())*
                    }),
                    factory,
                    after: HashSet::new(),
                }
            }
        }

        #[allow(unused_parens)]
        #[allow(unused_mut)]
        impl<'a, R: 'static, F, $($t: WorldQuery<'a> + 'static,)*>
            IntoOnceSystem<'a, ($($t),*), R> for F
        where
            F: FnOnce($($t),*) -> R,
        {
            fn into_once_system(self) -> impl FnOnce(&World, usize) -> R {
                move |_world: &World, _system_idx| {
                    (self)(
                        $( unsafe { <$t>::new(std::mem::transmute(_world), _system_idx)},)*
                    )
                }
            }
            fn descriptor() -> SystemDescriptor<'a, R> {
                let dummy: fn($($t),*) -> R = |$(_:$t),*| unreachable!();
                dummy.descriptor()
            }
        }
    };
}

impl_intosys_fn!();
impl_intosys_fn!(Q0);
impl_intosys_fn!(Q0, Q1);
impl_intosys_fn!(Q0, Q1, Q2);
impl_intosys_fn!(Q0, Q1, Q2, Q3);
impl_intosys_fn!(Q0, Q1, Q2, Q3, Q4);
impl_intosys_fn!(Q0, Q1, Q2, Q3, Q4, Q5);
impl_intosys_fn!(Q0, Q1, Q2, Q3, Q4, Q5, Q6);
impl_intosys_fn!(Q0, Q1, Q2, Q3, Q4, Q5, Q6, Q7);
impl_intosys_fn!(Q0, Q1, Q2, Q3, Q4, Q5, Q6, Q7, Q8);
impl_intosys_fn!(Q0, Q1, Q2, Q3, Q4, Q5, Q6, Q7, Q8, Q9);
impl_intosys_fn!(Q0, Q1, Q2, Q3, Q4, Q5, Q6, Q7, Q8, Q9, Q10);
impl_intosys_fn!(Q0, Q1, Q2, Q3, Q4, Q5, Q6, Q7, Q8, Q9, Q10, Q11);
impl_intosys_fn!(Q0, Q1, Q2, Q3, Q4, Q5, Q6, Q7, Q8, Q9, Q10, Q11, Q12);
impl_intosys_fn!(Q0, Q1, Q2, Q3, Q4, Q5, Q6, Q7, Q8, Q9, Q10, Q11, Q12, Q13);
impl_intosys_fn!(Q0, Q1, Q2, Q3, Q4, Q5, Q6, Q7, Q8, Q9, Q10, Q11, Q12, Q13, Q14);
impl_intosys_fn!(Q0, Q1, Q2, Q3, Q4, Q5, Q6, Q7, Q8, Q9, Q10, Q11, Q12, Q13, Q14, Q15);
impl_intosys_fn!(Q0, Q1, Q2, Q3, Q4, Q5, Q6, Q7, Q8, Q9, Q10, Q11, Q12, Q13, Q14, Q15, Q16);
impl_intosys_fn!(Q0, Q1, Q2, Q3, Q4, Q5, Q6, Q7, Q8, Q9, Q10, Q11, Q12, Q13, Q14, Q15, Q16, Q17);
impl_intosys_fn!(
    Q0, Q1, Q2, Q3, Q4, Q5, Q6, Q7, Q8, Q9, Q10, Q11, Q12, Q13, Q14, Q15, Q16, Q17, Q18
);
impl_intosys_fn!(
    Q0, Q1, Q2, Q3, Q4, Q5, Q6, Q7, Q8, Q9, Q10, Q11, Q12, Q13, Q14, Q15, Q16, Q17, Q18, Q19
);
impl_intosys_fn!(
    Q0, Q1, Q2, Q3, Q4, Q5, Q6, Q7, Q8, Q9, Q10, Q11, Q12, Q13, Q14, Q15, Q16, Q17, Q18, Q19, Q20
);
