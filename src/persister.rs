use serde::{
    ser::{SerializeMap, SerializeSeq},
    Deserialize, Serialize,
};
use std::{cell::RefCell, collections::BTreeMap};

use crate::{entity_id::EntityId, prelude::Query, Component, World};

type IntermediateValue = ron::Value;

pub struct WorldPersister {
    savers: RefCell<BTreeMap<&'static str, ErasedSaver>>,
    world: *mut World,
}

impl Default for WorldPersister {
    fn default() -> Self {
        Self {
            world: std::ptr::null_mut(),
            savers: RefCell::default(),
        }
    }
}

impl WorldPersister {
    pub fn register_component<'a, T: Component + Serialize + Deserialize<'a>>(&'a mut self) {
        let name = std::any::type_name::<T>();
        let mut savers = self.savers.borrow_mut();
        savers.insert(name, ErasedSaver::component::<T>());
    }

    pub fn set_world<'a>(&'a mut self, world: &'a mut World) {
        self.world = world as *mut _;
    }

    pub fn save<'a, S: serde::Serializer>(
        &'a self,
        s: S,
        world: &World,
    ) -> Result<S::Ok, S::Error> {
        let mut savers = self.savers.borrow_mut();
        let mut root = s.serialize_map(Some(savers.len()))?;
        root.serialize_entry("__ids", &world.entity_ids)?;
        for (name, saver) in savers.iter_mut() {
            saver.world = world as *const _;
            root.serialize_entry(name, saver)?;
            saver.world = std::ptr::null();
        }
        root.end()
    }
}

struct ErasedSaver {
    world: *const World,
    // FIXME: serializing to Value so we can serialize to another format is yucky
    // this is here because we need a type erased intermediate value for serializing that itself
    // implements Serialize
    //
    // TODO: return error?
    for_each: fn(&World, &mut dyn FnMut(IntermediateValue)),
    count: fn(&World) -> usize,
}

impl ErasedSaver {
    /// Saves list of components
    pub fn component<T: Component + Serialize>() -> Self {
        Self {
            world: std::ptr::null(),
            for_each: |world, fun| {
                for (id, val) in Query::<(EntityId, &T)>::new(world).iter() {
                    let value = ron::to_string(&(id, val)).unwrap();
                    let value = ron::from_str(&value).unwrap();
                    fun(value);
                }
            },
            count: |world| Query::<(EntityId, &T)>::new(world).count(),
        }
    }

    pub fn save<'a, S: serde::Serializer>(
        &'a self,
        s: S,
        world: &World,
    ) -> Result<S::Ok, S::Error> {
        let mut s = s.serialize_seq(Some((self.count)(world)))?;
        (self.for_each)(world, &mut |value: IntermediateValue| {
            s.serialize_element(&value).unwrap();
        });
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
        assert_ne!(self.world, std::ptr::null_mut());
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
    fn save_test() {
        let mut world = World::new(4);

        for i in 0u32..10u32 {
            let id = world.insert_entity().unwrap();
            world.set_component(id, 42i32).unwrap();
            world.set_component(id, i).unwrap();
            world.set_component(id, Foo { value: i }).unwrap();
        }

        let mut p = WorldPersister::default();
        p.register_component::<i32>();
        p.register_component::<Foo>();
        p.set_world(&mut world);

        let result = serde_json::to_string_pretty(&p).unwrap();

        panic!("{}", result);
    }
}
