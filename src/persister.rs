use serde::{
    de::{DeserializeOwned, Visitor},
    ser::SerializeMap,
    Serialize,
};
use std::marker::PhantomData;

use crate::{entity_id::EntityId, prelude::Query, Component, World};

const ENTITY_FREE_LIST_KEY: &str = "entity_index_freelist";
const ENTITY_IDS_KEY: &str = "entity_ids";

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
    pub fn with_component<U: Component + Serialize + DeserializeOwned>(
        self,
    ) -> WorldPersister<U, Self> {
        WorldPersister {
            depth: self.depth + 1,
            next: Some(self),
            ty: SerTy::Component,
            _m: PhantomData,
        }
    }

    pub fn with_resource<U: Component + Serialize + DeserializeOwned>(
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
    fn with_component<U: Component + Serialize + DeserializeOwned>(self)
        -> WorldPersister<U, Self>;
    fn with_resource<U: Component + Serialize + DeserializeOwned>(self) -> WorldPersister<U, Self>;

    fn save<S: serde::Serializer>(&self, s: S, world: &World) -> Result<S::Ok, S::Error>;
    fn save_entry<S: serde::Serializer>(
        &self,
        s: &mut S::SerializeMap,
        world: &World,
    ) -> Result<(), S::Error>;

    fn load<'a, D: serde::Deserializer<'a>>(&self, d: D) -> Result<World, D::Error>;

    fn visit_map_value<'de, A>(
        &self,
        key: &str,
        map: &mut A,
        world: &mut World,
    ) -> Result<(), A::Error>
    where
        A: serde::de::MapAccess<'de>;
}

fn entry_name<T: 'static>(ty: SerTy) -> String {
    format!("{:?}_{}", ty, std::any::type_name::<T>())
}

// Never actually called, just stops the impl recursion
impl WorldSerializer for () {
    fn with_component<U: Component + Serialize + DeserializeOwned>(
        self,
    ) -> WorldPersister<U, Self> {
        unreachable!()
    }

    fn with_resource<U: Component + Serialize + DeserializeOwned>(self) -> WorldPersister<U, Self> {
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

    fn load<'a, D: serde::Deserializer<'a>>(&self, _d: D) -> Result<World, D::Error> {
        unreachable!()
    }

    fn visit_map_value<'de, A>(
        &self,
        _key: &str,
        _map: &mut A,
        _world: &mut World,
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
    ids_initialized: bool,
    index_initialized: bool,
}

impl<'a, 'de: 'a, T, P> Visitor<'de> for WorldVisitor<'a, T, P>
where
    P: WorldSerializer,
    T: Component + DeserializeOwned + Serialize,
{
    type Value = World;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("Serialized World")
    }

    fn visit_map<A>(mut self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        while let Some(key) = map.next_key::<std::borrow::Cow<'de, str>>()? {
            match key.as_ref() {
                ENTITY_FREE_LIST_KEY => {
                    #[cfg(feature = "tracing")]
                    tracing::trace!("Deserializing entity_free_list");
                    let free_list: Vec<(u32, u32)> = map.next_value()?;
                    unsafe {
                        let free_list = free_list.iter().map(|(_, i)| *i).collect();
                        self.world.entity_ids.get_mut().set_free_list(free_list);
                    }
                    for (gen, i) in free_list {
                        unsafe {
                            self.world.entity_ids.get_mut().set_gen(i as usize, gen);
                        }
                    }
                    self.index_initialized = true;
                    #[cfg(feature = "tracing")]
                    tracing::trace!("Deserializing entity_free_list done");
                }
                ENTITY_IDS_KEY => {
                    if !self.index_initialized {
                        return Err(serde::de::Error::custom(
                            "Entity index must be initialized before deserializing entity_ids",
                        ));
                    }
                    #[cfg(feature = "tracing")]
                    tracing::trace!("Deserializing entity_ids");
                    let entity_ids: Vec<EntityId> = map.next_value()?;
                    for id in entity_ids {
                        unsafe {
                            self.world.entity_ids.get_mut().force_insert_entity(id);
                            self.world.init_id(id);
                        }
                    }
                    self.ids_initialized = true;
                    #[cfg(feature = "tracing")]
                    tracing::trace!("Deserializing entity_ids done");
                }
                _ => {
                    if !self.ids_initialized {
                        return Err(serde::de::Error::custom(
                            "Entity IDs must be initialized before deserializing other fields",
                        ));
                    }
                    self.persist
                        .visit_map_value(key.as_ref(), &mut map, &mut self.world)?;
                }
            }
        }

        Ok(self.world)
    }
}

