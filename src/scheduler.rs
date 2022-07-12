use std::{any::TypeId, collections::HashSet};

use crate::systems::SystemStage;

pub type Schedule = Vec<Vec<usize>>;

/// Return list of systems that must run sequentially
/// All sublist may run in parallel
pub fn schedule(stage: &SystemStage) -> Schedule {
    if stage.systems.is_empty() {
        return vec![];
    }
    let mut result = vec![vec![0]];
    let mut history = vec![QueryProperties {
        comp_mut: (stage.systems[0].components_mut)(),
        comp_const: (stage.systems[0].components_const)(),
        res_mut: (stage.systems[0].resources_mut)(),
        res_const: (stage.systems[0].resources_const)(),
    }];

    'systems: for (sys_index, sys) in stage.systems.iter().enumerate().skip(1) {
        let props = QueryProperties {
            comp_mut: (sys.components_mut)(),
            res_mut: (sys.resources_mut)(),
            comp_const: (sys.components_const)(),
            res_const: (sys.resources_const)(),
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

struct QueryProperties {
    comp_mut: HashSet<TypeId>,
    comp_const: HashSet<TypeId>,
    res_mut: HashSet<TypeId>,
    res_const: HashSet<TypeId>,
}

impl QueryProperties {
    fn is_disjoint(&self, other: &QueryProperties) -> bool {
        self.comp_mut.is_disjoint(&other.comp_const)
            && self.res_mut.is_disjoint(&other.res_const)
            && self.comp_mut.is_disjoint(&other.comp_mut)
            && self.res_mut.is_disjoint(&other.res_mut)
            && self.comp_const.is_disjoint(&other.comp_mut)
            && self.res_const.is_disjoint(&other.res_mut)
    }

    fn extend(&mut self, props: QueryProperties) {
        self.comp_mut.extend(props.comp_mut.into_iter());
        self.res_mut.extend(props.res_mut.into_iter());
        self.comp_const.extend(props.comp_const.into_iter());
        self.res_const.extend(props.res_const.into_iter());
    }
}

#[cfg(test)]
mod tests {

    use crate::{commands::Commands, prelude::Query};

    use super::*;

    #[test]
    fn basic_schedule_with_query_test() {
        fn system_0(_cmd: Commands, _q: Query<&i32>) {}
        fn system_1(_q: Query<&i32>, _: Query<&u32>) {}
        fn system_2(_q: Query<(&mut i32, &u32)>, _: Query<&u32>, _: Query<&String>) {}
        fn system_3(_q: Query<&mut i32>) {}
        fn system_4(_q: Query<&String>) {}

        let stage = SystemStage::new("many_systems")
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
}
