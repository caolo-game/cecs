use rustc_hash::FxHashMap;
use std::ptr::NonNull;

use smallvec::SmallVec;

use crate::{
    job_system::HomogeneousJobGraph,
    query::QueryProperties,
    systems::{sort_systems, ErasedSystem, SystemJob, SystemStage},
    World,
};

#[derive(Debug, Clone)]
pub struct Schedule {
    parents: Vec<SmallVec<[usize; 8]>>,
}

impl Schedule {
    pub fn from_stage(stage: &mut SystemStage) -> Self {
        Self::from_systems(&mut stage.systems)
    }

    pub fn from_systems<T>(systems: &mut [ErasedSystem<T>]) -> Self {
        let mut res = Self {
            parents: Vec::with_capacity(systems.len()),
        };
        if systems.is_empty() {
            return res;
        }
        sort_systems(systems);
        let indices = systems
            .iter()
            .enumerate()
            .map(|(i, sys)| (sys.descriptor.id, i))
            .collect::<FxHashMap<_, _>>();

        let mut history = vec![QueryProperties::from_system(&systems[0].descriptor)];
        history.reserve(systems.len() - 1);
        res.parents.push(Default::default());
        debug_assert!(systems[0].descriptor.after.is_empty(), "bad ordering");
        for i in 1..systems.len() {
            let sys = &systems[i];
            let props = QueryProperties::from_system(&sys.descriptor);
            res.parents.push(Default::default());

            for j in 0..i {
                if !props.is_disjoint(&history[j]) {
                    res.parents[i].push(j);
                }
            }
            // explicit orderings
            for id in sys.descriptor.after.iter() {
                if let Some(j) = indices.get(id) {
                    res.parents[i].push(*j);
                }
            }

            history.push(props);
        }
        res
    }

    /// NOTE: `stage` must be the same as the one used to create this schedule
    pub fn jobs<'a, T>(
        &self,
        stage: &[ErasedSystem<'a, T>],
        world: &World,
    ) -> HomogeneousJobGraph<SystemJob<'a, T>> {
        debug_assert_eq!(stage.len(), self.parents.len());

        let mut graph = HomogeneousJobGraph::new(
            stage
                .iter()
                .map(|s| SystemJob {
                    // TODO: neither of these should move in memory
                    // so maybe memoize the vector and clone per tick?
                    world: NonNull::from(world),
                    sys: NonNull::from(s),
                })
                .collect::<Vec<_>>(),
        );

        for (i, parents) in self.parents.iter().enumerate() {
            for j in parents {
                graph.add_dependency(*j, i);
            }
        }

        graph
    }
}
