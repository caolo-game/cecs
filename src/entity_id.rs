pub type Index = u32;

const INDEX_BITS: u32 = 24;
const GEN_BITS: u32 = 32 - INDEX_BITS;
pub const ENTITY_INDEX_MASK: u32 = (1 << INDEX_BITS) - 1;
pub const ENTITY_GEN_MASK: u32 = (1 << GEN_BITS) - 1;

// store the index in the more significant bits for ordering
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Copy, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct EntityId(u32);

impl EntityId {
    pub fn new(index: Index, gen: u32) -> Self {
        #[cfg(not(target_endian = "little"))]
        compile_error!(
            "EntityId might need updating for big endian targets, please validate before using"
        );

        assert!(index <= ENTITY_INDEX_MASK);
        assert!(gen <= ENTITY_GEN_MASK);
        let index = index << GEN_BITS;
        Self(gen | index)
    }

    #[inline]
    pub fn index(self) -> Index {
        self.0 >> GEN_BITS
    }

    #[inline]
    pub fn gen(self) -> u32 {
        self.0 & ENTITY_GEN_MASK
    }

    #[inline]
    pub fn to_pair(self) -> (Index, u32) {
        (self.index(), self.gen())
    }
}

impl From<u32> for EntityId {
    fn from(i: u32) -> Self {
        Self(i)
    }
}

impl From<EntityId> for u32 {
    fn from(id: EntityId) -> Self {
        id.0
    }
}

impl Default for EntityId {
    fn default() -> Self {
        Self(!0)
    }
}

impl std::fmt::Debug for EntityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EntityId({})={}", self.0, self)
    }
}

impl std::fmt::Display for EntityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}:{}", self.gen(), self.index())
    }
}

#[cfg(test)]
mod tests {
    use super::EntityId;

    #[test]
    fn entity_id_cast_integer_consistent() {
        let a = EntityId::new(696969, 42);

        let id: u32 = a.into();
        let b: EntityId = id.into();

        assert_eq!(a, b);
    }

    #[test]
    fn entity_id_getters_test() {
        let idx = 4194303;
        let id = EntityId::new(idx, 100);

        assert_eq!(id.index(), idx);
        assert_eq!(id.gen(), 100);

        let id = EntityId::new(0, 100);

        assert_eq!(id.index(), 0);
        assert_eq!(id.gen(), 100);

        let id = EntityId::new(16, 100);

        assert_eq!(id.index(), 16);
        assert_eq!(id.gen(), 100);
    }
}
