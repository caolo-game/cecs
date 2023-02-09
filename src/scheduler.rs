use std::{alloc::System, any::TypeId, collections::HashMap, ptr::NonNull};

use smallvec::SmallVec;

use crate::{
    job_system::JobGraph,
    query::QueryProperties,
    systems::{self, ErasedSystem, SystemJob, SystemStage},
    World,
};

pub type Schedule = Vec<Vec<usize>>;

pub struct ScheduleV2 {
    parents: Vec<SmallVec<[usize; 8]>>,
}

impl ScheduleV2 {
    pub fn from_stage<T>(systems: &[ErasedSystem<T>]) -> Self {
        let mut res = Self {
            parents: Vec::with_capacity(systems.len()),
        };
        if systems.is_empty() {
            return res;
        }
        // TODO:
        // - what about explicit ordering?
        // - forbid circular deps

        let mut history = vec![QueryProperties {
            exclusive: (systems[0].descriptor.exclusive)(),
            comp_mut: (systems[0].descriptor.components_mut)(),
            comp_const: (systems[0].descriptor.components_const)(),
            res_mut: (systems[0].descriptor.resources_mut)(),
            res_const: (systems[0].descriptor.resources_const)(),
        }];
        history.reserve(systems.len() - 1);
        res.parents.push(Default::default());
        for i in 1..systems.len() {
            let sys = &systems[i];
            let props = QueryProperties {
                exclusive: (sys.descriptor.exclusive)(),
                comp_mut: (sys.descriptor.components_mut)(),
                res_mut: (sys.descriptor.resources_mut)(),
                comp_const: (sys.descriptor.components_const)(),
                res_const: (sys.descriptor.resources_const)(),
            };
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

/// Return list of systems that must run sequentially
/// All sublist may run in parallel
pub fn schedule(stage: &SystemStage) -> Schedule {
    if stage.systems.is_empty() {
        return vec![];
    }

    let systems = match &stage.systems {
        crate::systems::StageSystems::Serial(v) => {
            // trivial schedule
            return vec![(0..v.len()).collect()];
        }
        crate::systems::StageSystems::Parallel(s) => s,
    };

    let mut result = vec![vec![0]];
    let mut history = vec![QueryProperties {
        exclusive: (systems[0].descriptor.exclusive)(),
        comp_mut: (systems[0].descriptor.components_mut)(),
        comp_const: (systems[0].descriptor.components_const)(),
        res_mut: (systems[0].descriptor.resources_mut)(),
        res_const: (systems[0].descriptor.resources_const)(),
    }];

    'systems: for (sys_index, sys) in systems.iter().enumerate().skip(1) {
        let props = QueryProperties {
            exclusive: (sys.descriptor.exclusive)(),
            comp_mut: (sys.descriptor.components_mut)(),
            res_mut: (sys.descriptor.resources_mut)(),
            comp_const: (sys.descriptor.components_const)(),
            res_const: (sys.descriptor.resources_const)(),
        };

        // try to find an existing group this system may run with, in parallel
        // if it fails then we add a new group
        for i in 0..result.len() {
            if props.is_disjoint(&history[i]) {
                result[i].push(sys_index);
                history[i].extend(props);
                continue 'systems;
            }
        }

        result.push(vec![sys_index]);
        history.push(props);
    }
    result
}

#[cfg(test)]
mod tests {

    use crate::{commands::Commands, prelude::Query, world_access::WorldAccess};

    use super::*;

    #[test]
    fn basic_schedule_with_query_test() {
        fn system_0(_cmd: Commands, _q: Query<&i32>) {}
        fn system_1(_q: Query<&i32>, _: Query<&u32>) {}
        fn system_2(_q: Query<(&mut i32, &u32)>, _: Query<&u32>, _: Query<&String>) {}
        fn system_3(_q: Query<&mut i32>) {}
        fn system_4(_q: Query<&String>) {}

        let stage = SystemStage::parallel("many_systems")
            .with_system(system_0)
            .with_system(system_1)
            .with_system(system_2)
            .with_system(system_3)
            .with_system(system_4);

        let schedule = schedule(&stage);

        assert_eq!(schedule.len(), 3);

        // TODO: this is a bit flaky
        assert_eq!(schedule, vec![vec![0, 1, 4], vec![2], vec![3]]);
    }

    #[test]
    fn exclusive_systems_get_their_own_group_test() {
        fn system_0(_cmd: Commands, _q: Query<&i32>) {}
        fn system_1(_q: Query<&i32>, _: Query<&u32>) {}
        fn system_2(_q: Query<(&mut i32, &u32)>, _: Query<&u32>, _: Query<&String>) {}
        fn system_3(_q: Query<&mut i32>) {}
        fn system_4(_q: Query<&String>) {}
        fn system_5(_w: WorldAccess) {}

        let stage = SystemStage::parallel("many_systems")
            .with_system(system_0)
            .with_system(system_5)
            .with_system(system_1)
            .with_system(system_2)
            .with_system(system_3)
            .with_system(system_4);

        let schedule = schedule(&stage);

        assert_eq!(schedule, [vec![0, 2, 5], vec![1], vec![3], vec![4]]);
    }
}
