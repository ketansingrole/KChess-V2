//! An argument's check is its type: a core method declared with a typed argument (no `= check`
//! in `core_api!`) receives it decoded and checked by the type's [`Validate`], whose refusal is
//! the message the caller sees.
//!
//! Arguments arrive as the values a frontend sent (`J`: JSON, where a `null` at the top is an
//! argument not given). A value of the wrong type takes the message its type names for that place
//! (`decode`'s `messages`, by the keys of the rejected value's path, `[]` for a list position);
//! without one, the decoder's own text prefixed by the path (`speeds[0]: unknown variant …`).
//! The rules corpus (`kchess-domain/tests/rules_corpus.rs`) pins these messages: a type that
//! replaces a `kchess_domain::misc::checks` validator answers its recorded cases there.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use kchess_domain::misc::patterns;
use kchess_domain::misc::value::{self, J};
use kchess_domain::variant::Variant;
use kchess_domain::{js, replay};

mod accounts;
mod lookups;

pub use accounts::*;
pub use lookups::*;

/// Why an argument was refused, as the caller is told.
pub type Refusal = String;

/// A core method's argument type: decoded from the value as sent (`J::Undef` when absent), then
/// checked and normalized.
pub trait Validate: Sized {
    fn validate(value: J) -> Result<Self, Refusal>;
}

/// `value` decoded and checked as `T`.
pub fn check<T: Validate>(value: J) -> Result<T, Refusal> {
    T::validate(value)
}

/// An optional argument: absent (or `null`) is `None`, anything else is checked.
impl<T: Validate> Validate for Option<T> {
    fn validate(value: J) -> Result<Self, Refusal> {
        match value {
            J::Undef | J::Null => Ok(None),
            value => T::validate(value).map(Some),
        }
    }
}

/// Types read as serde reads them, with no message of their own and nothing more to check.
macro_rules! decoded {
    ($($ty:ty),* $(,)?) => {$(
        impl Validate for $ty {
            fn validate(value: J) -> Result<Self, Refusal> {
                decode(value, plain)
            }
        }
    )*};
}

decoded!(bool, String, u32, u64, Value);

/* ── Decoding and the shared checks ── */

/// A custom message for a rejected value.
pub type Msg = Option<&'static str>;

/// Decodes `value` as `T`. A rejection takes the message `messages` names for the keys of the
/// rejected value's path, else the decoder's own text.
pub fn decode<T: DeserializeOwned>(value: J, messages: fn(&[&str]) -> Msg) -> Result<T, Refusal> {
    value::decode(value).map_err(|error| match messages(&error.keys()) {
        Some(message) => message.to_owned(),
        None => error.to_string(),
    })
}

/// No custom messages.
pub fn plain(_: &[&str]) -> Msg {
    None
}

/// `msg`, or a description of what the value at `path` should have been.
fn reject(msg: Msg, path: &str, expected: &str) -> Refusal {
    match msg {
        Some(message) => message.to_owned(),
        None if path.is_empty() => format!("invalid value, expected {expected}"),
        None => format!("{path}: invalid value, expected {expected}"),
    }
}

pub fn ensure(
    ok: bool,
    msg: Msg,
    path: &str,
    expected: impl FnOnce() -> String,
) -> Result<(), Refusal> {
    if ok {
        Ok(())
    } else {
        Err(reject(msg, path, &expected()))
    }
}

/// At most `max` UTF-16 units.
pub fn max_chars(text: &str, max: usize, msg: Msg, path: &str) -> Result<(), Refusal> {
    ensure(js::utf16_len(text) <= max, msg, path, || {
        format!("at most {max} characters")
    })
}

/// At least `min` UTF-16 units.
pub fn min_chars(text: &str, min: usize, msg: Msg, path: &str) -> Result<(), Refusal> {
    ensure(js::utf16_len(text) >= min, msg, path, || match min {
        1 => "at least 1 character".into(),
        _ => format!("at least {min} characters"),
    })
}

pub fn max_items<T>(items: &[T], max: usize, msg: Msg, path: &str) -> Result<(), Refusal> {
    ensure(items.len() <= max, msg, path, || {
        format!("at most {max} items")
    })
}

pub fn within<N: PartialOrd + std::fmt::Display>(
    n: N,
    min: N,
    max: N,
    msg: Msg,
    path: &str,
) -> Result<(), Refusal> {
    let ok = n >= min && n <= max;
    ensure(ok, msg, path, || format!("a number from {min} to {max}"))
}

pub fn one_of(text: &str, options: &[&str], msg: Msg, path: &str) -> Result<(), Refusal> {
    ensure(options.contains(&text), msg, path, || {
        format!("one of {}", options.join(", "))
    })
}

pub fn matches(
    text: &str,
    test: fn(&str) -> bool,
    msg: Msg,
    path: &str,
    what: &str,
) -> Result<(), Refusal> {
    ensure(test(text), msg, path, || what.to_owned())
}

/// A string from `options` (a wire enum's values), read as `T`; any rejection is `msg`, or the
/// decoder's or the list's own text without one.
pub fn picked<T: DeserializeOwned>(value: J, options: &[&str], msg: Msg) -> Result<T, Refusal> {
    let text: String = decode(value, plain).map_err(|error| msg.map_or(error, str::to_owned))?;
    one_of(&text, options, msg, "")?;
    serde_json::from_value(Value::String(text)).map_err(|error| error.to_string())
}

