use serde::{
    de::{DeserializeOwned, Visitor},
    ser::SerializeMap,
    Serialize,
};
use std::collections::HashSet;
use std::marker::PhantomData;

use crate::{entity_id::EntityId, prelude::Query, Component, World, VOID_TY};

pub struct WorldPersister<T = (), P = ()> {
    next: Option<P>,
    ty: SerTy,
    depth: usize,
    _m: PhantomData<T>,
}

impl WorldPersister<(), ()> {
    pub fn new() -> Self {
        WorldPersister {
            next: None,
            depth: 0,
            ty: SerTy::Noop,
            _m: PhantomData,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SerTy {
    Component,
    Resource,
    Noop,
}

impl<T, P> WorldPersister<T, P> {
    /// Component will be serialized
    ///
    /// Entities with no component in WorldPersister will not be serialized
    ///
    /// You can GC unserialized entities after deserialization by deleting entities with
    /// [[World::gc_empty_entities]]
    pub fn add_component<U: Component + Serialize + DeserializeOwned>(
        self,
    ) -> WorldPersister<U, Self> {
        WorldPersister {
            depth: self.depth + 1,
            next: Some(self),
            ty: SerTy::Component,
            _m: PhantomData,
        }
    }

    pub fn add_resource<U: Component + Serialize + DeserializeOwned>(
        self,
    ) -> WorldPersister<U, Self> {
        WorldPersister {
            depth: self.depth + 1,
            next: Some(self),
            ty: SerTy::Resource,
            _m: PhantomData,
        }
    }
}

pub trait WorldSerializer: Sized {
    fn add_component<U: Component + Serialize + DeserializeOwned>(self) -> WorldPersister<U, Self>;
    fn add_resource<U: Component + Serialize + DeserializeOwned>(self) -> WorldPersister<U, Self>;

    fn save<S: serde::Serializer>(&self, s: S, world: &World) -> Result<S::Ok, S::Error>;
    fn save_entry<S: serde::Serializer>(
        &self,
        s: &mut S::SerializeMap,
        world: &World,
    ) -> Result<(), S::Error>;

    fn load<'a, D: serde::Deserializer<'a>>(&'a self, d: D) -> Result<World, D::Error>;

    fn visit_map_value<'de, A>(
        &self,
        key: &str,
        map: &mut A,
        world: &mut World,
        initialized_entities: &mut HashSet<EntityId>,
    ) -> Result<(), A::Error>
    where
        A: serde::de::MapAccess<'de>;
}

fn entry_name<T: 'static>(ty: SerTy) -> String {
    format!("{:?}_{}", ty, std::any::type_name::<T>())
}

// Never actually called, just stops the impl recursion
impl WorldSerializer for () {
    fn add_component<U: Component + Serialize + DeserializeOwned>(self) -> WorldPersister<U, Self> {
        unreachable!()
    }

    fn add_resource<U: Component + Serialize + DeserializeOwned>(self) -> WorldPersister<U, Self> {
        unreachable!()
    }

    fn save<S: serde::Serializer>(&self, _s: S, _world: &World) -> Result<S::Ok, S::Error> {
        unreachable!()
    }

    fn save_entry<S: serde::Serializer>(
        &self,
        _s: &mut S::SerializeMap,
        _world: &World,
    ) -> Result<(), S::Error> {
        unreachable!()
    }

    fn load<'a, D: serde::Deserializer<'a>>(&'a self, _d: D) -> Result<World, D::Error> {
        unreachable!()
    }

    fn visit_map_value<'de, A>(
        &self,
        _key: &str,
        _map: &mut A,
        _world: &mut World,
        _initialized_entities: &mut HashSet<EntityId>,
    ) -> Result<(), A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        unreachable!()
    }
}

struct WorldVisitor<'a, T, P>
where
    P: WorldSerializer,
    T: Component + DeserializeOwned + Serialize,
{
    persist: &'a WorldPersister<T, P>,
    world: World,
    initialized_entities: HashSet<EntityId>,
    ids_initialized: bool,
}

