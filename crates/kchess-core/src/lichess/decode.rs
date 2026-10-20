//! Serde building blocks for reading Lichess answers into typed shapes: bounded strings, keys
//! that may be absent but not null, and timestamps sent as JSON numbers.
//!
//! A plain `Option<T>` field (with `#[serde(default)]`) reads an absent key and null as None. A
//! field marked `#[serde(default, deserialize_with = "present")]` reads an absent key as None
//! but rejects null. A [`Loose`] field never fails: a value of another shape reads as None.
//! Unknown keys are ignored.

use serde::de::{DeserializeOwned, Error};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// A string of at most `MAX` UTF-16 code units (the length JavaScript counts).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Text<const MAX: usize>(pub String);

impl<'de, const MAX: usize> Deserialize<'de> for Text<MAX> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.encode_utf16().count() > MAX {
            return Err(D::Error::custom(format!("longer than {MAX} characters")));
        }
        Ok(Text(text))
    }
}

impl<const MAX: usize> From<Text<MAX>> for String {
    fn from(text: Text<MAX>) -> String {
        text.0
    }
}

impl<const MAX: usize> std::ops::Deref for Text<MAX> {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

/// A whole number of milliseconds read from any JSON number (the fraction is dropped).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Millis(pub i64);

impl<'de> Deserialize<'de> for Millis {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        f64::deserialize(deserializer).map(|ms| Millis(ms as i64))
    }
}

/// A value that reads as `T` when it has that shape, and as None otherwise (never an error).
#[derive(Clone, Debug, PartialEq)]
pub struct Loose<T>(pub Option<T>);

impl<T> Default for Loose<T> {
    fn default() -> Self {
        Loose(None)
    }
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for Loose<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Value::deserialize(deserializer).map(|value| Loose(read(&value)))
    }
}

/// For an optional key that must not be null: with `#[serde(default)]` an absent key is None,
/// a present one must read as `T`.
pub fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// `value` read as `T`, or None when it does not have that shape.
pub fn read<T: DeserializeOwned>(value: &Value) -> Option<T> {
    T::deserialize(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Shape {
        name: Text<3>,
        #[serde(default, deserialize_with = "present")]
        strict: Option<u8>,
        #[serde(default)]
        loose: Option<u8>,
        #[serde(default)]
        any: Loose<u8>,
        at: Millis,
    }

    #[test]
    fn keys_read_by_their_rules() {
        let shape: Option<Shape> = read(&json!({ "name": "abc", "at": 1.9, "extra": true }));
        assert_eq!(
            shape,
            Some(Shape {
                name: Text("abc".into()),
                strict: None,
                loose: None,
                any: Loose(None),
                at: Millis(1),
            })
        );
        assert!(read::<Shape>(&json!({ "name": "abcd", "at": 0 })).is_none());
        // UTF-16 units: one astral character counts twice.
        assert!(read::<Shape>(&json!({ "name": "😀😀", "at": 0 })).is_none());
        assert!(read::<Shape>(&json!({ "name": "a", "at": 0, "strict": null })).is_none());
        assert!(read::<Shape>(&json!({ "name": "a", "at": 0, "loose": null })).is_some());
        assert!(read::<Shape>(&json!({ "name": "a" })).is_none());
        let any =
            |value| read::<Shape>(&json!({ "name": "a", "at": 0, "any": value })).map(|s| s.any);
        assert_eq!(any(json!(7)), Some(Loose(Some(7))));
        assert_eq!(any(json!("7")), Some(Loose(None)));
    }
}
