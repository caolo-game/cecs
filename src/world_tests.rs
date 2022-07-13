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
    fn my_system(q: Query<(&mut Foo, EntityId)>) {
        for (foo, _id) in q.iter() {
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

    let q = Query::<(&mut Foo, &String)>::new(&world);
    let (foo, s) = q.fetch(id).unwrap();

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

    fn sys0(q: Query<&mut Foo>) {
        for foo in q.iter() {
            foo.value = 42;
        }
    }

    fn assert_sys(q: Query<&Foo>) {
        for foo in q.iter() {
            assert_eq!(foo.value, 42);
        }
    }

    world.add_stage(
        SystemStage::new("many_systems")
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
        SystemStage::new("")
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
        SystemStage::new("instant-run")
            .with_should_run(should_not_run)
            .with_system(system),
    );

    world.add_stage(
        SystemStage::new("tick-run")
            .with_should_run(should_not_run)
            .with_system(system),
    );

    world.tick();

    assert_eq!(world.get_resource::<i32>().unwrap(), &0);
}

#[cfg(feature = "serde")]
#[test]
fn save_load_test() {
    let mut world = World::new(100);

    let mut ids = std::collections::HashSet::new();
    for _ in 0..100 {
        ids.insert(world.insert_entity().unwrap());
    }

    let mut payload = Vec::new();
    world
        .save_entity_ids(&mut bincode::Serializer::new(
            &mut payload,
            bincode::config::DefaultOptions::new(),
        ))
        .unwrap();

    let mut deser =
        bincode::de::Deserializer::from_slice(&payload, bincode::config::DefaultOptions::new());

    let world2 = World::load_entity_ids(&mut deser).unwrap();

    let mut seen = std::collections::HashSet::new();
    for id in Query::<EntityId>::new(&world2).iter() {
        assert!(ids.contains(&id));
        assert!(world.is_id_valid(id));
        seen.insert(id);
    }

    assert_eq!(ids, seen);
}
