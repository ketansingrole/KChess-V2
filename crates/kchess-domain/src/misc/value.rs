//! The values a frontend's arguments arrive as, and a serde [`Deserializer`] over them, so the
//! validators decode arguments into typed structs. An error names the path of the value it
//! rejected, for the validators that map it to their own message.

use std::fmt;

use serde::de::{
    self, DeserializeOwned, DeserializeSeed, Deserializer, IntoDeserializer, MapAccess, SeqAccess,
    Unexpected, Visitor,
};

/// A JSON-like value that also keeps what JSON cannot carry: `undefined` (a missing or
/// `undefined` property), NaN and the infinities (as `Num`), and binary data (a `Uint8Array`,
/// kept as its length and first bytes). Object properties keep their order.
#[derive(Clone, Debug, PartialEq)]
pub enum J {
    Undef,
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Bytes { len: usize, head: Vec<u8> },
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

impl J {
    /// The property `key` of an object, present even when its value is `undefined`.
    pub fn get(&self, key: &str) -> Option<&J> {
        match self {
            J::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            J::Str(s) => Some(s),
            _ => None,
        }
    }

    /// JavaScript truthiness (`!!value`).
    pub fn truthy(&self) -> bool {
        match self {
            J::Undef | J::Null => false,
            J::Bool(b) => *b,
            J::Num(n) => *n != 0.0 && !n.is_nan(),
            J::Str(s) => !s.is_empty(),
            J::Bytes { .. } | J::Arr(_) | J::Obj(_) => true,
        }
    }

    /// Whether `typeof value === 'object'` holds (`null` aside, which callers check first).
    pub fn is_object(&self) -> bool {
        matches!(self, J::Arr(_) | J::Obj(_) | J::Bytes { .. })
    }

