#[cfg(test)]
mod tests;

use std::{any::TypeId, borrow::Cow, collections::HashSet, rc::Rc};

use crate::{query::WorldQuery, World};

pub type InnerSystem<'a, R> = dyn Fn(&'a World, usize) -> R + 'a;
pub type ShouldRunSystem<'a> = InnerSystem<'a, bool>;

type SystemStorage<T> = smallvec::SmallVec<[T; 4]>;

#[derive(Clone)]
pub struct SystemStage<'a> {
    pub name: Cow<'a, str>,
    pub should_run: SystemStorage<ErasedSystem<'a, bool>>,
    pub systems: StageSystems<'a>,
}

impl Default for SystemStage<'_> {
    fn default() -> Self {
        Self {
            name: "<default-empty-stage>".into(),
            should_run: smallvec::smallvec![],
            systems: StageSystems::Serial(smallvec::smallvec![]),
        }
    }
}

#[derive(Clone)]
pub enum StageSystems<'a> {
    Serial(SystemStorage<ErasedSystem<'a, ()>>),
    #[cfg(feature = "parallel")]
    Parallel(SystemStorage<ErasedSystem<'a, ()>>),
}

impl<'a> StageSystems<'a> {
    pub fn push(&mut self, sys: ErasedSystem<'a, ()>) {
        match self {
            StageSystems::Serial(v) => v.push(sys),
            #[cfg(feature = "parallel")]
            StageSystems::Parallel(v) => v.push(sys),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            StageSystems::Serial(v) => v.len(),
            #[cfg(feature = "parallel")]
            StageSystems::Parallel(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_slice(&self) -> &[ErasedSystem<'a, ()>] {
        match self {
            StageSystems::Serial(v) => v.as_slice(),
            #[cfg(feature = "parallel")]
            StageSystems::Parallel(v) => v.as_slice(),
        }
    }

    /// Returns `true` if the stage systems is [`Serial`].
    ///
    /// [`Serial`]: StageSystems::Serial
    pub fn is_serial(&self) -> bool {
        matches!(self, Self::Serial(..))
    }

    /// Returns `true` if the stage systems is [`Parallel`].
    ///
    /// [`Parallel`]: StageSystems::Parallel
    #[cfg(feature = "parallel")]
    pub fn is_parallel(&self) -> bool {
        matches!(self, Self::Parallel(..))
    }

    #[cfg(not(feature = "parallel"))]
    pub fn is_parallel(&self) -> bool {
        false
    }
}

impl<'a> SystemStage<'a> {
    pub fn serial<'b: 'a, N: Into<Cow<'b, str>>>(name: N) -> Self {
        Self {
            name: name.into(),
            should_run: SystemStorage::with_capacity(1),
            systems: StageSystems::Serial(SystemStorage::with_capacity(4)),
        }
    }

    /// If `feature=parallel` is disabled, then this created a `serial` SystemStage
    pub fn parallel<'b: 'a, N: Into<Cow<'b, str>>>(name: N) -> Self {
        Self {
            name: name.into(),
            should_run: SystemStorage::with_capacity(1),
            #[cfg(feature = "parallel")]
            systems: StageSystems::Parallel(SystemStorage::with_capacity(4)),
            #[cfg(not(feature = "parallel"))]
            systems: StageSystems::Serial(SystemStorage::with_capacity(4)),
        }
    }

    /// Multiple should_runs will be executed serially, and "and'ed" together
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
        let commands_index;
        #[cfg(feature = "parallel")]
        {
            commands_index = self.systems.len();
        }
        #[cfg(not(feature = "parallel"))]
        {
            commands_index = 0;
        }

        let descriptor = Rc::new(system.descriptor());
        let system = ErasedSystem {
            execute: (descriptor.factory)(),
            commands_index,
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
    pub name: Cow<'a, str>,
    pub components_mut: Box<dyn 'a + Fn() -> HashSet<TypeId>>,
    pub resources_mut: Box<dyn 'a + Fn() -> HashSet<TypeId>>,
    pub components_const: Box<dyn 'a + Fn() -> HashSet<TypeId>>,
    pub resources_const: Box<dyn 'a + Fn() -> HashSet<TypeId>>,
    pub exclusive: Box<dyn 'a + Fn() -> bool>,
    /// produce a system
    pub factory: Box<dyn 'a + Fn() -> Box<InnerSystem<'a, R>>>,
}

pub struct ErasedSystem<'a, R> {
    pub(crate) commands_index: usize,
    pub(crate) execute: Box<InnerSystem<'a, R>>,
    pub(crate) descriptor: Rc<SystemDescriptor<'a, R>>,
}

impl<'a, R> From<SystemDescriptor<'a, R>> for ErasedSystem<'a, R> {
    fn from(system: SystemDescriptor<'a, R>) -> Self {
        let descriptor = Rc::new(system);
        ErasedSystem {
            execute: (descriptor.factory)(),
            commands_index: 0,
            descriptor,
        }
    }
}

unsafe impl<R> Send for ErasedSystem<'_, R> {}
unsafe impl<R> Sync for ErasedSystem<'_, R> {}

impl<'a, R> Clone for ErasedSystem<'a, R> {
    fn clone(&self) -> Self {
        Self {
            commands_index: self.commands_index,
            execute: (self.descriptor.factory)(),
            descriptor: self.descriptor.clone(),
        }
    }
}

pub struct Piped<'a, R1, R2> {
    lhs: SystemDescriptor<'a, R1>,
    rhs: SystemDescriptor<'a, R2>,
}

