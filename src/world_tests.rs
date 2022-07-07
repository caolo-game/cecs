use crate::query::Query;

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

    let mut exp = vec![(id1, "poggers1".to_string()), (id2, "poggers2".to_string())];
    exp.sort_by_key(|(id, _)| *id);

    // let mut act = Query::<(EntityId, &String)>::new(&world)
    //     .iter()
    //     .collect::<Vec<_>>();
    // act.sort_by_key(|(id, _)| *id);
    //
    // assert_eq!(&exp[..], &act[..]);

    // test if compiles
    Query::<EntityId>::new(&world);
}
