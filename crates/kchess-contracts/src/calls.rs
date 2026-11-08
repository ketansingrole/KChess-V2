//! Typed core calls: each names a method, its arguments in order and the shape of its answer, so
//! a frontend that makes one cannot misspell the method, send arguments of the wrong shape or
//! misread what comes back. A frontend still reaches the core by method name (`AppCore::request`
//! in the app sends `METHOD` with `args()` and decodes the answer with `decode`).
//!
//! `CALLS` lists every typed call's method and argument count; the core's tests check each
//! against the facade's methods and the arities the validator accepts
//! (`crates/kchess-core/tests/core/facade_dispatch.rs`).

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::CoreError;
use crate::types::{CustomThemeReport, Settings};

/// One core method as a typed call.
pub trait CoreCall {
    /// The method's name, as `CoreApi` (or the facade, for its own methods) lists it.
    const METHOD: &'static str;
    /// What the method answers.
    type Answer: DeserializeOwned;
    /// The arguments, in the method's order.
    fn args(self) -> Vec<Value>;
}

/// The core's answer to `C` in its shape, or an error that names the method.
pub fn decode<C: CoreCall>(answer: Value) -> Result<C::Answer, CoreError> {
    serde_json::from_value(answer).map_err(|cause| {
        CoreError::new(format!(
            "The core's answer to {} could not be read: {cause}",
            C::METHOD
        ))
    })
}

/// An argument as JSON. The contract types always serialize; `null` stands in if one did not.
fn arg(value: impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

macro_rules! calls {
    (@count) => { 0 };
    (@count $head:ident $($tail:ident)*) => { 1 + calls!(@count $($tail)*) };
    ($(
        $(#[$doc:meta])*
        $name:ident = $method:literal ($($field:ident: $ty:ty),*) -> $answer:ty;
    )*) => {
        $(
            $(#[$doc])*
            #[derive(Debug, Clone, PartialEq)]
            pub struct $name {
                $(pub $field: $ty,)*
            }

            impl CoreCall for $name {
                const METHOD: &'static str = $method;
                type Answer = $answer;
                fn args(self) -> Vec<Value> {
                    vec![$(arg(self.$field)),*]
                }
            }
        )*

        /// Every typed call's method and how many arguments it sends.
        pub const CALLS: &[(&str, usize)] = &[$(($method, calls!(@count $($field)*))),*];
    };
}

calls! {
    /// Every setting as stored: the core's preferences and the desktop's.
    DesktopSettings = "desktopSettings" () -> Settings;
    /// Store `settings`, edited from `base`: only what changed since `base` is written, so another
    /// window's changes stay. Answers the settings as stored.
    SaveDesktopSettings = "saveDesktopSettings" (settings: Settings, base: Settings) -> Settings;
    /// The user's own app themes, and a line per theme file that could not be used.
    CustomThemes = "customThemes" () -> CustomThemeReport;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings() -> Settings {
        crate::defaults::default_settings().expect("the defaults decode")
    }

    #[test]
    fn each_call_sends_its_arguments_in_order() {
        assert!(DesktopSettings {}.args().is_empty());
        assert!(CustomThemes {}.args().is_empty());
        let mut base = settings();
        let edited = settings();
        base.frontend.sound_enabled = !edited.frontend.sound_enabled;
        let args = SaveDesktopSettings {
            settings: edited.clone(),
            base: base.clone(),
        }
        .args();
        assert_eq!(args, [arg(&edited), arg(&base)]);
        assert_ne!(args[0], args[1]);
        assert_eq!(
            CALLS,
            [
                ("desktopSettings", 0),
                ("saveDesktopSettings", 2),
                ("customThemes", 0)
            ]
        );
    }

    #[test]
    fn an_answer_is_read_in_the_call_s_shape() {
        let stored = settings();
        assert_eq!(decode::<DesktopSettings>(arg(&stored)), Ok(stored));
        let report = json!({ "themes": [], "dir": "/themes", "problems": ["bad.json: no name"] });
        let read = decode::<CustomThemes>(report).expect("a report");
        assert_eq!(read.dir, "/themes");
        assert_eq!(read.problems, ["bad.json: no name"]);
        let failed = decode::<SaveDesktopSettings>(json!(12)).expect_err("not settings");
        assert!(
            failed
                .message
                .starts_with("The core's answer to saveDesktopSettings could not be read: "),
            "{}",
            failed.message
        );
    }
}