impl<'a> IntoSystem<'a, (), ()> for Piped<'a, (), ()>
where
    Self: 'a,
{
    fn descriptor(self) -> SystemDescriptor<'a, ()> {
        let name = Cow::Owned(format!("{} | {}", self.lhs.name, self.rhs.name));
        let components_mut = {
            let lhs = self.lhs.components_mut;
            let rhs = self.rhs.components_mut;
            Box::new(move || {
                let mut result = (lhs)();
                result.extend((rhs)().into_iter());
                result
            })
        };
        let components_const = {
            let lhs = self.lhs.components_const;
            let rhs = self.rhs.components_const;
            Box::new(move || {
                let mut result = (lhs)();
                result.extend((rhs)().into_iter());
                result
            })
        };
        let resources_mut = {
            let lhs = self.lhs.resources_mut;
            let rhs = self.rhs.resources_mut;
            Box::new(move || {
                let mut result = (lhs)();
                result.extend((rhs)().into_iter());
                result
            })
        };
        let resources_const = {
            let lhs = self.lhs.resources_const;
            let rhs = self.rhs.resources_const;
            Box::new(move || {
                let mut result = (lhs)();
                result.extend((rhs)().into_iter());
                result
            })
        };
        let exclusive = {
            let lhs = self.lhs.exclusive;
            let rhs = self.rhs.exclusive;
            Box::new(move || ((lhs)() || (rhs)()))
        };
        let factory: Box<dyn Fn() -> Box<InnerSystem<'a, ()>>> = {
            let lfactory = self.lhs.factory;
            let rfactory = self.rhs.factory;

            Box::new(move || {
                let lhs = (lfactory)();
                let rhs = (rfactory)();
                Box::new(move |world: &'a World, i| {
                    (lhs)(world, i);
                    (rhs)(world, i);
                })
            })
        };
        SystemDescriptor {
            name,
            factory,
            components_mut,
            resources_mut,
            components_const,
            resources_const,
            exclusive,
        }
    }
}

pub trait IntoSystem<'a, Param, R> {
    fn descriptor(self) -> SystemDescriptor<'a, R>;
}

pub trait Pipe<'a, P1, P2, Rhs> {
    type Output;

    fn pipe(self, rhs: Rhs) -> Self::Output;
}

impl<'a, P1, P2, Lhs, Rhs> Pipe<'a, P1, P2, Rhs> for Lhs
where
    Lhs: IntoSystem<'a, P1, ()>,
    Rhs: IntoSystem<'a, P2, ()>,
{
    type Output = Piped<'a, (), ()>;

    fn pipe(self, rhs: Rhs) -> Self::Output {
        let lhs = self.descriptor();
        let rhs = rhs.descriptor();
        Piped { lhs, rhs }
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
                        Box::new(move |_world: &'a World, _commands_index| {
                            (self)(
                                $(<$t>::new(_world, _commands_index),)*
                            )
                        })
                    });
                SystemDescriptor {
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
                    factory,
                }
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
