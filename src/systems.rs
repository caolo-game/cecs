use crate::{query::WorldQuery, World};

pub trait System<'a> {
    type Query: WorldQuery<'a>;

    fn execute(&mut self, world: &'a World);
}
