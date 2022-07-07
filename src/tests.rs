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

    w.delete_entity(id).unwrap();
}
