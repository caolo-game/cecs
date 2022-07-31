use anyhow::Context;
use serde::{
    de::{DeserializeOwned, Visitor},
    ser::{SerializeMap, SerializeSeq},
    Serialize,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
};

use crate::{entity_id::EntityId, prelude::Query, Component, World};

// FIXME: serializing to Value so we can serialize to another format is yucky
// this is here because we need a type erased intermediate value for serializing that itself
// implements Serialize
//
type ErasedValue = ron::Value;

/// Maps saved entity ids to their new entity ids
type EntityMap = HashMap<EntityId, EntityId>;

pub struct WorldPersister {
    savers: RefCell<BTreeMap<String, ErasedSaver>>,
    loaders: RefCell<BTreeMap<String, ErasedLoader>>,
    world: *mut World,
}

impl Default for WorldPersister {
    fn default() -> Self {
        Self {
            world: std::ptr::null_mut(),
            savers: RefCell::default(),
            loaders: RefCell::default(),
        }
    }
}

impl WorldPersister {
    pub fn register_component<T: Component + Serialize + DeserializeOwned>(&mut self) {
        let name = std::any::type_name::<T>();
        let name = format!("Component_{}", name);
        let mut savers = self.savers.borrow_mut();
        let mut loaders = self.loaders.borrow_mut();
        savers.insert(name.clone(), ErasedSaver::component::<T>());
        loaders.insert(name, ErasedLoader::component::<T>());
    }

    pub fn register_resource<T: Component + Serialize + DeserializeOwned>(&mut self) {
        let name = std::any::type_name::<T>();
        let name = format!("Resource_{}", name);
        let mut savers = self.savers.borrow_mut();
        let mut loaders = self.loaders.borrow_mut();
        savers.insert(name.clone(), ErasedSaver::resource::<T>());
        loaders.insert(name, ErasedLoader::resource::<T>());
    }

    pub fn set_world<'a>(&'a mut self, world: &'a mut World) {
        self.world = world as *mut _;
    }

    pub fn save<S: serde::Serializer>(&self, s: S, world: &World) -> Result<S::Ok, S::Error> {
        let mut savers = self.savers.borrow_mut();
        let mut root = s.serialize_map(Some(savers.len()))?;
        for (name, saver) in savers.iter_mut() {
            saver.world = world as *const _;
            let result = root.serialize_entry(name, saver);
            saver.world = std::ptr::null();
            result?;
        }
        root.end()
    }

    /// Ids of entities are not guaranteed to be the same as when they were saved
    /// the returned EntityMap provides a mapping from the old ids to the new ids
    pub fn load<'a, D: serde::Deserializer<'a>>(
        &'a self,
        d: D,
    ) -> Result<(World, EntityMap), D::Error> {
        struct WorldVisitor<'a> {
            persist: &'a WorldPersister,
            world: World,
            entity_map: EntityMap,
        }

        impl<'a, 'de: 'a> Visitor<'de> for WorldVisitor<'a> {
            type Value = (World, EntityMap);

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("Serialized World")
            }

            fn visit_map<A>(mut self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut loaders = self.persist.loaders.borrow_mut();

                while let Some(key) = map.next_key::<&str>()? {
                    // TODO ignore unknown fields for now, maybe return error in future?
                    if let Some(loader) = loaders.get_mut(key) {
                        let intermediate: ErasedValue = map.next_value()?;
                        loader.world = &mut self.world as *mut _;
                        loader.entity_map = &mut self.entity_map as *mut _;

                        let result = loader
                            .load(intermediate)
                            .map_err(|err| serde::de::Error::custom(err));

                        loader.entity_map = std::ptr::null_mut();
                        loader.world = std::ptr::null_mut();

                        result?;
                    }
                }

                Ok((self.world, self.entity_map))
            }
        }

        let visitor = WorldVisitor {
            persist: self,
            world: World::new(100),
            entity_map: Default::default(),
        };

        d.deserialize_map(visitor)
    }
}

struct ErasedLoader {
    world: *mut World,
    entity_map: *mut EntityMap,
    insert: fn(&mut World, &mut EntityMap, ErasedValue) -> anyhow::Result<()>,
}

impl ErasedLoader {
    /// Loads a resource
    pub fn resource<T: Component + DeserializeOwned>() -> Self {
        Self {
            world: std::ptr::null_mut(),
            entity_map: std::ptr::null_mut(),
            insert: |world, _entity_map, values| {
                let values: [T; 1] = values
                    .into_rust()
                    .with_context(|| "Failed to deserialize component list")?;
                for res in values {
                    world.insert_resource(res);
                }
                Ok(())
            },
        }
    }

