use commands::Commands;

use crate::entity_id::EntityId;
use crate::prelude::ResMut;
use crate::query::resource_query::Res;
use crate::query::{filters::WithOut, Query};

use super::*;

#[test]
fn world_deinit_test() {
    let mut w = World::new(500);
    let _id = w.insert_entity().unwrap();
}

#[test]
fn can_insert_component_test() {
    let mut w = World::new(500);

    let id = w.insert_entity().unwrap();
    w.set_component(id, "poggers".to_string()).unwrap();
    w.set_component(id, 32u32).unwrap();
    w.set_component(id, 42u64).unwrap();

    let id2 = w.insert_entity().unwrap();
    w.set_component(id2, 1u32).unwrap();
    w.set_component(id2, "poggers2".to_string()).unwrap();
    w.delete_entity(id2).unwrap();

    let id3 = w.insert_entity().unwrap();
    w.set_component(id3, 2u32).unwrap();
    w.set_component(id3, "poggers3".to_string()).unwrap();
    w.remove_component::<String>(id3).unwrap();
    w.delete_entity(id3).unwrap();
}

#[test]
fn can_remove_component_test() {
    let mut w = World::new(500);

    let id = w.insert_entity().unwrap();
    w.set_component(id, 2u32).unwrap();
    w.set_component(id, "poggers3".to_string()).unwrap();

    assert!(w.get_component::<String>(id).unwrap() == "poggers3");

    w.remove_component::<String>(id).unwrap();
    let res = w.get_component::<String>(id);
    assert!(res.is_none(), "Expected none, got: {:?}", res);
}

#[test]
fn can_update_component_test() {
    let mut w = World::new(500);

    let id = w.insert_entity().unwrap();
    w.set_component(id, "poggers3".to_string()).unwrap();
    assert!(w.get_component::<String>(id).unwrap() == "poggers3");

    w.set_component(id, "poggers2".to_string()).unwrap();
    assert!(w.get_component::<String>(id).unwrap() == "poggers2");
}

#[test]
fn query_can_iter_multiple_archetypes_test() {
    let mut world = World::new(500);

    let id = world.insert_entity().unwrap();
    world.set_component(id, "poggers".to_string()).unwrap();
    world.set_component(id, 16).unwrap();

    let id = world.insert_entity().unwrap();
    world.set_component(id, "poggers".to_string()).unwrap();

    let mut count = 0;
    for pog in Query::<&String>::new(&world).iter() {
        assert_eq!(*pog, "poggers");
        count += 1;
    }
    assert_eq!(count, 2);

    // test if compiles
    Query::<(&u32, &String)>::new(&world);
    Query::<(&mut u32, &String)>::new(&world);
    Query::<(&u32, &mut String)>::new(&world);
    Query::<(&mut String, &u32)>::new(&world);
}

