use crate::entity_id::EntityId;

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

    let q = ComponentQuery::<&String>::default();
    for (comp, exp) in q.iter(&archetype).zip(["pog", "pog32"].iter()) {
        assert_eq!(*comp, *exp);
    }
}

#[test]
fn joined_iter_test() {
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

    let q = ComponentQuery::<(&String, &u32)>::default();
    for ((s, i), (exps, expi)) in q.iter(&archetype).zip([("pog", 42), ("pog32", 69)].iter()) {
        assert_eq!(*s, exps);
        assert_eq!(*i, expi);
    }
}

#[test]
fn non_existent_component_iter_is_empty_test() {
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

    let q = ComponentQuery::<(&String, &u64)>::default();
    for _ in q.iter(&archetype) {
        panic!();
    }
}

#[test]
fn mutable_iterator_test() {
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

    let q = ComponentQuery::<&mut String>::default();
    for mut s in q.iter(&mut archetype) {
        *s = "winnie".to_string();
    }

    let q = ComponentQuery::<&String>::default();
    for s in q.iter(&archetype) {
        assert_eq!(*s, "winnie");
    }
}
