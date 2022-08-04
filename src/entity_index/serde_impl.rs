use serde::{
    de::{self, Visitor},
    ser::SerializeStruct,
    Deserialize, Serialize,
};
use std::borrow::Cow;

use super::*;

impl Serialize for Entry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        debug_assert!(
            size_of::<EntryData>() >= size_of::<FreeList>(),
            "code assumes that the data union member is the larger"
        );
        let mut state = serializer.serialize_struct("Entry", 2)?;

        state.serialize_field("gen", &self.gen)?;
        state.serialize_field("free_list", &unsafe { self.data.free_list })?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for Entry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct EntryVisitor;
        impl<'de> Visitor<'de> for EntryVisitor {
            type Value = Entry;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("struct Entry")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let gen = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::missing_field("gen"))?;
                let free_list = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::missing_field("free_list"))?;

                Ok(Entry {
                    gen,
                    data: EntryData { free_list },
                })
            }
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut gen = None;
                let mut free_list = None;

                while let Some(key) = map.next_key::<Cow<str>>()? {
                    match key.as_ref() {
                        "gen" => gen = Some(map.next_value()?),
                        "free_list" => free_list = Some(map.next_value()?),
                        _ => {}
                    }
                }

                let gen = gen.ok_or_else(|| de::Error::missing_field("gen"))?;
                let free_list = free_list.ok_or_else(|| de::Error::missing_field("free_list"))?;

                Ok(Entry {
                    gen,
                    data: EntryData { free_list },
                })
            }
        }
        const FIELDS: &[&str] = &["gen", "free_list"];
        deserializer.deserialize_struct("Entry", FIELDS, EntryVisitor)
    }
}

impl Serialize for EntityIndex {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("EntityIndex", 4)?;
        state.serialize_field("cap", &self.cap)?;
        // serialize the first free item, too
        state.serialize_field("free_list", &self.free_list)?;
        state.serialize_field("count", &self.count)?;
        // serialize all the entries, because when replaying the order of the free list must stay
        // consistent!
        state.serialize_field("entries", self.entries())?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for EntityIndex {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct HandleTableVisitor;

        impl<'de> Visitor<'de> for HandleTableVisitor {
            type Value = EntityIndex;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("struct EntityIndex")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let cap = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::missing_field("cap"))?;
                let free_list = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::missing_field("free_list"))?;
                let count = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::missing_field("count"))?;
                let entries: Vec<Entry> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::missing_field("entries"))?;

                let mut result = EntityIndex::new(cap);
                result.free_list = free_list;
                result.count = count;

                if result.cap as usize != entries.len() {
                    return Err(de::Error::custom(
                        "expected `cap` and number of `entries` to match.",
                    ));
                }

                result.entries_mut().copy_from_slice(&entries);

                Ok(result)
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut entries: Option<Vec<Entry>> = None;
                let mut cap = None;
                let mut free_list = None;
                let mut count = None;

                while let Some(key) = map.next_key::<Cow<str>>()? {
                    match key.as_ref() {
                        "cap" => {
                            if cap.is_some() {
                                return Err(de::Error::duplicate_field("cap"));
                            }
                            cap = Some(map.next_value()?);
                        }
                        "entries" => {
                            if entries.is_some() {
                                return Err(de::Error::duplicate_field("entries"));
                            }
                            entries = Some(map.next_value()?);
                        }
                        "free_list" => {
                            if free_list.is_some() {
                                return Err(de::Error::duplicate_field("free_list"));
                            }
                            free_list = Some(map.next_value()?);
                        }
                        "count" => {
                            if count.is_some() {
                                return Err(de::Error::duplicate_field("count"));
                            }
                            count = Some(map.next_value()?);
                        }
                        _ => {}
                    }
                }

                let mut result =
                    EntityIndex::new(cap.ok_or_else(|| de::Error::missing_field("cap"))?);
                result.free_list =
                    free_list.ok_or_else(|| de::Error::missing_field("free_list"))?;
                result.count = count.ok_or_else(|| de::Error::missing_field("count"))?;

                let entries = entries.ok_or_else(|| de::Error::missing_field("entries"))?;
                if result.cap as usize != entries.len() {
                    return Err(de::Error::custom(
                        "expected `cap` and number of `entries` to match.",
                    ));
                }

                result.entries_mut().copy_from_slice(&entries);

                Ok(result)
            }
        }

        const FIELDS: &[&str] = &["cap", "entries", "free_list", "count"];
        deserializer.deserialize_struct("EntityIndex", FIELDS, HandleTableVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_de_serialize() {
        let mut table = EntityIndex::new(200);

        for _ in 0..100 {
            let id = table.allocate().unwrap();
            table.free(id);
            table.allocate().unwrap();
        }

        let payload = bincode::serialize(&table).unwrap();

        let mut res: EntityIndex = bincode::deserialize(&payload).unwrap();

        assert_eq!(res.len(), table.len());

        for _ in 0..50 {
            let id1 = table.allocate().unwrap();
            let id2 = res.allocate().unwrap();
            assert_eq!(id1, id2);
        }
    }
}
