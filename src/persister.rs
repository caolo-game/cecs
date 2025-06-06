//! Provides utilities to save and load Worlds.
//! EntityIds are not stable between loads and saves!
//!
use serde::{
    de::{DeserializeOwned, Visitor},
    ser::SerializeMap,
    Serialize,
};
use std::marker::PhantomData;

use crate::{entity_id::EntityId, prelude::Query, Component, World};

pub struct WorldPersister<T = (), P = ()> {
    next: Option<Box<P>>,
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
            next: Some(Box::new(self)),
            ty: SerTy::Component,
            _m: PhantomData,
        }
    }

    pub fn with_resource<U: Component + Serialize + DeserializeOwned>(
        self,
    ) -> WorldPersister<U, Self> {
        WorldPersister {
            depth: self.depth + 1,
            next: Some(Box::new(self)),
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
            self.persist
                .visit_map_value(key.as_ref(), &mut map, &mut self.world)?;
        }

        Ok(self.world)
    }
}

fn save_impl<T: Component + Serialize, S: serde::Serializer>(
    ty: SerTy,
    s: &mut S::SerializeMap,
    world: &World,
) -> Result<(), S::Error> {
    let tname = entry_name::<T>(ty);

    #[cfg(feature = "tracing")]
    tracing::trace!(name = &tname, "Serializing");
    match ty {
        SerTy::Component => {
            // TODO: save iterator or adapter pls
            let values: Vec<(EntityId, &T)> = Query::<(EntityId, &T)>::new(world)
                .iter()
                .inspect(|(_id, _)| {
                    #[cfg(feature = "tracing")]
                    tracing::trace!(id = tracing::field::display(_id), "Serializing entity");
                })
                .collect();
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
    Ok(())
}

fn visit_map_value_impl<'de, A, T: Component + DeserializeOwned>(
    ty: SerTy,
    map: &mut A,
    world: &mut World,
) -> Result<(), A::Error>
where
    A: serde::de::MapAccess<'de>,
{
    let tname = entry_name::<T>(ty);
    #[cfg(feature = "tracing")]
    tracing::trace!(name = &tname, "Deserializing");
    match ty {
        SerTy::Component => {
            let values: Vec<(EntityId, T)> = map.next_value()?;
            #[cfg(feature = "tracing")]
            tracing::trace!("Got {} entries", values.len());
            for (id, value) in values {
                if !world.is_id_valid(id) {
                    #[cfg(feature = "tracing")]
                    tracing::trace!("Inserting id {id}");
                    world.insert_id(id).unwrap();
                }
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
        let mut s = s.serialize_map(Some(self.depth))?;
        self.save_entry::<S>(&mut s, world)?;
        s.end()
    }

    fn save_entry<S: serde::Serializer>(
        &self,
        s: &mut S::SerializeMap,
        world: &World,
    ) -> Result<(), S::Error> {
        save_impl::<T, S>(self.ty, s, world)?;

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
            // missing deserializaers are not an error,
            // this lets clients add additional data into persistent payloads
            // or ignore obsolete types
            return Ok(());
        }
        return visit_map_value_impl::<A, T>(self.ty, map, world);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;
    use std::collections::HashSet;

    #[derive(serde_derive::Serialize, serde_derive::Deserialize, Clone)]
    struct Foo {
        value: u32,
    }

    #[derive(serde_derive::Serialize, serde_derive::Deserialize, Clone, PartialEq, Eq)]
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

        let mut result1 = Vec::<u8>::new();
        let mut s = bincode::Serializer::new(&mut result1, bincode::config::DefaultOptions::new());
        p.save(&mut s, &world1).unwrap();

        assert_eq!(
            result, result1,
            "Re-serializing the deserialized world should yield the same payload"
        );

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
    fn ids_are_stable_test() {
        let mut world0 = World::new(10);

        for i in 0u32..10u32 {
            // produce some gaps
            for _ in 0..4 {
                let _id = world0.insert_entity();
            }
            // bump generation
            for _ in 0..4 {
                let id = world0.insert_entity();
                world0.delete_entity(id).unwrap();
            }
            let id = world0.insert_entity();
            world0.set_component(id, 42i32).unwrap();
            // produce multiple archetypes
            if i % 2 == 0 {
                world0.set_component(id, i).unwrap();
            }
            if i % 3 == 0 {
                world0.set_component(id, 4.2f32).unwrap();
            }
            world0.set_component(id, Foo { value: i }).unwrap();
        }

        let p = WorldPersister::new()
            .with_component::<i32>()
            .with_component::<Foo>();

        let mut result = Vec::<u8>::new();
        let mut s = bincode::Serializer::new(&mut result, bincode::config::DefaultOptions::new());
        p.save(&mut s, &world0).unwrap();

        let mut world1 = p
            .load(&mut bincode::de::Deserializer::from_slice(
                result.as_slice(),
                bincode::config::DefaultOptions::new(),
            ))
            .unwrap();

        type Q<'a> = Query<
            'a,
            (EntityId, ArchetypeHash, Option<&'a u32>, Option<&'a f32>),
            (With<i32>, With<Foo>),
        >;

        let u32_hash = crate::hash_ty::<u32>();
        let f32_hash = crate::hash_ty::<f32>();

        let q0 = Q::new(&world0);
        let q1 = Q::new(&world1);

        assert_eq!(q0.count(), q1.count());

        for ((id0, mut h0, c, d), (id1, h1, _, _)) in
            Q::new(&world0).iter().zip(Q::new(&world1).iter())
        {
            assert_eq!(id0, id1);
            if c.is_some() {
                // unserialized components will be lost
                h0.0 ^= u32_hash;
            }
            if d.is_some() {
                // unserialized components will be lost
                h0.0 ^= f32_hash;
            }
            assert_eq!(h0, h1);
        }
        let mut ids = Query::<EntityId>::new(&world1)
            .iter()
            .collect::<HashSet<_>>();

        for _ in 0..1024 {
            let id = world1.insert_entity();
            assert!(!ids.contains(&id));
            ids.insert(id);
        }

        assert_eq!(
            Query::<&u32>::new(&world1).count(),
            0,
            "Assumes that non registered types are not (de)serialized"
        );
    }
}
