use crate::World;

use super::*;

#[derive(Default, Clone)]
struct Foo {
    value: i32,
}

#[derive(Default, Clone)]
struct Bar;

#[test]
fn pair_query_set_test() {
    fn sys<'a>(mut q: QuerySet<(Query<&'a mut Foo>, Query<(&'a mut Foo, &'a Bar)>)>) {
        for foo in q.q0_mut().iter_mut() {
            assert_eq!(foo.value, 0);
            foo.value = 42;
        }

        for (foo, _bar) in q.q1().iter() {
            assert_eq!(foo.value, 42);
        }
    }

    let mut world = World::new(4);

    let e = world.insert_entity();
    world.set_component(e, Foo::default()).unwrap();
    world.set_component(e, Bar).unwrap();

    world.run_system(sys);
}

#[test]
fn triplet_query_set_test() {
    fn sys<'a>(
        mut q: QuerySet<(
            Query<(&'a mut Foo, &'a Bar)>,
            Query<(&'a mut Foo, &'a Bar, &'a mut i32)>,
            Query<&'a mut Foo>,
        )>,
    ) {
        for foo in q.q2_mut().iter_mut() {
            assert_eq!(foo.value, 0);
            foo.value = 42;
        }

        for (foo, _bar, _) in q.q1().iter() {
            assert_eq!(foo.value, 42);
        }
    }

    let mut world = World::new(4);

    let e = world.insert_entity();
    world.set_component(e, Foo::default()).unwrap();
    world.set_component(e, Bar).unwrap();

    world.run_system(sys);
}
