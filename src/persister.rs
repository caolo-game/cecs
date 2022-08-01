use serde::{
    de::{DeserializeOwned, Visitor},
    ser::SerializeMap,
    Serialize,
};
use std::{collections::HashMap, marker::PhantomData};

use crate::{entity_id::EntityId, prelude::Query, Component, World};

/// Maps saved entity ids to their loaded entity ids
pub type EntityMap = HashMap<EntityId, EntityId>;

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

    fn load<'a, D: serde::Deserializer<'a>>(&'a self, d: D)
        -> Result<(World, EntityMap), D::Error>;

    fn visit_map_value<'de, A>(
        &self,
        key: &str,
        map: &mut A,
        world: &mut World,
        id_map: &mut EntityMap,
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

    fn load<'a, D: serde::Deserializer<'a>>(
        &'a self,
        _d: D,
    ) -> Result<(World, EntityMap), D::Error> {
        unreachable!()
    }

    fn visit_map_value<'de, A>(
        &self,
        _key: &str,
        _map: &mut A,
        _world: &mut World,
        _id_map: &mut EntityMap,
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
    entity_map: EntityMap,
}

impl<'a, 'de: 'a, T, P> Visitor<'de> for WorldVisitor<'a, T, P>
where
    P: WorldSerializer,
    T: Component + DeserializeOwned + Serialize,
{
    type Value = (World, EntityMap);

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("Serialized World")
    }

    fn visit_map<A>(mut self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        while let Some(key) = map.next_key::<std::borrow::Cow<'de, str>>()? {
            self.persist.visit_map_value(
                key.as_ref(),
                &mut map,
                &mut self.world,
                &mut self.entity_map,
            )?;
        }

        Ok((self.world, self.entity_map))
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
        let mut s = s.serialize_map(Some(self.depth))?;
        self.save_entry::<S>(&mut s, world)?;
        s.end()
    }

    fn save_entry<S: serde::Serializer>(
        &self,
        s: &mut S::SerializeMap,
        world: &World,
    ) -> Result<(), S::Error> {
        let tname = entry_name::<T>(self.ty);
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
        if let Some(p) = self.next.as_ref() {
            p.save_entry::<S>(s, world)?;
        }
        Ok(())
    }

    fn load<'a, D: serde::Deserializer<'a>>(
        &'a self,
        d: D,
    ) -> Result<(World, EntityMap), D::Error> {
        let visitor = WorldVisitor {
            persist: self,
            world: World::new(100),
            entity_map: Default::default(),
        };

        d.deserialize_map(visitor)
    }

    fn visit_map_value<'de, A>(
        &self,
        key: &str,
        map: &mut A,
        world: &mut World,
        id_map: &mut EntityMap,
    ) -> Result<(), A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let tname = entry_name::<T>(self.ty);
        if tname != key {
            if let Some(next) = &self.next {
                next.visit_map_value(key, map, world, id_map)?;
            }
            return Ok(());
        }
        match self.ty {
            SerTy::Component => {
                let values: Vec<(EntityId, T)> = map.next_value()?;
                for (old_id, value) in values {
                    let id = *id_map.entry(old_id).or_insert_with(|| {
                        world
                            .insert_entity()
                            .expect("Failed to allocate new entity")
                    });

                    world.set_component(id, value).unwrap();
                }
            }
            SerTy::Resource => {
                let value: T = map.next_value()?;
                world.insert_resource(value);
            }
            SerTy::Noop => {}
        }

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

        let (world1, id_map) = p
            .load(&mut serde_json::Deserializer::from_str(result.as_str()))
            .unwrap();

        type QueryTuple<'a> = (EntityId, &'a i32, &'a Foo);

        for ((id0, i0, f0), (id1, i1, f1)) in Query::<QueryTuple>::new(&world0)
            .iter()
            .zip(Query::<QueryTuple>::new(&world1).iter())
        {
            let id01 = id_map[&id0];
            assert_eq!(id01, id1);
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

        let (world1, id_map) = p
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
            let id01 = id_map[&id0];
            assert_eq!(id01, id1);
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

        let (world1, id_map) = p
            .load(&mut serde_json::Deserializer::from_reader(pl.as_slice()))
            .unwrap();

        type QueryTuple<'a> = (EntityId, &'a Foo);

        for ((id0, f0), (id1, f1)) in Query::<QueryTuple>::new(&world0)
            .iter()
            .zip(Query::<QueryTuple>::new(&world1).iter())
        {
            let id01 = id_map[&id0];
            assert_eq!(id01, id1);
            assert_eq!(f0.value, f1.value);
        }

        assert_eq!(
            world1.get_resource::<Foo>().expect("foo not found").value,
            69
        );
    }
}