    /// The JSON value a result is returned as. `undefined` properties are left out, as
    /// `JSON.stringify` does; anything JSON cannot carry becomes `null`.
    pub fn into_value(self) -> serde_json::Value {
        use serde_json::Value;
        match self {
            J::Undef | J::Null | J::Bytes { .. } => Value::Null,
            J::Bool(b) => Value::Bool(b),
            J::Num(n) => number_value(n),
            J::Str(s) => Value::String(s),
            J::Arr(items) => Value::Array(items.into_iter().map(J::into_value).collect()),
            J::Obj(fields) => Value::Object(
                fields
                    .into_iter()
                    .filter(|(_, v)| *v != J::Undef)
                    .map(|(k, v)| (k, v.into_value()))
                    .collect(),
            ),
        }
    }
}

/// A number as JSON. Integral numbers go out as integers, as `JSON.stringify` writes them, so a
/// consumer reading an integer (an id, a page) accepts the normalized value.
pub fn number_value(n: f64) -> serde_json::Value {
    use serde_json::Value;
    if n.is_finite() && n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0 {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

/// Integral floating-point numbers of `value` as integers (see [`number_value`]).
pub fn integral(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Number(n) if n.is_f64() => n.as_f64().map_or(Value::Number(n), number_value),
        Value::Array(items) => Value::Array(items.into_iter().map(integral).collect()),
        Value::Object(fields) => {
            Value::Object(fields.into_iter().map(|(k, v)| (k, integral(v))).collect())
        }
        other => other,
    }
}

/// The own enumerable entries of a value, as `Object.keys`/spread see them: an object's fields, an
/// array's items by index, a `Uint8Array`'s bytes by index (the first four are kept, which is all a
/// key check can see), a string's code units by index.
pub fn own_entries(v: J) -> Vec<(String, J)> {
    match v {
        J::Obj(fields) => fields,
        J::Arr(items) => items
            .into_iter()
            .enumerate()
            .map(|(i, item)| (i.to_string(), item))
            .collect(),
        J::Bytes { len, head } => (0..len.min(4))
            .map(|i| {
                (
                    i.to_string(),
                    J::Num(f64::from(head.get(i).copied().unwrap_or(0))),
                )
            })
            .collect(),
        J::Str(s) => s
            .encode_utf16()
            .enumerate()
            .map(|(i, unit)| {
                // Unpaired surrogates cannot be held as Rust text; U+FFFD keeps the length.
                let c = char::from_u32(u32::from(unit)).unwrap_or('\u{FFFD}');
                (i.to_string(), J::Str(c.to_string()))
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// One step of the path to a rejected value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Key(String),
    Index(usize),
}

/// Why a value did not decode, and where. The path is empty when the value itself was rejected:
/// a missing or unknown property is reported at the object that lacks or has it.
#[derive(Debug, PartialEq, Eq)]
pub struct Error {
    pub path: Vec<Step>,
    pub message: String,
}

impl Error {
    /// The property names on the path, with `[]` for any list position.
    pub fn keys(&self) -> Vec<&str> {
        self.path
            .iter()
            .map(|step| match step {
                Step::Key(key) => key.as_str(),
                Step::Index(_) => "[]",
            })
            .collect()
    }

    fn at(mut self, step: Step) -> Error {
        self.path.insert(0, step);
        self
    }
}

impl fmt::Display for Error {
    /// `speeds[0]: unknown variant …`, or only the message for the value itself.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut path = String::new();
        for step in &self.path {
            match step {
                Step::Key(key) if path.is_empty() => path.push_str(key),
                Step::Key(key) => {
                    path.push('.');
                    path.push_str(key);
                }
                Step::Index(i) => path.push_str(&format!("[{i}]")),
            }
        }
        if path.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{path}: {}", self.message)
        }
    }
}

impl std::error::Error for Error {}

impl de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Error {
            path: Vec::new(),
            message: msg.to_string(),
        }
    }
}

/// Decodes `value` as `T`.
pub fn decode<T: DeserializeOwned>(value: J) -> Result<T, Error> {
    T::deserialize(value)
}

/// The largest integer a number holds exactly (`Number.MAX_SAFE_INTEGER`).
const SAFE: f64 = 9_007_199_254_740_991.0;

impl J {
    fn unexpected(&self) -> Unexpected<'_> {
        match self {
            J::Undef => Unexpected::Other("undefined"),
            J::Null => Unexpected::Other("null"),
            J::Bool(b) => Unexpected::Bool(*b),
            J::Num(n) if n.is_nan() => Unexpected::Other("NaN"),
            J::Num(n) => Unexpected::Float(*n),
            J::Str(s) => Unexpected::Str(s),
            J::Bytes { .. } => Unexpected::Other("Uint8Array"),
            J::Arr(_) => Unexpected::Seq,
            J::Obj(_) => Unexpected::Map,
        }
    }
}

impl<'de> Deserializer<'de> for J {
    type Error = Error;

    /// `undefined`, `null`, NaN and binary data have no Rust counterpart a field accepts: an optional field
    /// takes `undefined` as absent ([`Self::deserialize_option`]), and nothing else takes any of
    /// them. Integral numbers are integers, so integer fields accept them.
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            J::Bool(b) => visitor.visit_bool(b),
            J::Num(n) if n.fract() == 0.0 && n.abs() <= SAFE => {
                if n >= 0.0 {
                    visitor.visit_u64(n as u64)
                } else {
                    visitor.visit_i64(n as i64)
                }
            }
            J::Num(n) if !n.is_nan() => visitor.visit_f64(n),
            J::Str(s) => visitor.visit_string(s),
            J::Arr(items) => {
                let mut seq = Items {
                    items: items.into_iter().enumerate(),
                };
                let value = visitor.visit_seq(&mut seq)?;
                match seq.items.len() {
                    0 => Ok(value),
                    left => Err(de::Error::invalid_length(left, &"fewer items")),
                }
            }
            J::Obj(fields) => visitor.visit_map(Fields {
                fields: fields.into_iter(),
                pending: None,
            }),
            other => Err(de::Error::invalid_type(other.unexpected(), &visitor)),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            J::Undef => visitor.visit_none(),
            other => visitor.visit_some(other),
        }
    }

    /// Unit variants are named by a string.
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        match self {
            J::Str(s) => visitor.visit_enum(s.into_deserializer()),
            other => Err(de::Error::invalid_type(other.unexpected(), &visitor)),
        }
    }

    /// Whatever `typeof value === 'object'` holds for decodes as a struct or a map, by its own
    /// entries (a list by its indices, not by position), as the TypeScript validators read it.
    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_map(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            J::Obj(_) | J::Arr(_) | J::Bytes { .. } => visitor.visit_map(Fields {
                fields: own_entries(self).into_iter(),
                pending: None,
            }),
            other => Err(de::Error::invalid_type(other.unexpected(), &visitor)),
        }
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_unit()
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_newtype_struct(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf unit unit_struct seq tuple tuple_struct identifier
    }
}