#[test]
fn can_query_entity_id_test() {
    let mut world = World::new(500);

    let id1 = world.insert_entity().unwrap();
    world.set_component(id1, "poggers1".to_string()).unwrap();
    world.set_component(id1, 16).unwrap();

    let id2 = world.insert_entity().unwrap();
    world.set_component(id2, "poggers2".to_string()).unwrap();

    let mut exp = vec![(id1, "poggers1"), (id2, "poggers2")];
    exp.sort_by_key(|(id, _)| *id);

    let mut act = Query::<(EntityId, &String)>::new(&world)
        .iter()
        .map(|(id, s)| (id, s.as_str()))
        .collect::<Vec<_>>();
    act.sort_by_key(|(id, _)| *id);

    assert_eq!(&exp[..], &act[..]);

    // test if compiles
    Query::<EntityId>::new(&world);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Foo {
    value: i32,
}

#[test]
fn system_test() {
    fn my_system(mut q: Query<(&mut Foo, EntityId)>) {
        for (foo, _id) in q.iter_mut() {
            foo.value = 69;
        }
    }

    let mut world = World::new(500);

    for i in 0..4 {
        let id = world.insert_entity().unwrap();
        world.set_component(id, Foo { value: i }).unwrap();
        if i % 2 == 0 {
            world.set_component(id, "poggers".to_string()).unwrap();
        }
        dbg!(id, i);
    }

    my_system(Query::new(&world));

    for foo in Query::<&Foo>::new(&world).iter() {
        assert_eq!(foo.value, 69);
    }
}

#[test]
fn can_fetch_single_entity_test() {
    let mut world = World::new(500);

    for i in 0..4 {
        let id = world.insert_entity().unwrap();
        world.set_component(id, Foo { value: i }).unwrap();
        if i % 2 == 0 {
            world.set_component(id, "poggers".to_string()).unwrap();
        }
    }
    let id = world.insert_entity().unwrap();
    world.set_component(id, Foo { value: 0xbeef }).unwrap();
    world.set_component(id, "winnie".to_string()).unwrap();
    for i in 0..4 {
        let id = world.insert_entity().unwrap();
        world.set_component(id, Foo { value: i }).unwrap();
        if i % 2 == 0 {
            world.set_component(id, "poggers".to_string()).unwrap();
        }
        dbg!(id, i);
    }

    let mut q = Query::<(&mut Foo, &String)>::new(&world);
    let (foo, s) = q.fetch_mut(id).unwrap();

    assert_eq!(foo.value, 0xbeef);
    foo.value = 0;
    assert_eq!(s, "winnie");
}

#[test]
fn optional_query_test() {
    let mut world = World::new(500);

    for i in 0..4 {
        let id = world.insert_entity().unwrap();
        world.set_component(id, Foo { value: i }).unwrap();
        if i % 2 == 0 {
            world.set_component(id, "poggers".to_string()).unwrap();
        }
    }

    let cnt = Query::<(&Foo, Option<&String>)>::new(&world).iter().count();
    assert_eq!(cnt, 4);

    // assert compiles
    Query::<(&Foo, Option<&mut String>)>::new(&world);
}

#[test]
#[cfg(feature = "parallel")]
fn test_parallel() {
    use rayon::prelude::*;

    let mut world = World::new(500);

    for i in 0..4 {
        let id = world.insert_entity().unwrap();
        world.set_component(id, Foo { value: i }).unwrap();
        if i % 2 == 0 {
            world.set_component(id, "poggers".to_string()).unwrap();
        }
    }

    fn par_sys<'a>(mut q: Query<(&'a mut Foo, &'a String)>) {
        q.iter_mut().par_bridge().for_each(|(foo, _)| {
            foo.value += 1;
        });
    }

    world.run_system(par_sys);
}

#[test]
#[cfg(feature = "clone")]
fn world_clone_test() {
    let mut world = World::new(500);

    for i in 0..4 {
        let id = world.insert_entity().unwrap();
        world.set_component(id, Foo { value: i }).unwrap();
        if i % 2 == 0 {
            world.set_component(id, "poggers".to_string()).unwrap();
        }
    }

    let w2 = world.clone();

    let a = Query::<(EntityId, &Foo, Option<&String>)>::new(&world).iter();
    let b = Query::<(EntityId, &Foo, Option<&String>)>::new(&w2).iter();

    for (a, b) in a.zip(b) {
        assert_eq!(a, b);
    }
}

#[test]
fn filtered_query_test() {
    let mut world = World::new(500);

    for i in 0..4 {
        let id = world.insert_entity().unwrap();
        world.set_component(id, Foo { value: i }).unwrap();
        if i % 2 == 0 {
            world.set_component(id, "poggers".to_string()).unwrap();
        }
    }

    for i in Query::<&Foo, WithOut<String>>::new(&world).iter() {
        assert!(i.value % 2 == 1);
    }
}

#[test]
fn resource_test() {
    let mut world = World::new(4);
    world.insert_resource(4i32);

    let res = world.get_resource::<i32>().unwrap();
    assert_eq!(res, &4);

    world.remove_resource::<i32>();

    assert!(world.get_resource::<i32>().is_none());
}

#[test]
fn resource_query_test() {
    let mut world = World::new(4);
    world.insert_resource(4i32);

    fn sys(res: Res<i32>) {
        assert_eq!(*res, 4i32);
    }

    sys(Res::new(&world));
}

