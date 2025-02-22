use crate::{prelude::ResMut, World};

use super::SystemStage;

#[test]
fn test_unordered_systems_keep_the_original_ordering() {
    let mut stage = SystemStage::new("test");

    stage.add_system(move |mut c: ResMut<Vec<usize>>| c.push(0));
    stage.add_system(move |mut c: ResMut<Vec<usize>>| c.push(1));
    stage.add_system(move |mut c: ResMut<Vec<usize>>| c.push(2));
    stage.add_system(move |mut c: ResMut<Vec<usize>>| c.push(3));
    stage.add_system(move |mut c: ResMut<Vec<usize>>| c.push(4));

    let stage = stage.build();

    let mut w = World::new(0);
    w.insert_resource(Vec::<usize>::with_capacity(5));

    w.run_stage(stage);

    let nums = w.get_resource::<Vec<usize>>().unwrap();

    assert_eq!(nums.len(), 5);

    for (i, j) in nums.iter().enumerate() {
        assert_eq!(i, *j);
    }
}