impl<T: Component + Serialize + DeserializeOwned, P> WorldSerializer for WorldPersister<T, P>
where
    P: WorldSerializer,
{
    fn with_component<U: Component + Serialize + DeserializeOwned>(
        self,
    ) -> WorldPersister<U, Self> {
        self.with_component::<U>()
    }

    fn with_resource<U: Component + Serialize + DeserializeOwned>(self) -> WorldPersister<U, Self> {
        self.with_resource::<U>()
    }

    fn save<S: serde::Serializer>(&self, s: S, world: &World) -> Result<S::Ok, S::Error> {
        let mut s = s.serialize_map(Some(self.depth + 2))?;
        #[cfg(feature = "tracing")]
        tracing::trace!("Serializing entity free_list");
        let free_list = world.entity_ids().walk_free_list().collect::<Vec<_>>();
        s.serialize_entry(ENTITY_FREE_LIST_KEY, &free_list)?;
        #[cfg(feature = "tracing")]
        tracing::trace!("Serializing entity free_list done");
        #[cfg(feature = "tracing")]
        tracing::trace!("Serializing entity_ids");
        let ids = Query::<EntityId>::new(world).iter().collect::<Vec<_>>();
        s.serialize_entry(ENTITY_IDS_KEY, &ids)?;
        #[cfg(feature = "tracing")]
        tracing::trace!("Serializing entity_ids done");

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
        tracing::trace!(name = &tname, "Serializing");

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
        tracing::trace!(name = &tname, "Serializing done");
        if let Some(p) = self.next.as_ref() {
            p.save_entry::<S>(s, world)?;
        }
        Ok(())
    }

    fn load<'a, D: serde::Deserializer<'a>>(&self, d: D) -> Result<World, D::Error> {
        let world = World::new(0);
        let visitor = WorldVisitor {
            persist: self,
            world,
            ids_initialized: false,
            index_initialized: false,
        };
        let world = d.deserialize_map(visitor)?;

        Ok(world)
    }

    fn visit_map_value<'de, A>(
        &self,
        key: &str,
        map: &mut A,
        world: &mut World,
    ) -> Result<(), A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let tname = entry_name::<T>(self.ty);
        if tname != key {
            if let Some(next) = &self.next {
                next.visit_map_value(key, map, world)?;
            }
            return Ok(());
        }
        #[cfg(feature = "tracing")]
        tracing::trace!(name = &tname, "Deserializing");
        match self.ty {
            SerTy::Component => {
                let values: Vec<(EntityId, T)> = map.next_value()?;
                for (id, value) in values {
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
        tracing::trace!(name = &tname, "Deserializing done");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut world0 = World::new(10);

        for i in 0u32..10u32 {
            let id = world0.insert_entity();
            world0.set_component(id, 42i32).unwrap();
            world0.set_component(id, i).unwrap();
            world0.set_component(id, Foo { value: i }).unwrap();
            world0.set_component(id, Bar::Baz).unwrap();
        }

        let p = WorldPersister::new()
            .with_component::<i32>()
            .with_component::<Foo>()
            .with_component::<Bar>();
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
        let mut world0 = World::new(10);

        for i in 0u32..10u32 {
            let id = world0.insert_entity();
            world0.set_component(id, 42i32).unwrap();
            world0.set_component(id, i).unwrap();
            world0.set_component(id, Foo { value: i }).unwrap();
        }

        let p = WorldPersister::new()
            .with_component::<i32>()
            .with_component::<Foo>();

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
            let id = world0.insert_entity();
            world0.set_component(id, 42i32).unwrap();
            world0.set_component(id, i).unwrap();
            world0.set_component(id, Foo { value: i }).unwrap();
        }

        world0.insert_resource(Foo { value: 69 });

        let p = WorldPersister::new()
            .with_component::<Foo>()
            .with_resource::<Foo>();

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
        let mut world0 = World::new(400);

        let mut ids = Vec::with_capacity(100);
        for i in 0u32..100u32 {
            let id = world0.insert_entity();
            world0.set_component(id, 42i32).unwrap();
            world0.set_component(id, i).unwrap();
            world0.set_component(id, Foo { value: i }).unwrap();
            ids.push(id);
        }
        // add some entities that are not saved
        for i in 0u32..100u32 {
            let id = world0.insert_entity();
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
            .with_component::<Foo>()
            .with_component::<i32>();

        let mut pl = Vec::<u8>::new();
        let mut s = serde_json::Serializer::pretty(&mut pl);

        p.save(&mut s, &world0).unwrap();

        // let pretty = std::str::from_utf8(&pl).unwrap();
        // println!("{}", pretty);

        let mut world1 = p
            .load(&mut serde_json::Deserializer::from_reader(pl.as_slice()))
            .unwrap();

        for i in 0..20 {
            let id0 = world0.insert_entity();
            let id1 = world1.insert_entity();
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
            let id0 = world0.insert_entity();
            let id1 = world1.insert_entity();

            assert_eq!(id0, id1, "#{}: expected: {} actual: {}", i, id0, id1,);
        }
    }

    #[test]
    #[cfg_attr(feature = "tracing", tracing_test::traced_test)]
    fn can_serde_multiple_resources_test() {
        // regression test: had a bug where the first resource would not be deserialized

        let mut world0 = World::new(4);
        world0.insert_resource(42i64);
        world0.insert_resource(69u32);

        let p = WorldPersister::new()
            .with_resource::<u32>()
            .with_resource::<i64>();

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

    #[test]
    #[cfg_attr(feature = "tracing", tracing_test::traced_test)]
    #[cfg(feature = "clone")]
    fn can_clone_deserialized_world_test() {
        let mut world0 = World::new(100);

        for i in 0u32..100u32 {
            let id = world0.insert_entity();
            world0.set_component(id, 42i32).unwrap();
            world0.set_component(id, i).unwrap();
            world0.set_component(id, Foo { value: i }).unwrap();
        }

        let p = WorldPersister::new()
            .with_component::<Foo>()
            .with_component::<i32>();

        let mut result = Vec::<u8>::new();
        let mut s = bincode::Serializer::new(&mut result, bincode::config::DefaultOptions::new());
        p.save(&mut s, &world0).unwrap();

        let world1 = p
            .load(&mut bincode::de::Deserializer::from_slice(
                result.as_slice(),
                bincode::config::DefaultOptions::new(),
            ))
            .unwrap();

        let _world2 = world1.clone();
    }
}
