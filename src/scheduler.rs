use std::ptr::NonNull;

use smallvec::SmallVec;

use crate::{
    job_system::JobGraph,
    query::QueryProperties,
    systems::{ErasedSystem, SystemJob, SystemStage},
    World,
};

#[derive(Debug, Clone)]
pub struct Schedule {
    parents: Vec<SmallVec<[usize; 8]>>,
}

impl Schedule {
    pub fn from_stage(stage: &SystemStage) -> Self {
        let systems = match &stage.systems {
            crate::systems::StageSystems::Serial(_v) => {
                // trivial schedule
                return Self { parents: vec![] };
            }
            crate::systems::StageSystems::Parallel(s) => s,
        };
        Self::from_systems(systems)
    }

    pub fn from_systems<T>(systems: &[ErasedSystem<T>]) -> Self {
        let mut res = Self {
            parents: Vec::with_capacity(systems.len()),
        };
        if systems.is_empty() {
            return res;
        }
        // TODO:
        // - what about explicit ordering?
        // - forbid circular deps

        let mut history = vec![QueryProperties::from_system(&systems[0].descriptor)];
        history.reserve(systems.len() - 1);
        res.parents.push(Default::default());
        for i in 1..systems.len() {
            let sys = &systems[i];
            let props = QueryProperties::from_system(&sys.descriptor);
            res.parents.push(Default::default());

            for j in 0..i {
                if !props.is_disjoint(&history[j]) {
                    res.parents[i].push(j);
                }
            }
            history.push(props);
        }
        res
    }

    pub fn jobs<'a, T>(
        &self,
        stage: &[ErasedSystem<'a, T>],
        world: &World,
    ) -> JobGraph<SystemJob<'a, T>> {
        debug_assert_eq!(stage.len(), self.parents.len());

        let mut graph = JobGraph::new(
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
                graph.add_child(*j, i);
            }
        }

        graph
    }
}
