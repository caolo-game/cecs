use crate::prelude::ResMut;

use super::*;

#[test]
fn basic_pipe_test() {
    fn sys1(mut i: ResMut<i32>) {
        assert_eq!(*i, 0);
        *i += 1;
    }
    fn sys2(mut i: ResMut<i32>) {
        assert_eq!(*i, 1);
        *i += 1;
    }

    let mut world = World::new(0);

    world.add_stage(SystemStage::new("test").with_system(Piped {
        lhs: sys1.descriptor(),
        rhs: sys2.descriptor(),
    }));

    world.insert_resource(0i32);
    world.tick();

    let i = world.get_resource::<i32>().unwrap();
    assert_eq!(i, &2i32);
}

#[test]
fn function_can_be_piped_test() {
    fn sys1(mut i: ResMut<i32>) {
        assert_eq!(*i, 0);
        *i += 1;
    }
    fn sys2(mut i: ResMut<i32>) {
        assert_eq!(*i, 1);
        *i += 1;
    }
    fn sys3(mut i: ResMut<i32>) {
        assert_eq!(*i, 2);
        *i += 1;
    }

    let mut world = World::new(0);

    world.add_stage(SystemStage::new("test").with_system(sys1.pipe(sys2).pipe(sys3)));

    world.insert_resource(0i32);
    world.tick();

    let i = world.get_resource::<i32>().unwrap();
    assert_eq!(i, &3i32);
}
