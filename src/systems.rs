use std::{any::TypeId, collections::HashSet, ptr::NonNull};

use crate::World;

pub trait System<'a> {
    fn execute(&mut self, world: &'a World);
    fn exclusive_components(&self) -> HashSet<TypeId>;
    fn exclusive_resources(&self) -> HashSet<TypeId>;
}

pub struct SystemStage<'a> {
    pub systems: Vec<NonNull<dyn System<'a>>>,
}