/// A whole number from `MIN` to `MAX`: any other is refused with the range.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct Bounded<const MIN: u64, const MAX: u64>(pub u64);

impl<const MIN: u64, const MAX: u64> Validate for Bounded<MIN, MAX> {
    fn validate(value: J) -> Result<Self, Refusal> {
        let n: u64 = decode(value, plain)?;
        within(n, MIN, MAX, None, "")?;
        Ok(Bounded(n))
    }
}

/* ── Positions ── */

/// A position: at most 100 characters (`length` when longer), then the FEN shape (`msg`).
pub fn fen(text: &str, length: Msg, msg: Msg, path: &str) -> Result<(), Refusal> {
    max_chars(text, 100, length, path)?;
    matches(text, patterns::fen, msg, path, "a FEN position")
}

/// A position the engine can be given: Stockfish refuses more than 16 pieces for a side, and
/// quits rather than search it.
pub fn engine_position(text: &str) -> Result<(), Refusal> {
    if kchess_domain::position::fen_problem(text) == Some("tooManyPieces") {
        return Err("Each side can have at most 16 pieces.".into());
    }
    Ok(())
}

/// The variant a key names when its rules are not standard chess's; `Invalid variant.` for a key
/// KChess does not play.
pub fn other_variant(key: Option<&str>) -> Result<Option<Variant>, Refusal> {
    match key.map(Variant::from_key) {
        None => Ok(None),
        Some(None) => Err("Invalid variant.".into()),
        Some(Some(variant)) => Ok((!variant.standard_rules()).then_some(variant)),
    }
}

/// A position to analyse in `variant` (standard chess without one), as an analysis request of it
/// alone checks it.
pub fn analysis_position(text: &str, variant: Option<&str>) -> Result<(), Refusal> {
    if let Some(variant) = other_variant(variant)? {
        max_chars(text, 120, None, "fen")?;
        return match replay::replay_setup::<&str>(variant, text, &[]) {
            Some(_) => Ok(()),
            None => Err("Invalid position.".into()),
        };
    }
    fen(text, None, Some("Invalid position."), "fen")?;
    engine_position(text)?;
    if replay::fen_after(text, &[] as &[&str], 0).is_none() {
        return Err("Invalid analysis position or move history.".into());
    }
    Ok(())
}

/// A FEN as sent, read as text ("Invalid position." otherwise); what it must be is the method's
/// to check, as it may depend on another argument (a variant's position).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct Fen(pub String);

impl Validate for Fen {
    fn validate(value: J) -> Result<Self, Refusal> {
        decode(value, |_| Some("Invalid position.")).map(Fen)
    }
}

/// A standard chess position the engine can analyse.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct Position(pub String);

impl Validate for Position {
    fn validate(value: J) -> Result<Self, Refusal> {
        let Fen(text) = Fen::validate(value)?;
        analysis_position(&text, None)?;
        Ok(Position(text))
    }
}

/* ── Accounts ── */

const USERNAME: &str = "Enter a valid Lichess username.";

/// A Lichess username, trimmed.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct Username(pub String);

impl Validate for Username {
    fn validate(value: J) -> Result<Self, Refusal> {
        let text: String = decode(value, |_| Some(USERNAME))?;
        let trimmed = js::trim(&text);
        matches(trimmed, patterns::username, Some(USERNAME), "", "")?;
        Ok(Username(trimmed.to_owned()))
    }
}

/* ── Library ids ── */

/// The id of a library entry (a played game, a drill): 1 to 80 characters.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct LibraryId(pub String);

impl Validate for LibraryId {
    fn validate(value: J) -> Result<Self, Refusal> {
        let id: String = decode(value, plain)?;
        min_chars(&id, 1, None, "")?;
        max_chars(&id, 80, None, "")?;
        Ok(LibraryId(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_optional_argument_is_absent_as_null_or_undefined() {
        assert_eq!(check::<Option<bool>>(J::Undef), Ok(None));
        assert_eq!(check::<Option<bool>>(J::Null), Ok(None));
        assert_eq!(check::<Option<bool>>(J::Bool(true)), Ok(Some(true)));
        assert_eq!(
            check::<Option<bool>>(J::Str("yes".into())),
            Err("invalid type: string \"yes\", expected a boolean".into())
        );
    }

    #[test]
    fn a_bounded_number_names_its_range() {
        assert_eq!(check::<Bounded<1, 5>>(J::Num(3.0)), Ok(Bounded(3)));
        assert_eq!(
            check::<Bounded<1, 5>>(J::Num(6.0)),
            Err("invalid value, expected a number from 1 to 5".into())
        );
    }

    #[test]
    fn a_position_is_checked_by_its_variant_s_rules() {
        let zh = "rnb1kbnr/ppp1pppp/8/3q4/8/8/PPPP1PPP/RNBQKBNR[Pp] w KQkq - 0 3";
        assert_eq!(analysis_position(zh, Some("crazyhouse")), Ok(()));
        assert_eq!(analysis_position(zh, None), Err("Invalid position.".into()));
        assert_eq!(
            analysis_position(zh, Some("bughouse")),
            Err("Invalid variant.".into())
        );
        assert_eq!(
            check::<Position>(J::Num(1.0)),
            Err("Invalid position.".into())
        );
    }
}
