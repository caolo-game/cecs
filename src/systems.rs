use std::{any::TypeId, borrow::Cow, collections::HashSet, rc::Rc};

use crate::{query::WorldQuery, World};

pub type InnerSystem<'a, R> = dyn Fn(&'a World) -> R + 'a;
pub type ShouldRunSystem<'a> = InnerSystem<'a, bool>;

#[derive(Clone)]
pub struct SystemStage<'a> {
    pub name: Cow<'a, str>,
    pub should_run: Option<ErasedSystem<'a, bool>>,
    pub systems: Vec<ErasedSystem<'a, ()>>,
}

impl<'a> SystemStage<'a> {
    pub fn new<'b: 'a, N: Into<Cow<'b, str>>>(name: N) -> Self {
        Self {
            name: name.into(),
            should_run: None,
            systems: Vec::with_capacity(4),
        }
    }

    pub fn with_should_run<S, P>(mut self, system: S) -> Self
    where
        S: IntoSystem<'a, P, bool>,
    {
        self.should_run = Some(system.system());
        self
    }

    pub fn with_system<S, P>(mut self, system: S) -> Self
    where
        S: IntoSystem<'a, P, ()>,
    {
        self.systems.push(system.system());
        self
    }
}

pub struct ErasedSystem<'a, R> {
    pub name: Cow<'a, str>,
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
                        assert!(p.is_disjoint(&_props));
                        _props.extend(p);
                    )*
                }
                // TODO: assert that the queries are disjoint
                let factory: Rc<dyn Fn()-> Box<InnerSystem<'a, R>>>
                    = Rc::new(move || {
                        Box::new(move |_world: &'a World| {
                            (self)(
                                $(<$t>::new(_world),)*
                            )
                        })
                    });
                ErasedSystem {
                    name: std::any::type_name::<F>().into(),
                    execute: factory(),
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
