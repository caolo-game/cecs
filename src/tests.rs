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
    w.delete_entity(id3).unwrap();
}
