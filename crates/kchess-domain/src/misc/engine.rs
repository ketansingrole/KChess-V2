//! The schema engine the TypeScript validators used (valibot 1.5), ported over [`J`]: the same
//! issue order, default messages, expectations and output shapes, so a rejected input reports the
//! same text. Only the combinators `crates/kchess-wasm/js/validate.ts` and `library.ts` use exist here.

use crate::js;

pub use super::value::{J, own_entries};

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
    MinLength(usize, Msg),
    MaxLength(usize, Msg),
    Regex {
        source: &'static str,
        test: fn(&str) -> bool,
        msg: Msg,
    },
}

/// A schema of the subset of valibot the validators use.
pub enum Schema {
    Str(Msg),
    Unknown,
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

pub fn picklist_str(options: &[&str], msg: Msg) -> Schema {
    Schema::Picklist {
        options: options.iter().map(|s| J::Str((*s).into())).collect(),
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
            Schema::Unknown => "unknown".into(),
            Schema::Literal { value, .. } => stringify(value),
            Schema::Picklist { options, .. } => {
                join_expects(options.iter().map(stringify).collect(), "|")
            }
            Schema::Object { .. } | Schema::Variant { .. } => "Object".into(),
            Schema::Array { .. } => "Array".into(),
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
            Schema::Unknown => Ds::ok(input),
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
            Action::Regex { source, test, msg } => {
                if let J::Str(s) = &ds.value
                    && ds.typed
                    && !test(s)
                {
                    ds.issues
                        .push(message(*msg, "format", Some(source), &stringify(&ds.value)));
                }
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
}
