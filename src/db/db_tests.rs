use super::*;

#[test]
fn iter_query_test() {
    let mut archetype = ArchetypeStorage::empty()
        .extend_with_column::<String>()
        .extend_with_column::<u32>();

    {
        let id = EntityId::new(0, 1);
        let index = archetype.insert_entity(id);
        archetype.set_component(id, index, "pog".to_string());
        archetype.set_component(id, index, 42u32);

        let id = EntityId::new(32, 1);
        let index = archetype.insert_entity(id);
        archetype.set_component(id, index, "pog32".to_string());
        archetype.set_component(id, index, 69u32);
    }

    let q = ComponentQuery::<String>::default();
    for (comp, exp) in q.iter(&archetype).zip(["pog", "pog32"].iter()) {
        assert_eq!(&*comp, exp);
    }
}
