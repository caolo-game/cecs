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

    let q = ArchQuery::<&String>::default();
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

    let q = ArchQuery::<(&String, &u32)>::default();
    for ((s, i), (exps, expi)) in q.iter(&archetype).zip([("pog", 42), ("pog32", 69)].iter()) {
        assert_eq!(&*s, exps);
        assert_eq!(&*i, expi);
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

    let q = ArchQuery::<(&String, &u64)>::default();
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

    let q = ArchQuery::<&mut String>::default();
    for mut s in q.iter(&mut archetype) {
        *s = "winnie".to_string();
    }

    let q = ArchQuery::<&String>::default();
    for s in q.iter(&archetype) {
        assert_eq!(*s, "winnie");
    }
}

#[test]
fn can_mix_mut_ref_test() {
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

    for (_a, mut b) in ArchQuery::<(&String, &mut u32)>::default().iter(&archetype) {
        *b = 42424242;
    }
    for val in ArchQuery::<&u32>::default().iter(&archetype) {
        assert_eq!(*val, 42424242);
    }

    for (mut a, _b) in ArchQuery::<(&mut String, &u32)>::default().iter(&archetype) {
        *a = "winnie".to_string();
    }

    for val in ArchQuery::<&String>::default().iter(&archetype) {
        assert_eq!(*val, "winnie");
    }

    // test if compiles
    let _ = ArchQuery::<(&String, &mut u32)>::default();
    let _ = ArchQuery::<(&mut String, &mut u32)>::default();
    let _ = ArchQuery::<(&mut u32, &mut String)>::default();
}
