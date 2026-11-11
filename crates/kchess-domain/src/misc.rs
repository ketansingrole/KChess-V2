//! Validation and library documents. Validators are found by the name the core's contracts give
//! them (`assertX`), and see JavaScript-like values (`J`): JSON arguments, or JSON with a sidecar
//! listing where `undefined`, NaN, the infinities and binary data sat (`inputs`), since JSON
//! cannot carry them.

pub mod constants;
pub mod contracts;
pub mod documents;
pub mod patterns;
pub mod unicode_tables;
pub mod validate;
pub mod value;

pub use constants::MAX_DOCUMENT;
pub use contracts::{ASSISTANCE_METHODS, TV_CHANNEL_KEYS, arities, normalize, validate_arguments};

use serde_json::Value;
use value::J;

/// The normalized value validator `name` gives for `args`, or the message it rejected them with.
pub fn validate(name: &str, args: Vec<J>) -> Result<Value, String> {
    match Run::find(name) {
        Some(run) => run.call(args),
        None => Err(format!("Unknown validator {name}.")),
    }
}

/// Whether `name` is a validator or a document check.
pub fn is_validator(name: &str) -> bool {
    Run::find(name).is_some()
}

/// `validate` of JSON arguments, a JSON `null` being `null`.
pub fn validate_values(name: &str, args: &[Value]) -> Result<Value, String> {
    validate(name, args.iter().map(J::from).collect())
}

/// A value validator or a document check.
#[derive(Clone, Copy)]
pub(crate) enum Run {
    Validate(validate::Validator),
    Check(documents::Check),
}

impl Run {
    /// The validator or document check by the name the contracts give it.
    pub(crate) fn find(name: &str) -> Option<Run> {
        validate::validator(name)
            .map(Run::Validate)
            .or_else(|| documents::check(name).map(Run::Check))
    }

    /// Its normalized value for `args`, or the message it rejected them with.
    pub(crate) fn call(self, args: Vec<J>) -> Result<Value, String> {
        match self {
            Run::Validate(validate) => validate(args),
            Run::Check(check) => check(args).map(J::into_value),
        }
    }
}

/// The arguments as values, with the sidecar's `undefined`, NaN, infinite and binary entries.
pub fn inputs(user: &[Value], sidecar: &Value) -> Result<Vec<J>, String> {
    let Value::Array(entries) = sidecar else {
        return Err("the lossless sidecar is not a list".into());
    };
    let mut values: Vec<J> = user.iter().map(from_json).collect();
    // Replacements first: they address present values only. Insertions follow in key order, so
    // each `undefined` property lands where its key sat in the original object.
    let mut inserts = Vec::new();
    for entry in entries {
        let at = entry
            .get("at")
            .and_then(Value::as_array)
            .ok_or("a sidecar entry has no path")?;
        let (arg, path) = segments(at)?;
        let kind = entry
            .get("k")
            .and_then(Value::as_u64)
            .ok_or("a sidecar entry has no kind")?;
        match (kind, path.last()) {
            (0, Some(Seg::Key(key))) => {
                let position = entry.get("i").and_then(Value::as_u64).unwrap_or(0) as usize;
                inserts.push((position, arg, path[..path.len() - 1].to_vec(), key.clone()));
            }
            (0, _) => replace(&mut values, arg, &path, J::Undef)?,
            (1, _) => replace(&mut values, arg, &path, J::Num(f64::NAN))?,
            (2, _) => replace(&mut values, arg, &path, J::Num(f64::INFINITY))?,
            (3, _) => replace(&mut values, arg, &path, J::Num(f64::NEG_INFINITY))?,
            (4, _) => {
                let len = entry.get("n").and_then(Value::as_u64).unwrap_or(0) as usize;
                let head = entry
                    .get("h")
                    .and_then(Value::as_array)
                    .map(|bytes| {
                        bytes
                            .iter()
                            .filter_map(|b| b.as_u64())
                            .map(|b| b as u8)
                            .collect()
                    })
                    .unwrap_or_default();
                replace(&mut values, arg, &path, J::Bytes { len, head })?;
            }
            _ => return Err("a sidecar entry has an unknown kind".into()),
        }
    }
    inserts.sort_by_key(|(position, ..)| *position);
    for (position, arg, parent, key) in inserts {
        let node = navigate(
            values.get_mut(arg).ok_or("a sidecar argument is missing")?,
            &parent,
        )
        .ok_or("a sidecar path is malformed")?;
        if let J::Obj(fields) = node
            && !fields.iter().any(|(k, _)| *k == key)
        {
            let at = position.min(fields.len());
            fields.insert(at, (key, J::Undef));
        }
    }
    Ok(values)
}

/// A step from a value to one of its properties or items.
#[derive(Clone)]
enum Seg {
    Key(String),
    Index(usize),
}

/// The argument position and the path inside it of a sidecar entry.
fn segments(at: &[Value]) -> Result<(usize, Vec<Seg>), String> {
    let (first, rest) = at.split_first().ok_or("a sidecar path is empty")?;
    let arg = first
        .as_u64()
        .ok_or("a sidecar path does not start with an argument")? as usize;
    let path = rest
        .iter()
        .map(|seg| match seg {
            Value::String(key) => Ok(Seg::Key(key.clone())),
            Value::Number(n) => n
                .as_u64()
                .map(|i| Seg::Index(i as usize))
                .ok_or_else(|| "a sidecar index is malformed".to_string()),
            _ => Err("a sidecar path segment is malformed".to_string()),
        })
        .collect::<Result<_, _>>()?;
    Ok((arg, path))
}

impl From<&Value> for J {
    fn from(value: &Value) -> J {
        from_json(value)
    }
}

pub(crate) fn from_json(v: &Value) -> J {
    match v {
        Value::Null => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Number(n) => J::Num(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => J::Str(s.clone()),
        Value::Array(items) => J::Arr(items.iter().map(from_json).collect()),
        Value::Object(fields) => J::Obj(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), from_json(v)))
                .collect(),
        ),
    }
}

/// The value at `path` below `value`, if the path exists.
fn navigate<'a>(value: &'a mut J, path: &[Seg]) -> Option<&'a mut J> {
    let mut current = value;
    for seg in path {
        current = match (current, seg) {
            (J::Obj(fields), Seg::Key(key)) => {
                fields.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)?
            }
            (J::Arr(items), Seg::Index(i)) => items.get_mut(*i)?,
            _ => return None,
        };
    }
    Some(current)
}

fn replace(values: &mut [J], arg: usize, path: &[Seg], new: J) -> Result<(), String> {
    let root = values.get_mut(arg).ok_or("a sidecar argument is missing")?;
    let node = navigate(root, path).ok_or("a sidecar path is malformed")?;
    *node = new;
    Ok(())
}
