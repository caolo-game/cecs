use std::{any::TypeId, borrow::Cow, collections::HashSet, rc::Rc};

use crate::{query::WorldQuery, World};

pub type InnerSystem<'a, R> = dyn Fn(&'a World, usize) -> R + 'a;
pub type ShouldRunSystem<'a> = InnerSystem<'a, bool>;

#[derive(Clone)]
pub struct SystemStage<'a> {
    pub name: Cow<'a, str>,
    pub should_run: Vec<ErasedSystem<'a, bool>>,
    pub systems: StageSystems<'a>,
}

#[derive(Clone)]
pub enum StageSystems<'a> {
    Serial(Vec<ErasedSystem<'a, ()>>),
    #[cfg(feature = "parallel")]
    Parallel(Vec<ErasedSystem<'a, ()>>),
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
            should_run: Vec::with_capacity(1),
            systems: StageSystems::Serial(Vec::with_capacity(4)),
        }
    }

    /// If `feature=parallel` is disabled, then this created a `serial` SystemStage
    pub fn parallel<'b: 'a, N: Into<Cow<'b, str>>>(name: N) -> Self {
        Self {
            name: name.into(),
            should_run: Vec::with_capacity(1),
            #[cfg(feature = "parallel")]
            systems: StageSystems::Parallel(Vec::with_capacity(4)),
            #[cfg(not(feature = "parallel"))]
            systems: StageSystems::Serial(Vec::with_capacity(4)),
        }
    }

    /// Multiple should_runs will be executed serially, and "and'ed" together
    pub fn with_should_run<S, P>(mut self, system: S) -> Self
    where
        S: IntoSystem<'a, P, bool>,
    {
        self.should_run.push(system.system());
        self
    }

    pub fn with_system<S, P>(mut self, system: S) -> Self
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

        let mut system = system.system();
        system.commands_index = commands_index;
        self.systems.push(system);
        self
    }
}

pub struct ErasedSystem<'a, R> {
    pub name: Cow<'a, str>,
    pub commands_index: usize,
    pub(crate) execute: Box<InnerSystem<'a, R>>,
    pub(crate) components_mut: fn() -> HashSet<TypeId>,
    pub(crate) resources_mut: fn() -> HashSet<TypeId>,
    pub(crate) components_const: fn() -> HashSet<TypeId>,
    pub(crate) resources_const: fn() -> HashSet<TypeId>,
    factory: Rc<dyn Fn() -> Box<InnerSystem<'a, R>>>,
}

unsafe impl<R> Send for ErasedSystem<'_, R> {}
unsafe impl<R> Sync for ErasedSystem<'_, R> {}

impl<'a, R> Clone for ErasedSystem<'a, R> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            commands_index: self.commands_index.clone(),
            execute: (self.factory)(),
            components_mut: self.components_mut,
            resources_mut: self.resources_mut,
            components_const: self.components_const,
            resources_const: self.resources_const,
            factory: self.factory.clone(),
        }
    }
}

pub trait IntoSystem<'a, Param, R> {
    fn system(self) -> ErasedSystem<'a, R>;
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
            fn system(self) -> ErasedSystem<'a, R> {
                #[cfg(debug_assertions)]
                {
                    let mut _props = crate::query::QueryProperties::default();
                    // assert queries
                    $(
                        let p = crate::query::ensure_query_valid::<$t>();
                        assert!(p.is_disjoint(&_props), "system {} has incompatible queries!", std::any::type_name::<F>());
                        _props.extend(p);
                    )*
                }
                let factory: Rc<dyn Fn()-> Box<InnerSystem<'a, R>>>
                    = Rc::new(move || {
                        Box::new(move |_world: &'a World, _commands_index| {
                            (self)(
                                $(<$t>::new(_world, _commands_index),)*
                            )
                        })
                    });
                ErasedSystem {
                    name: std::any::type_name::<F>().into(),
                    execute: factory(),
                    commands_index: 0,
                    components_mut: || {
                        let mut res = HashSet::new();
                        $(<$t>::components_mut(&mut res);)*
                        res
                    },
                    resources_mut: || {
                        let mut res = HashSet::new();
                        $(<$t>::resources_mut(&mut res);)*
                        res
                    },
                    components_const: || {
                        let mut res = HashSet::new();
                        $(<$t>::components_const(&mut res);)*
                        res
                    },
                    resources_const: || {
                        let mut res = HashSet::new();
                        $(<$t>::resources_const(&mut res);)*
                        res
                    },
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