    /// Loads list of components
    pub fn component<T: Component + DeserializeOwned>() -> Self {
        Self {
            world: std::ptr::null_mut(),
            entity_map: std::ptr::null_mut(),
            insert: |world, entity_map, values| {
                let values: Vec<(EntityId, T)> = values
                    .into_rust()
                    .with_context(|| "Failed to deserialize component list")?;
                for (id, component) in values {
                    let new_id = *entity_map
                        .entry(id)
                        .or_insert_with(|| world.insert_entity().unwrap());

                    assert!(world.is_id_valid(new_id));

                    world.set_component(new_id, component).unwrap();
                }
                Ok(())
            },
        }
    }

    pub fn load(&mut self, values: ErasedValue) -> anyhow::Result<()> {
        assert_ne!(self.world, std::ptr::null_mut());
        assert_ne!(self.entity_map, std::ptr::null_mut());
        let world = unsafe { &mut *self.world };
        let entity_map = unsafe { &mut *self.entity_map };
        (self.insert)(world, entity_map, values)
    }
}

struct ErasedSaver {
    world: *const World,
    for_each: fn(&World, &mut dyn FnMut(ErasedValue) -> anyhow::Result<()>) -> anyhow::Result<()>,
    count: fn(&World) -> usize,
}

impl ErasedSaver {
    /// Saves list of components
    pub fn component<T: Component + Serialize>() -> Self {
        Self {
            world: std::ptr::null(),
            for_each: |world, fun| {
                let mut buffer = Vec::with_capacity(1024);
                for (id, val) in Query::<(EntityId, &T)>::new(world).iter() {
                    buffer.clear();
                    ron::ser::to_writer(&mut buffer, &(id, val))
                        .with_context(|| "Failed to serialize into ErasedValue")?;
                    let value: ErasedValue = ron::de::from_reader(buffer.as_slice()).unwrap();
                    fun(value)?;
                }
                Ok(())
            },
            count: |world| Query::<(EntityId, &T)>::new(world).count(),
        }
    }

    /// Saves a resource
    ///
    /// If resource isn't inserted into the `World` then this is a noop
    pub fn resource<T: Component + Serialize>() -> Self {
        Self {
            world: std::ptr::null(),
            for_each: |world, fun| {
                if let Some(res) = world.get_resource::<T>() {
                    let mut buffer = Vec::with_capacity(1024);
                    ron::ser::to_writer(&mut buffer, res)
                        .with_context(|| "Failed to serialize resource into ErasedValue")?;
                    let value: ErasedValue = ron::de::from_reader(buffer.as_slice()).unwrap();
                    fun(value)?;
                }
                Ok(())
            },
            count: |world| world.get_resource::<T>().is_some() as usize,
        }
    }

    /// always waves slice of values, even for single items
    pub fn save<'a, S: serde::Serializer>(
        &'a self,
        s: S,
        world: &World,
    ) -> Result<S::Ok, S::Error> {
        let mut s = s.serialize_seq(Some((self.count)(world)))?;
        (self.for_each)(world, &mut |value: ErasedValue| {
            s.serialize_element(&value)
                .map_err(|err| anyhow::anyhow!("Failed to serialize ErasedValue: {:?}", err))?;
            Ok(())
        })
        .unwrap(); // TODO: return this error
        s.end()
    }
}

impl Serialize for ErasedSaver {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        assert_ne!(self.world, std::ptr::null());
        unsafe { self.save(s, &*self.world) }
    }
}

impl Serialize for WorldPersister {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        assert_ne!(
            self.world,
            std::ptr::null_mut(),
            "Call set_world before serializing!"
        );
        let world = self.world;
        unsafe { self.save(s, &*world) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize, Clone)]
    struct Foo {
        value: u32,
    }

    #[test]
    fn save_load_json_test() {
        let mut world0 = World::new(4);

        for i in 0u32..10u32 {
            let id = world0.insert_entity().unwrap();
            world0.set_component(id, 42i32).unwrap();
            world0.set_component(id, i).unwrap();
            world0.set_component(id, Foo { value: i }).unwrap();
        }

        let mut p = WorldPersister::default();
        p.register_component::<i32>();
        p.register_component::<Foo>();
        p.set_world(&mut world0);
        let result = serde_json::to_string_pretty(&p).unwrap();

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

        let mut p = WorldPersister::default();
        p.register_component::<i32>();
        p.register_component::<Foo>();
        p.set_world(&mut world0);
        let result = bincode::serialize(&p).unwrap();

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

        let mut p = WorldPersister::default();
        p.register_component::<Foo>();
        p.register_resource::<Foo>();
        p.set_world(&mut world0);

        let pl = serde_json::to_string_pretty(&p).unwrap();

        let (world1, id_map) = p
            .load(&mut serde_json::Deserializer::from_str(pl.as_str()))
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
