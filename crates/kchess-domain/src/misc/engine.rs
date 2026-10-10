//! The schema engine the TypeScript validators used (valibot 1.5), ported over [`J`]: the same
//! issue order, default messages, expectations and output shapes, so a rejected input reports the
//! same text. Only the combinators `crates/kchess-wasm/js/validate.ts` and `library.ts` use exist here.

use crate::js;

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
            // Integral numbers go out as integers, as `JSON.stringify` writes them, so a consumer
            // reading an integer (an id, a page) accepts the normalized value.
            J::Num(n)
                if n.is_finite() && n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0 =>
            {
                Value::from(n as i64)
            }
            J::Num(n) => serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number),
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

/// `String(value)`: the text a value takes in string coercion.
pub fn js_string(v: &J) -> String {
    match v {
        J::Undef => "undefined".into(),
        J::Null => "null".into(),
        J::Bool(b) => b.to_string(),
        J::Num(n) => number_text(*n),
        J::Str(s) => s.clone(),
        J::Arr(items) => items
            .iter()
            .map(|item| match item {
                J::Undef | J::Null => String::new(),
                other => js_string(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        J::Bytes { .. } | J::Obj(_) => "[object Object]".into(),
    }
}

/// `Number.prototype.toString()` for any number, NaN and the infinities included.
pub fn number_text(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n == f64::INFINITY {
        "Infinity".into()
    } else if n == f64::NEG_INFINITY {
        "-Infinity".into()
    } else {
        js::number_to_string(n)
    }
}

/// valibot's `_stringify`: how an issue shows the value it rejected.
pub fn stringify(v: &J) -> String {
    match v {
        J::Undef => "undefined".into(),
        J::Null => "null".into(),
        J::Bool(b) => b.to_string(),
        J::Num(n) => number_text(*n),
        J::Str(s) => format!("\"{s}\""),
        J::Arr(_) => "Array".into(),
        J::Obj(_) => "Object".into(),
        J::Bytes { .. } => "Uint8Array".into(),
    }
}

/// A custom message from the schema, as the TypeScript source wrote it.
pub type Msg = Option<&'static str>;

/// `_addIssue`'s message: the custom one, else `Invalid <label>: Expected <x> but received <y>`.
pub fn message(msg: Msg, label: &str, expected: Option<&str>, received: &str) -> String {
    if let Some(m) = msg {
        return m.to_string();
    }
    match expected {
        Some(e) if !e.is_empty() => {
            format!("Invalid {label}: Expected {e} but received {received}")
        }
        _ => format!("Invalid {label}: Received {received}"),
    }
}

/// valibot's `uuid` requirement: `/^[\da-f]{8}(?:-[\da-f]{4}){3}-[\da-f]{12}$/iu`.
fn is_uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    const LENGTHS: [usize; 5] = [8, 4, 4, 4, 12];
    groups.len() == LENGTHS.len()
        && groups.iter().zip(LENGTHS).all(|(group, length)| {
            group.len() == length && group.bytes().all(|b| b.is_ascii_hexdigit())
        })
}

/// valibot's `_joinExpects`: distinct expectations, parenthesised when there are several.
fn join_expects(list: Vec<String>, separator: &str) -> String {
    let mut distinct: Vec<String> = Vec::with_capacity(list.len());
    for item in list {
        if !distinct.contains(&item) {
            distinct.push(item);
        }
    }
    match distinct.len() {
        0 => "never".into(),
        1 => distinct.remove(0),
        _ => format!("({})", distinct.join(&format!(" {separator} "))),
    }
}

/// The outcome of running a schema: whether the input still has its type, the output value and
/// the issues (messages, in valibot's order). Valid input has no issues.
pub struct Ds {
    pub typed: bool,
    pub value: J,
    pub issues: Vec<String>,
}

impl Ds {
    fn ok(value: J) -> Ds {
        Ds {
            typed: true,
            value,
            issues: Vec::new(),
        }
    }

    /// A schema-level failure: the value loses its type.
    fn fail(value: J, issue: String) -> Ds {
        Ds {
            typed: false,
            value,
            issues: vec![issue],
        }
    }
}

fn type_fail(msg: Msg, expects: &str, input: J) -> Ds {
    let issue = message(msg, "type", Some(expects), &stringify(&input));
    Ds::fail(input, issue)
}

/// Actions of a pipe: validations (which only report issues) and transformations.
pub enum Action {
    Trim,
    MinLength(usize, Msg),
    MaxLength(usize, Msg),
    MinValue(f64, Msg),
    MaxValue(f64, Msg),
    Integer(Msg),
    Finite(Msg),
    /// valibot's `uuid()`: the 8-4-4-4-12 hexadecimal form, in either case.
    Uuid(Msg),
    Regex {
        source: &'static str,
        test: fn(&str) -> bool,
        msg: Msg,
    },
    Check {
        check: fn(&J) -> bool,
        msg: Msg,
    },
    Transform(fn(J) -> J),
}

/// A schema of the subset of valibot the validators use.
pub enum Schema {
    Str(Msg),
    Num(Msg),
    Bool(Msg),
    Unknown,
    /// `v.instance(Uint8Array)`.
    Bytes(Msg),
    Literal {
        value: J,
        msg: Msg,
    },
    Picklist {
        options: Vec<J>,
        msg: Msg,
    },
    Object {
        entries: Vec<(&'static str, Schema)>,
        msg: Msg,
        strict: bool,
    },
    Array {
        item: Box<Schema>,
        msg: Msg,
    },
    Record {
        key: Box<Schema>,
        value: Box<Schema>,
        msg: Msg,
    },
    Union {
        options: Vec<Schema>,
        msg: Msg,
    },
    Optional {
        wrapped: Box<Schema>,
        default: Option<J>,
    },
    Pipe {
        base: Box<Schema>,
        actions: Vec<Action>,
    },
    /// `v.variant(key, options)` over object schemas whose `key` is a literal.
    Variant {
        key: &'static str,
        options: Vec<Schema>,
        msg: Msg,
    },
}

pub fn pipe(base: Schema, actions: Vec<Action>) -> Schema {
    Schema::Pipe {
        base: Box::new(base),
        actions,
    }
}

pub fn optional(wrapped: Schema) -> Schema {
    Schema::Optional {
        wrapped: Box::new(wrapped),
        default: None,
    }
}

pub fn object(entries: Vec<(&'static str, Schema)>, msg: Msg) -> Schema {
    Schema::Object {
        entries,
        msg,
        strict: false,
    }
}

pub fn strict_object(entries: Vec<(&'static str, Schema)>, msg: Msg) -> Schema {
    Schema::Object {
        entries,
        msg,
        strict: true,
    }
}

pub fn array(item: Schema, msg: Msg) -> Schema {
    Schema::Array {
        item: Box::new(item),
        msg,
    }
}

pub fn union(options: Vec<Schema>, msg: Msg) -> Schema {
    Schema::Union { options, msg }
}

pub fn picklist_str(options: &[&str], msg: Msg) -> Schema {
    Schema::Picklist {
        options: options.iter().map(|s| J::Str((*s).into())).collect(),
        msg,
    }
}

pub fn picklist_num(options: &[f64], msg: Msg) -> Schema {
    Schema::Picklist {
        options: options.iter().map(|n| J::Num(*n)).collect(),
        msg,
    }
}

pub fn literal(value: &str) -> Schema {
    Schema::Literal {
        value: J::Str(value.into()),
        msg: None,
    }
}

impl Schema {
    /// valibot's `expects`: what the schema accepts, as an issue describes it.
    pub fn expects(&self) -> String {
        match self {
            Schema::Str(_) => "string".into(),
            Schema::Num(_) => "number".into(),
            Schema::Bool(_) => "boolean".into(),
            Schema::Unknown => "unknown".into(),
            Schema::Bytes(_) => "Uint8Array".into(),
            Schema::Literal { value, .. } => stringify(value),
            Schema::Picklist { options, .. } => {
                join_expects(options.iter().map(stringify).collect(), "|")
            }
            Schema::Object { .. } | Schema::Record { .. } | Schema::Variant { .. } => {
                "Object".into()
            }
            Schema::Array { .. } => "Array".into(),
            Schema::Union { options, .. } => {
                join_expects(options.iter().map(Schema::expects).collect(), "|")
            }
            Schema::Optional { wrapped, .. } => format!("({} | undefined)", wrapped.expects()),
            Schema::Pipe { base, .. } => base.expects(),
        }
    }

    /// The value an absent property takes, for schemas that run on it (`optional` with a default).
    fn default_for_missing(&self) -> Option<J> {
        match self {
            Schema::Optional {
                default: Some(d), ..
            } => Some(d.clone()),
            _ => None,
        }
    }

    fn is_optional(&self) -> bool {
        matches!(self, Schema::Optional { .. })
    }

    /// valibot's `~run`: validate `input` (with abortEarly off, the library default).
    pub fn run(&self, input: J) -> Ds {
        match self {
            Schema::Str(msg) => match input {
                J::Str(_) => Ds::ok(input),
                other => type_fail(*msg, "string", other),
            },
            Schema::Num(msg) => match input {
                J::Num(n) if !n.is_nan() => Ds::ok(input),
                other => type_fail(*msg, "number", other),
            },
            Schema::Bool(msg) => match input {
                J::Bool(_) => Ds::ok(input),
                other => type_fail(*msg, "boolean", other),
            },
            Schema::Unknown => Ds::ok(input),
            Schema::Bytes(msg) => match input {
                J::Bytes { .. } => Ds::ok(input),
                other => type_fail(*msg, "Uint8Array", other),
            },
            Schema::Literal { value, msg } => {
                if input == *value {
                    Ds::ok(input)
                } else {
                    let expects = stringify(value);
                    type_fail(*msg, &expects, input)
                }
            }
            Schema::Picklist { options, msg } => {
                if options.contains(&input) {
                    Ds::ok(input)
                } else {
                    let expects = self.expects();
                    type_fail(*msg, &expects, input)
                }
            }
            Schema::Object {
                entries,
                msg,
                strict,
            } => run_object(entries, *msg, *strict, input),
            Schema::Array { item, msg } => match input {
                J::Arr(items) => {
                    let mut ds = Ds::ok(J::Undef);
                    let mut out = Vec::with_capacity(items.len());
                    for value in items {
                        let item_ds = item.run(value);
                        ds.issues.extend(item_ds.issues);
                        if !item_ds.typed {
                            ds.typed = false;
                        }
                        out.push(item_ds.value);
                    }
                    ds.value = J::Arr(out);
                    ds
                }
                other => type_fail(*msg, "Array", other),
            },
            Schema::Record { key, value, msg } => match input {
                J::Obj(fields) => {
                    let mut ds = Ds::ok(J::Undef);
                    let mut out = Vec::with_capacity(fields.len());
                    for (entry_key, entry_value) in fields {
                        let key_ds = key.run(J::Str(entry_key));
                        ds.issues.extend(key_ds.issues);
                        let value_ds = value.run(entry_value);
                        ds.issues.extend(value_ds.issues);
                        if !key_ds.typed || !value_ds.typed {
                            ds.typed = false;
                        }
                        if let (true, J::Str(name)) = (key_ds.typed, key_ds.value) {
                            out.push((name, value_ds.value));
                        }
                    }
                    ds.value = J::Obj(out);
                    ds
                }
                other => type_fail(*msg, "Object", other),
            },
            Schema::Union { options, msg } => run_union(options, *msg, input),
            Schema::Optional { wrapped, default } => {
                if input == J::Undef {
                    match default {
                        Some(d) => wrapped.run(d.clone()),
                        None => Ds::ok(J::Undef),
                    }
                } else {
                    wrapped.run(input)
                }
            }
            Schema::Pipe { base, actions } => {
                let mut ds = base.run(input);
                for action in actions {
                    let transformation = matches!(action, Action::Trim | Action::Transform(_));
                    if transformation && !ds.issues.is_empty() {
                        ds.typed = false;
                        break;
                    }
                    action.apply(&mut ds);
                }
                ds
            }
            Schema::Variant { key, options, msg } => run_variant(key, options, *msg, input),
        }
    }
}

impl Action {
    /// Run one action of a pipe. Validations add an issue only while the value is typed.
    fn apply(&self, ds: &mut Ds) {
        match self {
            Action::Trim => {
                if let J::Str(s) = &mut ds.value {
                    let start_len = s.len();
                    let trimmed = js::trim(s);
                    if trimmed.len() != start_len {
                        let owned = trimmed.to_owned();
                        *s = owned;
                    }
                }
            }
            Action::MinLength(min, msg) => {
                if let Some(count) = length_of(&ds.value).filter(|c| ds.typed && c < min) {
                    ds.issues.push(message(
                        *msg,
                        "length",
                        Some(&format!(">={min}")),
                        &count.to_string(),
                    ));
                }
            }
            Action::MaxLength(max, msg) => {
                if let Some(count) = length_of(&ds.value).filter(|c| ds.typed && c > max) {
                    ds.issues.push(message(
                        *msg,
                        "length",
                        Some(&format!("<={max}")),
                        &count.to_string(),
                    ));
                }
            }
            Action::MinValue(min, msg) => {
                if let J::Num(n) = ds.value
                    && ds.typed
                    && n < *min
                {
                    let expects = format!(">={}", number_text(*min));
                    ds.issues
                        .push(message(*msg, "value", Some(&expects), &number_text(n)));
                }
            }
            Action::MaxValue(max, msg) => {
                if let J::Num(n) = ds.value
                    && ds.typed
                    && n > *max
                {
                    let expects = format!("<={}", number_text(*max));
                    ds.issues
                        .push(message(*msg, "value", Some(&expects), &number_text(n)));
                }
            }
            Action::Integer(msg) => {
                if let J::Num(n) = ds.value
                    && ds.typed
                    && !(n.is_finite() && n.fract() == 0.0)
                {
                    ds.issues
                        .push(message(*msg, "integer", None, &number_text(n)));
                }
            }
            Action::Finite(msg) => {
                if let J::Num(n) = ds.value
                    && ds.typed
                    && !n.is_finite()
                {
                    ds.issues
                        .push(message(*msg, "finite", None, &number_text(n)));
                }
            }
            Action::Uuid(msg) => {
                if let J::Str(s) = &ds.value
                    && ds.typed
                    && !is_uuid(s)
                {
                    ds.issues
                        .push(message(*msg, "UUID", None, &stringify(&ds.value)));
                }
            }
            Action::Regex { source, test, msg } => {
                if let J::Str(s) = &ds.value
                    && ds.typed
                    && !test(s)
                {
                    ds.issues
                        .push(message(*msg, "format", Some(source), &stringify(&ds.value)));
                }
            }
            Action::Check { check, msg } => {
                if ds.typed && !check(&ds.value) {
                    ds.issues
                        .push(message(*msg, "input", None, &stringify(&ds.value)));
                }
            }
            Action::Transform(f) => {
                let value = std::mem::replace(&mut ds.value, J::Undef);
                ds.value = f(value);
            }
        }
    }
}

/// `value.length` for strings (UTF-16 units) and arrays.
fn length_of(v: &J) -> Option<usize> {
    match v {
        J::Str(s) => Some(js::utf16_len(s)),
        J::Arr(items) => Some(items.len()),
        _ => None,
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

fn run_object(entries: &[(&'static str, Schema)], msg: Msg, strict: bool, input: J) -> Ds {
    if !input.is_object() {
        return type_fail(msg, "Object", input);
    }
    let fields = own_entries(input);
    let mut slots: Vec<(String, Option<J>)> =
        fields.into_iter().map(|(k, v)| (k, Some(v))).collect();
    let mut ds = Ds::ok(J::Undef);
    let mut out = Vec::with_capacity(entries.len());
    for (key, schema) in entries {
        let present = slots
            .iter_mut()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| v.take());
        let value = present.or_else(|| schema.default_for_missing());
        match value {
            Some(v) => {
                let value_ds = schema.run(v);
                ds.issues.extend(value_ds.issues);
                if !value_ds.typed {
                    ds.typed = false;
                }
                out.push(((*key).to_string(), value_ds.value));
            }
            None if schema.is_optional() => {}
            None => {
                let expects = format!("\"{key}\"");
                ds.issues
                    .push(message(msg, "key", Some(&expects), "undefined"));
                ds.typed = false;
            }
        }
    }
    if strict
        && let Some((extra, _)) = slots
            .iter()
            .find(|(k, _)| !entries.iter().any(|(e, _)| e == k))
    {
        ds.issues
            .push(message(msg, "key", Some("never"), &format!("\"{extra}\"")));
        ds.typed = false;
    }
    ds.value = J::Obj(out);
    ds
}

fn run_union(options: &[Schema], msg: Msg, input: J) -> Ds {
    let mut typed_runs: Vec<Ds> = Vec::new();
    let mut untyped_runs: Vec<Ds> = Vec::new();
    for option in options {
        let ds = option.run(input.clone());
        if ds.typed {
            if ds.issues.is_empty() {
                return ds;
            }
            typed_runs.push(ds);
        } else {
            untyped_runs.push(ds);
        }
    }
    if typed_runs.len() == 1 {
        return typed_runs.remove(0);
    }
    // Several options got past the type check: the union's own issue, typed, with their issues
    // attached (valibot's `subIssues`, which `parse` does not report).
    if !typed_runs.is_empty() {
        let expects = join_expects(options.iter().map(Schema::expects).collect(), "|");
        let issue = message(msg, "type", Some(&expects), &stringify(&input));
        let mut ds = Ds::fail(input, issue);
        ds.typed = true;
        return ds;
    }
    if untyped_runs.len() == 1 {
        return untyped_runs.remove(0);
    }
    let expects = join_expects(options.iter().map(Schema::expects).collect(), "|");
    let issue = message(msg, "type", Some(&expects), &stringify(&input));
    Ds::fail(input, issue)
}

fn run_variant(key: &str, options: &[Schema], msg: Msg, input: J) -> Ds {
    if !input.is_object() {
        return type_fail(msg, "Object", input);
    }
    let mut expected: Vec<String> = Vec::new();
    let mut matched: Option<Ds> = None;
    for option in options {
        let Schema::Object { entries, .. } = option else {
            continue;
        };
        let Some((_, discriminator)) = entries.iter().find(|(k, _)| *k == key) else {
            continue;
        };
        let value = input.get(key).cloned().unwrap_or(J::Undef);
        if discriminator.run(value).issues.is_empty() {
            let ds = option.run(input.clone());
            let replace = match &matched {
                None => true,
                Some(current) => !current.typed && ds.typed,
            };
            if replace {
                matched = Some(ds);
            }
            if matched.as_ref().is_some_and(|d| d.issues.is_empty()) {
                break;
            }
        } else {
            expected.push(discriminator.expects());
        }
    }
    if let Some(ds) = matched {
        return ds;
    }
    let received = input.get(key).cloned().unwrap_or(J::Undef);
    let expects = join_expects(expected, "|");
    let issue = message(msg, "type", Some(&expects), &stringify(&received));
    Ds::fail(input, issue)
}

/// Parse `input`: its output value, or the first issue's message (`parse` in `validate.ts`).
pub fn parse(schema: &Schema, input: J) -> Result<J, String> {
    let ds = schema.run(input);
    match ds.issues.into_iter().next() {
        Some(issue) => Err(issue),
        None => Ok(ds.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_messages_match_valibot() {
        let schema = pipe(
            Schema::Str(None),
            vec![
                Action::MaxLength(3, None),
                Action::Regex {
                    source: "/^a$/",
                    test: |s| s == "a",
                    msg: None,
                },
            ],
        );
        assert_eq!(
            parse(&schema, J::Str("abcd".into())),
            Err("Invalid length: Expected <=3 but received 4".into())
        );
        assert_eq!(
            parse(&schema, J::Str("b".into())),
            Err("Invalid format: Expected /^a$/ but received \"b\"".into())
        );
        assert_eq!(
            parse(&schema, J::Num(1.0)),
            Err("Invalid type: Expected string but received 1".into())
        );
    }

    #[test]
    fn missing_keys_report_the_object_message() {
        let schema = object(vec![("a", Schema::Str(None))], Some("Bad."));
        assert_eq!(parse(&schema, J::Obj(vec![])), Err("Bad.".into()));
        let plain = object(vec![("a", Schema::Str(None))], None);
        assert_eq!(
            parse(&plain, J::Obj(vec![])),
            Err("Invalid key: Expected \"a\" but received undefined".into())
        );
    }

    #[test]
    fn nan_and_undefined_are_not_null() {
        let schema = Schema::Num(None);
        assert_eq!(
            parse(&schema, J::Num(f64::NAN)),
            Err("Invalid type: Expected number but received NaN".into())
        );
        assert_eq!(
            parse(&schema, J::Null),
            Err("Invalid type: Expected number but received null".into())
        );
    }
}
