use crate::{
    entity_id::EntityId,
    query::filters::{Filter, Or},
};

use super::{filters::With, *};

#[test]
fn iter_query_test() {
    let mut archetype = ArchetypeStorage::empty()
        .extend_with_column::<String>()
        .extend_with_column::<u32>();

    {
        let id = EntityId::new(0, 1);
        let index = archetype.insert_entity(id);
        archetype.set_component(index, "pog".to_string());
        archetype.set_component(index, 42u32);

        let id = EntityId::new(32, 1);
        let index = archetype.insert_entity(id);
        archetype.set_component(index, "pog32".to_string());
        archetype.set_component(index, 69u32);
    }

    for (comp, exp) in ArchQuery::<&String>::iter(&archetype).zip(["pog", "pog32"].iter()) {
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
        archetype.set_component(index, "pog".to_string());
        archetype.set_component(index, 42u32);

        let id = EntityId::new(32, 1);
        let index = archetype.insert_entity(id);
        archetype.set_component(index, "pog32".to_string());
        archetype.set_component(index, 69u32);
    }

    for ((s, i), (exps, expi)) in
        ArchQuery::<(&String, &u32)>::iter(&archetype).zip([("pog", 42), ("pog32", 69)].iter())
    {
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
        archetype.set_component(index, "pog".to_string());
        archetype.set_component(index, 42u32);

        let id = EntityId::new(32, 1);
        let index = archetype.insert_entity(id);
        archetype.set_component(index, "pog32".to_string());
        archetype.set_component(index, 69u32);
    }

    for _ in ArchQuery::<(&String, &u64)>::iter(&archetype) {
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
        archetype.set_component(index, "pog".to_string());
        archetype.set_component(index, 42u32);

        let id = EntityId::new(32, 1);
        let index = archetype.insert_entity(id);
        archetype.set_component(index, "pog32".to_string());
        archetype.set_component(index, 69u32);
    }

    for s in ArchQuery::<&mut String>::iter_mut(&mut archetype) {
        *s = "winnie".to_string();
    }

    for s in ArchQuery::<&String>::iter(&archetype) {
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
        archetype.set_component(index, "pog".to_string());
        archetype.set_component(index, 42u32);

        let id = EntityId::new(32, 1);
        let index = archetype.insert_entity(id);
        archetype.set_component(index, "pog32".to_string());
        archetype.set_component(index, 69u32);
    }

    for (_a, b) in ArchQuery::<(&String, &mut u32)>::iter_mut(&archetype) {
        *b = 42424242;
    }
    for val in ArchQuery::<&u32>::iter(&archetype) {
        assert_eq!(*val, 42424242);
    }

    for (a, _b) in ArchQuery::<(&mut String, &u32)>::iter_mut(&archetype) {
        *a = "winnie".to_string();
    }

    for val in ArchQuery::<&String>::iter(&archetype) {
        assert_eq!(*val, "winnie");
    }

    // test if compiles
    let _: ArchQuery<(&String, &mut u32)>;
    let _: ArchQuery<(&mut String, &mut u32)>;
    let _: ArchQuery<(&mut u32, &mut String)>;
}

#[test]
fn basic_filter_test() {
    let mut archetype = ArchetypeStorage::empty()
        .extend_with_column::<String>()
        .extend_with_column::<u32>();

    {
        let id = EntityId::new(0, 1);
        let index = archetype.insert_entity(id);
        archetype.set_component(index, "pog".to_string());
        archetype.set_component(index, 42u32);

        let id = EntityId::new(32, 1);
        let index = archetype.insert_entity(id);
        archetype.set_component(index, "pog32".to_string());
        archetype.set_component(index, 69u32);
    }

    assert!(!With::<i32>::filter(&archetype));

    assert!(With::<u32>::filter(&archetype));

    assert!(Or::<With<u32>, With<i32>>::filter(&archetype));
    assert!(<(With::<u32>, With::<String>) as Filter>::filter(
        &archetype
    ));
}
