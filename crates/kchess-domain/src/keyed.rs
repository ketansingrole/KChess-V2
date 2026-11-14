//! A JSON object kept in its keys' order with every value typed.

use serde::{Deserialize, Serialize};

/// A JSON object read and written in its keys' order, every value a `V` (values by game id,
/// by kind…); a key read twice keeps its first place and its last value, as in JavaScript.
#[derive(Debug, Clone, PartialEq)]
pub struct Keyed<V>(pub Vec<(String, V)>);

impl<V> Default for Keyed<V> {
    fn default() -> Self {
        Keyed(Vec::new())
    }
}

impl<V> Keyed<V> {
    pub fn get(&self, key: &str) -> Option<&V> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Sets `key` to `value`: in its place when it is there, else last.
    pub fn insert(&mut self, key: String, value: V) {
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some((_, known)) => *known = value,
            None => self.0.push((key, value)),
        }
    }

    /// The value of `key`, added last with `V::default()` when it is missing.
    pub fn entry(&mut self, key: &str) -> &mut V
    where
        V: Default,
    {
        let at = match self.0.iter().position(|(k, _)| k == key) {
            Some(at) => at,
            None => {
                self.0.push((key.to_string(), V::default()));
                self.0.len() - 1
            }
        };
        &mut self.0[at].1
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &V)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl<V: Serialize> Serialize for Keyed<V> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in &self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de, V: Deserialize<'de>> Deserialize<'de> for Keyed<V> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries<V>(std::marker::PhantomData<V>);
        impl<'de, V: Deserialize<'de>> serde::de::Visitor<'de> for Entries<V> {
            type Value = Keyed<V>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Keyed<V>, A::Error> {
                let mut keyed = Keyed(Vec::with_capacity(map.size_hint().unwrap_or(0)));
                while let Some((key, value)) = map.next_entry()? {
                    keyed.insert(key, value);
                }
                Ok(keyed)
            }
        }
        deserializer.deserialize_map(Entries(std::marker::PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keys come back in the order first read, each with its last value.
    #[test]
    fn reads_its_entries_in_their_first_order() {
        let empty = Keyed::<u32>::default();
        assert!(empty.is_empty());
        assert_eq!(empty.iter().count(), 0);
        let keyed: Keyed<u32> = serde_json::from_str(r#"{"b": 1, "a": 2, "b": 3}"#).unwrap();
        assert!(!keyed.is_empty());
        assert_eq!(keyed.iter().collect::<Vec<_>>(), [("b", &3), ("a", &2)]);
    }
}
