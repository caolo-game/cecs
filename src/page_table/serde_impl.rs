use std::marker::PhantomData;

use serde::{de::Visitor, ser::SerializeMap, Deserialize, Serialize};

use crate::RowIndex;

use super::Page;

impl<T: Serialize> Serialize for Page<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_map(Some(self.len()))?;
        for (id, val) in self.iter() {
            state.serialize_entry(&id, val)?;
        }

        state.end()
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Page<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct EntryVisitor<U> {
            _m: PhantomData<U>,
        }

        impl<U> EntryVisitor<U> {
            fn new() -> Self {
                Self {
                    _m: Default::default(),
                }
            }
        }

        impl<'de, T: Deserialize<'de>> Visitor<'de> for EntryVisitor<T> {
            type Value = Page<T>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("struct Page")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut result = Page::new();
                while let Some((id, val)) = map.next_entry::<RowIndex, _>()? {
                    result.insert(id as usize, val);
                }
                Ok(result)
            }
        }

        deserializer.deserialize_map(EntryVisitor::<T>::new())
    }
}

#[cfg(test)]
mod tests {
    use crate::page_table::PageTable;

    #[test]
    fn page_table_serde() {
        let mut table = PageTable::new(1024);

        for i in 0..10_000 {
            table.insert(i, i);
        }

        let mut payload = Vec::with_capacity(1024);
        bincode::serialize_into(&mut payload, &table).unwrap();

        let deser: PageTable<u32> = bincode::deserialize_from(&payload[..]).unwrap();

        assert_eq!(table.len(), deser.len());

        for (a, b) in table.iter().zip(deser.iter()) {
            assert_eq!(a.0, b.0);
            assert_eq!(a.1, b.1);
        }
    }
}
