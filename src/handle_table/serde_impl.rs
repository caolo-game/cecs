use serde::{
    de::{self, Visitor},
    ser::SerializeStruct,
    Deserialize, Serialize,
};
use smallvec::SmallVec;

use super::{Entry, HandleTable};

impl Serialize for HandleTable {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("HandleTable", 4)?;
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

impl<'de> Deserialize<'de> for HandleTable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct HandleTableVisitor;

        impl<'de> Visitor<'de> for HandleTableVisitor {
            type Value = HandleTable;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("struct HandleTable")
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
                let entries: SmallVec<[_; 64]> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::missing_field("entries"))?;

                let mut result = HandleTable::new(cap);
                result.free_list = free_list;
                result.count = count;

                if result.cap as usize != entries.len() {
                    return Err(de::Error::custom(
                        "expected `cap` and number of `entries` to match.",
                    ));
                }

                for (i, entry) in entries.into_iter().enumerate() {
                    result.entries_mut()[i] = entry;
                }

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

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
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
                    HandleTable::new(cap.ok_or_else(|| de::Error::missing_field("cap"))?);
                result.free_list =
                    free_list.ok_or_else(|| de::Error::missing_field("free_list"))?;
                result.count = count.ok_or_else(|| de::Error::missing_field("count"))?;

                let entries = entries.ok_or_else(|| de::Error::missing_field("entries"))?;
                if result.cap as usize != entries.len() {
                    return Err(de::Error::custom(
                        "expected `cap` and number of `entries` to match.",
                    ));
                }

                for (i, entry) in entries.into_iter().enumerate() {
                    result.entries_mut()[i] = entry;
                }

                Ok(result)
            }
        }

        const FIELDS: &[&str] = &["cap", "entries", "free_list", "count"];
        deserializer.deserialize_struct("HandleTable", FIELDS, HandleTableVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_de_serialize() {
        let mut table = HandleTable::new(2_000);

        for _ in 0..1000 {
            let id = table.alloc().unwrap();
            table.free(id);
            table.alloc().unwrap();
        }

        let payload = bincode::serialize(&table).unwrap();

        let mut res: HandleTable = bincode::deserialize(&payload).unwrap();

        assert_eq!(res.len(), table.len());

        for _ in 0..500 {
            let id1 = table.alloc().unwrap();
            let id2 = res.alloc().unwrap();
            assert_eq!(id1, id2);
        }
    }
}
