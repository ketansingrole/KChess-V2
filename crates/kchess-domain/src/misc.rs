//! Validation and library documents (`crates/kchess-wasm/js/validate.ts` and `library.ts`), reached
//! through `api::call`. The TypeScript wrappers send their arguments through `rulesLossless`:
//! the first argument is a sidecar listing where `undefined`, NaN, the infinities and binary data
//! sat, since JSON cannot carry them. Those are rebuilt here before validation, so each check sees
//! exactly the values the TypeScript validator saw.

mod constants;
mod contracts;
mod documents;
mod engine;
mod patterns;
mod validate;

pub use contracts::{Scope, TV_CHANNEL_KEYS, arities, normalize, validate_arguments};

use engine::J;
use serde_json::Value;

/// This module's methods for `api::call`; None when the method is not one of them.
pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let is_check = contracts::is_call(method);
    let run = if is_check {
        None
    } else {
        Some(validate::validator(method).or_else(|| documents::handler(method))?)
    };
    let Some((sidecar, user)) = args.split_first() else {
        return Some(Err(format!(
            "{method} was called without its lossless arguments"
        )));
    };
    if is_check {
        return Some(
            inputs(user, sidecar)
                .and_then(|values| contracts::argument_call(method, &values))
                .map(J::into_value),
        );
    }
    let run = run?;
    Some(inputs(user, sidecar).and_then(run).map(J::into_value))
}

/// The arguments as values, with the sidecar's `undefined`, NaN, infinite and binary entries.
fn inputs(user: &[Value], sidecar: &Value) -> Result<Vec<J>, String> {
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

fn from_json(v: &Value) -> J {
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