#[test]
fn world_execute_systems_test() {
    let mut world = World::new(400);

    for i in 0..400 {
        let id = world.insert_entity().unwrap();
        world.set_component(id, Foo { value: i }).unwrap();
        if i % 2 == 0 {
            world.set_component(id, "poggers".to_string()).unwrap();
        }
    }

    // FIXME: i'd like to be able to specify queries like this,
    // without using the same lifetime for all tuple items
    fn sys0<'a>(mut q: Query<(&'a mut Foo, &'a ())>) {
        for (foo, _) in q.iter_mut() {
            foo.value = 42;
        }
    }

    fn assert_sys(q: Query<(&Foo, &())>) {
        for (foo, _) in q.iter() {
            assert_eq!(foo.value, 42);
        }
    }

    world.add_stage(
        SystemStage::parallel("many_systems")
            .with_system(sys0)
            .with_system(assert_sys)
            .with_system(assert_sys)
            .with_system(assert_sys)
            .with_system(assert_sys)
            .with_system(assert_sys)
            .with_system(assert_sys)
            .with_system(assert_sys)
            .with_system(assert_sys)
            .with_system(assert_sys)
            .with_system(assert_sys)
            .with_system(assert_sys),
    );

    world.tick();

    world.run_system(assert_sys);

    world.run_stage(
        SystemStage::parallel("")
            .with_system(assert_sys)
            .with_system(assert_sys),
    );
}

#[test]
fn can_skip_stage_test() {
    let mut world = World::new(4);

    world.insert_resource(0i32);

    fn system(mut q: ResMut<i32>) {
        *q += 1i32;
    }

    fn should_not_run() -> bool {
        false
    }

    world.run_stage(
        SystemStage::parallel("instant-run")
            .with_should_run(should_not_run)
            .with_system(system),
    );

    world.add_stage(
        SystemStage::parallel("tick-run")
            .with_should_run(should_not_run)
            .with_system(system),
    );

    world.tick();

    assert_eq!(world.get_resource::<i32>().unwrap(), &0);
}

#[test]
fn borrowing_same_type_const_twice_is_ok_test() {
    fn sys(_valid_query1: Query<(&i32, &i32)>, _valid_query2: Query<(&i32, &i32)>) {}

    let mut world = World::new(1);

    world.run_system(sys);
}

#[test]
#[should_panic]
#[cfg(debug_assertions)]
fn invalid_query_panics_double_mut_test() {
    fn sys(_invalid_query: Query<(&mut i32, &mut i32)>) {}

    let mut world = World::new(1);

    world.run_system(sys);
}

#[test]
#[should_panic]
#[cfg(debug_assertions)]
fn invalid_query_panics_test() {
    fn sys(_invalid_query: Query<(&i32, &mut i32)>) {}

    let mut world = World::new(1);

    world.run_system(sys);
}

#[test]
#[should_panic]
#[cfg(debug_assertions)]
fn borrowing_same_type_mutable_twice_panics_test() {
    fn sys(_valid_query_1: Query<&mut i32>, _valid_query_2: Query<&mut i32>) {}

    let mut world = World::new(1);

    world.run_system(sys);
}

#[test]
#[should_panic]
#[cfg(debug_assertions)]
fn borrowing_same_resource_mutable_twice_panics_test() {
    fn sys(_valid_query_1: Res<i32>, _valid_query_2: ResMut<i32>) {}

    let mut world = World::new(1);

    world.run_system(sys);
}

#[test]
fn can_iterate_over_immutable_iter_of_refmut_component_test() {
    fn sys(q: Query<(&mut i32, &u32)>) {
        for (a, _b) in q.iter() {
            assert_eq!(a, &69);
        }
    }

    let mut world = World::new(1);
    world.run_system(sys);
}

#[test]
fn can_insert_bundle_test() {
    let mut world = World::new(2);

    let entity_id = world.insert_entity().unwrap();
    let entity_id2 = world.insert_entity().unwrap();

    world.set_bundle(entity_id, (42i32, 38u32)).unwrap();
    world
        .set_bundle(entity_id2, ("exo".to_string(), "poggers", 62i32, 88u32))
        .unwrap();

    let a = world.get_component::<i32>(entity_id).unwrap();
    assert_eq!(a, &42);

    let a = world.get_component::<u32>(entity_id).unwrap();
    assert_eq!(a, &38);
}

#[test]
fn can_insert_bundle_via_command_test() {
    let mut world = World::new(2);

    fn sys(mut cmd: Commands) {
        cmd.spawn().insert_bundle((42i32, 38u32));
    }

    world.run_system(sys);

    for (a, b) in Query::<(&u32, &i32)>::new(&world).iter() {
        assert_eq!(a, &38);
        assert_eq!(b, &42);
    }
}