impl IntoDeserializer<'_, Error> for J {
    type Deserializer = J;

    fn into_deserializer(self) -> J {
        self
    }
}

/// A list's items, each error marked with its position.
struct Items {
    items: std::iter::Enumerate<std::vec::IntoIter<J>>,
}

impl<'de> SeqAccess<'de> for &mut Items {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Error> {
        match self.items.next() {
            Some((i, item)) => seed
                .deserialize(item)
                .map(Some)
                .map_err(|e| e.at(Step::Index(i))),
            None => Ok(None),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.items.len())
    }
}

/// An object's properties, each error marked with its key.
struct Fields {
    fields: std::vec::IntoIter<(String, J)>,
    pending: Option<(String, J)>,
}

impl<'de> MapAccess<'de> for Fields {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Error> {
        match self.fields.next() {
            Some((key, value)) => {
                let name = seed.deserialize(J::Str(key.clone()))?;
                self.pending = Some((key, value));
                Ok(Some(name))
            }
            None => Ok(None),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
        let (key, value) = self
            .pending
            .take()
            .ok_or_else(|| <Error as de::Error>::custom("a value was read before its key"))?;
        seed.deserialize(value).map_err(|e| e.at(Step::Key(key)))
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.fields.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize, Debug, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct Inner {
        n: u32,
        s: Option<String>,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct Outer {
        list: Vec<Inner>,
    }

    fn obj(fields: Vec<(&str, J)>) -> J {
        J::Obj(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    #[test]
    fn errors_name_the_path_of_the_rejected_value() {
        let bad = obj(vec![(
            "list",
            J::Arr(vec![
                obj(vec![("n", J::Num(1.0))]),
                obj(vec![("n", J::Num(1.5))]),
            ]),
        )]);
        let error = decode::<Outer>(bad).unwrap_err();
        assert_eq!(error.keys(), ["list", "[]", "n"]);
        assert_eq!(
            error.to_string(),
            "list[1].n: invalid type: floating point `1.5`, expected u32"
        );
        let missing = obj(vec![("list", J::Arr(vec![obj(vec![])]))]);
        let error = decode::<Outer>(missing).unwrap_err();
        assert_eq!(error.keys(), ["list", "[]"]);
        assert_eq!(error.to_string(), "list[0]: missing field `n`");
    }

    #[test]
    fn undefined_is_absent_and_null_is_not() {
        let undefined = obj(vec![("n", J::Num(2.0)), ("s", J::Undef)]);
        assert_eq!(decode::<Inner>(undefined).unwrap(), Inner { n: 2, s: None });
        let null = obj(vec![("n", J::Num(2.0)), ("s", J::Null)]);
        assert_eq!(
            decode::<Inner>(null).unwrap_err().to_string(),
            "s: invalid type: null, expected a string"
        );
        assert!(decode::<f64>(J::Num(f64::NAN)).is_err());
        assert_eq!(decode::<f64>(J::Num(f64::INFINITY)).unwrap(), f64::INFINITY);
        let bytes = J::Bytes {
            len: 1,
            head: vec![b'a'],
        };
        assert!(decode::<String>(bytes).is_err());
        // A list is an object of its indices, never a struct by position.
        let positional = J::Arr(vec![J::Num(2.0), J::Undef]);
        assert_eq!(
            decode::<Inner>(positional).unwrap_err().to_string(),
            "unknown field `0`, expected `n` or `s`"
        );
    }

    #[test]
    fn integral_floats_become_integers() {
        assert_eq!(
            integral(serde_json::json!({ "a": [1.0, 0.5] })),
            serde_json::json!({ "a": [1, 0.5] })
        );
    }
}