impl<'a, 'de: 'a, T, P> Visitor<'de> for WorldVisitor<'a, T, P>
where
    P: WorldSerializer,
    T: Component + DeserializeOwned + Serialize,
{
    // return the world and set of entities that were initialized
    type Value = (World, HashSet<EntityId>);

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("Serialized World")
    }

    fn visit_map<A>(mut self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        while let Some(key) = map.next_key::<std::borrow::Cow<'de, str>>()? {
            if key == "entity_ids" {
                #[cfg(feature = "tracing")]
                tracing::trace!("• Deserializing entity_ids");
                let entity_ids = map.next_value()?;
                self.world.entity_ids = entity_ids;
                self.ids_initialized = true;
                #[cfg(feature = "tracing")]
                tracing::trace!("✓ Deserializing entity_ids");
            } else {
                assert!(
                    self.ids_initialized,
                    "Entity IDs must be initialized before deserializing other fields"
                );
                self.persist.visit_map_value(
                    key.as_ref(),
                    &mut map,
                    &mut self.world,
                    &mut self.initialized_entities,
                )?;
            }
        }

        Ok((self.world, self.initialized_entities))
    }
}

impl<T: Component + Serialize + DeserializeOwned, P> WorldSerializer for WorldPersister<T, P>
where
    P: WorldSerializer,
{
    fn add_component<U: Component + Serialize + DeserializeOwned>(self) -> WorldPersister<U, Self> {
        self.add_component::<U>()
    }

    fn add_resource<U: Component + Serialize + DeserializeOwned>(self) -> WorldPersister<U, Self> {
        self.add_resource::<U>()
    }

    fn save<S: serde::Serializer>(&self, s: S, world: &World) -> Result<S::Ok, S::Error> {
        let mut s = s.serialize_map(Some(self.depth + 1))?;
        #[cfg(feature = "tracing")]
        tracing::trace!("• Serializing entity_ids");
        s.serialize_entry("entity_ids", &world.entity_ids)?;
        #[cfg(feature = "tracing")]
        tracing::trace!("✓ Serializing entity_ids");

        self.save_entry::<S>(&mut s, world)?;
        s.end()
    }

    fn save_entry<S: serde::Serializer>(
        &self,
        s: &mut S::SerializeMap,
        world: &World,
    ) -> Result<(), S::Error> {
        let tname = entry_name::<T>(self.ty);

        #[cfg(feature = "tracing")]
        tracing::trace!(name = &tname, "• Serializing");

        match self.ty {
            SerTy::Component => {
                // TODO: save iterator or adapter pls
                let values: Vec<(EntityId, &T)> =
                    Query::<(EntityId, &T)>::new(world).iter().collect();
                s.serialize_entry(&tname, &values)?;
            }
            SerTy::Resource => {
                if let Some(value) = world.get_resource::<T>() {
                    s.serialize_entry(&tname, value)?;
                }
            }
            SerTy::Noop => {}
        }
        #[cfg(feature = "tracing")]
        tracing::trace!(name = &tname, "✓ Serializing");
        if let Some(p) = self.next.as_ref() {
            p.save_entry::<S>(s, world)?;
        }
        Ok(())
    }

    fn load<'a, D: serde::Deserializer<'a>>(&'a self, d: D) -> Result<World, D::Error> {
        let world = World::new(0);
        let visitor = WorldVisitor {
            persist: self,
            world,
            initialized_entities: Default::default(),
            ids_initialized: false,
        };
        let (mut world, initialized_entities) = d.deserialize_map(visitor)?;

        let free_set = world
            .entity_ids
            .walk_free_list()
            .map(|(gen, index)| EntityId::new(index, gen))
            .collect::<HashSet<EntityId>>();

        let mut uninitialized_entities = Vec::new();
        for (i, entry) in world.entity_ids.entries_mut().iter_mut().enumerate() {
            let id = EntityId::new(i as u32, entry.gen);

            if !free_set.contains(&id) && !initialized_entities.contains(&id) {
                // entity was allocated, but not serialized
                uninitialized_entities.push(id);
            }
        }
        for id in uninitialized_entities {
            unsafe {
                world.init_id(id);
            }
        }

        Ok(world)
    }

    fn visit_map_value<'de, A>(
        &self,
        key: &str,
        map: &mut A,
        world: &mut World,
        initialized_entities: &mut HashSet<EntityId>,
    ) -> Result<(), A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let tname = entry_name::<T>(self.ty);
        #[cfg(feature = "tracing")]
        if tname != key {
            if let Some(next) = &self.next {
                next.visit_map_value(key, map, world, initialized_entities)?;
            }
            return Ok(());
        }
        #[cfg(feature = "tracing")]
        tracing::trace!(name = &tname, "• Deserializing");
        match self.ty {
            SerTy::Component => {
                let values: Vec<(EntityId, T)> = map.next_value()?;
                for (id, value) in values {
                    if !initialized_entities.contains(&id) {
                        let void_store = world.archetypes.get_mut(&VOID_TY).unwrap();
                        let index = void_store.as_mut().insert_entity(id);
                        void_store.as_mut().set_component(index, ());
                        world
                            .entity_ids
                            .update(id, void_store.as_mut().get_mut(), index);
                        initialized_entities.insert(id);
                    }
                    debug_assert!(world.is_id_valid(id));
                    world.set_component(id, value).unwrap();
                }
            }
            SerTy::Resource => {
                let value: T = map.next_value()?;
                world.insert_resource(value);
            }
            SerTy::Noop => {}
        }

        #[cfg(feature = "tracing")]
        tracing::trace!(name = &tname, "✓ Deserializing");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_test::traced_test;

    #[derive(serde::Serialize, serde::Deserialize, Clone)]
    struct Foo {
        value: u32,
    }

    #[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Eq)]
    enum Bar {
        Foo,
        Bar,
        Baz,
    }

    #[test]
    fn save_load_json_test() {
        let mut world0 = World::new(4);

        for i in 0u32..10u32 {
            let id = world0.insert_entity().unwrap();
            world0.set_component(id, 42i32).unwrap();
            world0.set_component(id, i).unwrap();
            world0.set_component(id, Foo { value: i }).unwrap();
            world0.set_component(id, Bar::Baz).unwrap();
        }

        let p = WorldPersister::new()
            .add_component::<i32>()
            .add_component::<Foo>()
            .add_component::<Bar>();
        let mut result = Vec::<u8>::new();
        let mut s = serde_json::Serializer::pretty(&mut result);

        p.save(&mut s, &world0).unwrap();

        let result = String::from_utf8(result).unwrap();

        let world1 = p
            .load(&mut serde_json::Deserializer::from_str(result.as_str()))
            .unwrap();

        type QueryTuple<'a> = (EntityId, &'a i32, &'a Foo);

        for ((id0, i0, f0), (id1, i1, f1)) in Query::<QueryTuple>::new(&world0)
            .iter()
            .zip(Query::<QueryTuple>::new(&world1).iter())
        {
            assert_eq!(id0, id1);
            assert_eq!(i0, i1);
            assert_eq!(f0.value, f1.value);
        }

        assert_eq!(
            Query::<&u32>::new(&world1).count(),
            0,
            "Assumes that non registered types are not (de)serialized"
        );
    }

    #[test]
    fn save_load_bincode_test() {
        let mut world0 = World::new(4);

        for i in 0u32..10u32 {
            let id = world0.insert_entity().unwrap();
            world0.set_component(id, 42i32).unwrap();
            world0.set_component(id, i).unwrap();
            world0.set_component(id, Foo { value: i }).unwrap();
        }

        let p = WorldPersister::new()
            .add_component::<i32>()
            .add_component::<Foo>();

        let mut result = Vec::<u8>::new();
        let mut s = bincode::Serializer::new(&mut result, bincode::config::DefaultOptions::new());
        p.save(&mut s, &world0).unwrap();

        let world1 = p
            .load(&mut bincode::de::Deserializer::from_slice(
                result.as_slice(),
                bincode::config::DefaultOptions::new(),
            ))
            .unwrap();

        type QueryTuple<'a> = (EntityId, &'a i32, &'a Foo);

        for ((id0, i0, f0), (id1, i1, f1)) in Query::<QueryTuple>::new(&world0)
            .iter()
            .zip(Query::<QueryTuple>::new(&world1).iter())
        {
            assert_eq!(id0, id1);
            assert_eq!(i0, i1);
            assert_eq!(f0.value, f1.value);
        }

        assert_eq!(
            Query::<&u32>::new(&world1).count(),
            0,
            "Assumes that non registered types are not (de)serialized"
        );
    }

    #[test]
    fn resource_saveload_json_test() {
        let mut world0 = World::new(4);

        for i in 0u32..4u32 {
            let id = world0.insert_entity().unwrap();
            world0.set_component(id, 42i32).unwrap();
            world0.set_component(id, i).unwrap();
            world0.set_component(id, Foo { value: i }).unwrap();
        }

        world0.insert_resource(Foo { value: 69 });

        let p = WorldPersister::new()
            .add_component::<Foo>()
            .add_resource::<Foo>();

        let mut pl = Vec::<u8>::new();
        let mut s = serde_json::Serializer::pretty(&mut pl);

        p.save(&mut s, &world0).unwrap();

        let pretty = std::str::from_utf8(&pl).unwrap();
        println!("{}", pretty);

        let world1 = p
            .load(&mut serde_json::Deserializer::from_reader(pl.as_slice()))
            .unwrap();

        type QueryTuple<'a> = (EntityId, &'a Foo);

        let mut count = 0;
        for ((id0, f0), (id1, f1)) in Query::<QueryTuple>::new(&world0)
            .iter()
            .zip(Query::<QueryTuple>::new(&world1).iter())
        {
            assert_eq!(id0, id1);
            assert_eq!(f0.value, f1.value);
            count += 1;
        }
        assert_eq!(count, 4);

        assert_eq!(
            world1.get_resource::<Foo>().expect("foo not found").value,
            69
        );
    }

    #[test]
    fn entity_ids_inserted_are_the_same_after_serde_test() {
        let mut world0 = World::new(4);

        let mut ids = Vec::with_capacity(100);
        for i in 0u32..100u32 {
            let id = world0.insert_entity().unwrap();
            world0.set_component(id, 42i32).unwrap();
            world0.set_component(id, i).unwrap();
            world0.set_component(id, Foo { value: i }).unwrap();
            ids.push(id);
        }
        // add some entities that are not saved
        for i in 0u32..100u32 {
            let id = world0.insert_entity().unwrap();
            world0.set_component(id, i).unwrap();
            ids.push(id);
        }
        // delete some entities
        for id in ids.iter().copied() {
            if id.index() % 3 == 0 {
                world0.delete_entity(id).unwrap();
            }
        }

        let p = WorldPersister::new()
            .add_component::<Foo>()
            .add_component::<i32>();

        let mut pl = Vec::<u8>::new();
        let mut s = serde_json::Serializer::pretty(&mut pl);

        p.save(&mut s, &world0).unwrap();

        // let pretty = std::str::from_utf8(&pl).unwrap();
        // println!("{}", pretty);

        let mut world1 = p
            .load(&mut serde_json::Deserializer::from_reader(pl.as_slice()))
            .unwrap();

        for i in 0..20 {
            let id0 = world0.insert_entity().unwrap();
            let id1 = world1.insert_entity().unwrap();
            ids.push(id0);

            assert_eq!(id0, id1, "#{}: expected: {} actual: {}", i, id0, id1,);
        }

        // free all ids and then try allocating
        for id in ids {
            if id.index() % 3 != 0 {
                world0.delete_entity(id).unwrap_or_default();
                world1.delete_entity(id).unwrap_or_default();
            }
        }
        for i in 0..20 {
            let id0 = world0.insert_entity().unwrap();
            let id1 = world1.insert_entity().unwrap();

            assert_eq!(id0, id1, "#{}: expected: {} actual: {}", i, id0, id1,);
        }
    }

    #[test]
    #[traced_test]
    fn can_serde_multiple_resources_test() {
        // regression test: had a bug where the first resource would not be deserialized

        let mut world0 = World::new(4);
        world0.insert_resource(42i64);
        world0.insert_resource(69u32);

        let p = WorldPersister::new()
            .add_resource::<u32>()
            .add_resource::<i64>();

        let mut result = Vec::<u8>::new();
        let mut s = bincode::Serializer::new(&mut result, bincode::config::DefaultOptions::new());
        p.save(&mut s, &world0).unwrap();

        let world1 = p
            .load(&mut bincode::de::Deserializer::from_slice(
                result.as_slice(),
                bincode::config::DefaultOptions::new(),
            ))
            .unwrap();

        let i = world1.get_resource::<i64>().unwrap();
        assert_eq!(i, &42);
        let u = world1.get_resource::<u32>().unwrap();
        assert_eq!(u, &69);
    }
}
