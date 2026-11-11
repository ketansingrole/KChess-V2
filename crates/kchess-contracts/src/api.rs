//! What one declaration of a core method (`core_api!`, in `core_api.rs`) is made into, and the
//! pieces the generated code shares: argument checks, the assistance gate, the method table.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use kchess_domain::misc::value::J;

use crate::error::CoreError;
use crate::types::ProfileArea;

/// One core method as a typed call.
pub trait CoreCall {
    /// The method's name, as `CoreApi` lists it.
    const METHOD: &'static str;
    /// What the method answers.
    type Answer: DeserializeOwned;
    /// The arguments, in the method's order; an error when one cannot be written as JSON.
    fn args(self) -> Result<Vec<Value>, CoreError>;
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

/// `args` without the `null`s at its end: an optional argument not given is left out, rather
/// than sent as `null` (which the check reads alike).
pub fn without_trailing_nulls(mut args: Vec<Value>) -> Vec<Value> {
    while args.last().is_some_and(Value::is_null) {
        args.pop();
    }
    args
}

pub fn arg(method: &str, value: impl Serialize) -> Result<Value, CoreError> {
    serde_json::to_value(value).map_err(|cause| {
        CoreError::new(format!(
            "An argument of {method} could not be written: {cause}"
        ))
    })
}

pub struct Param {
    pub name: &'static str,
    pub optional: bool,
}

pub struct Method {
    pub name: &'static str,
    /// What the method does, one line per doc line; empty for none.
    pub doc: &'static str,
    pub params: &'static [Param],
    /// Refused during a live human game.
    pub assist: Option<Assist>,
    /// The parts of the profile a successful call changes.
    pub writes: &'static [ProfileArea],
}

/// The assistance a method gives, refused while a live human game is played (or not yet known
/// to be over): before the method runs, and again before its answer is let out, as a game may
/// have started meanwhile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assist {
    Engine,
    Analysis,
    Review,
    /// A stored review: refused until the game status is verified.
    StoredReview,
    Lookups,
    Explain,
}

impl Assist {
    /// Why the method is refused before it runs.
    pub fn refusal(self) -> &'static str {
        match self {
            Assist::StoredReview => {
                "Review is unavailable until your Lichess game status is verified."
            }
            other => other.refusal_after(),
        }
    }

    /// Why its answer is held back once it ran.
    pub fn refusal_after(self) -> &'static str {
        match self {
            Assist::Engine => "Engine assistance is unavailable during a live Lichess game.",
            Assist::Analysis => "Analysis is unavailable during a live Lichess game.",
            Assist::Review | Assist::StoredReview => {
                "Review is unavailable during a live Lichess game."
            }
            Assist::Lookups => "Position lookups are unavailable during a live Lichess game.",
            Assist::Explain => "Explanations are unavailable during a live Lichess game.",
        }
    }
}

#[doc(hidden)]
pub fn count_error(method: &str) -> String {
    format!("Invalid argument count for {method}.")
}

/// The arguments as checks see them: a JSON `null` is an argument not given.
#[doc(hidden)]
pub fn as_checked(args: Vec<Value>) -> Vec<J> {
    args.iter()
        .map(|value| match value {
            Value::Null => J::Undef,
            value => J::from(value),
        })
        .collect()
}

/// The argument at `at` (`J::Undef` when absent) checked as its type.
#[doc(hidden)]
pub fn typed<T: crate::check::Validate>(args: &[J], at: usize) -> Result<T, String> {
    T::validate(args.get(at).cloned().unwrap_or(J::Undef))
}

/// A checked argument as `check_arguments` lists it.
#[doc(hidden)]
pub fn listed(value: &impl Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|cause| cause.to_string())
}

/// A method's answer as it crosses to the caller.
#[doc(hidden)]
pub fn answer(value: impl Serialize) -> Result<Value, CoreError> {
    serde_json::to_value(value).map_err(|cause| CoreError::wrap("core result encoding", cause))
}

/// Declares the core's methods, grouped by area: each line names the typed call, the method,
/// the function the core implements, each argument with its type (the optional ones after `;`),
/// and the answer. An argument is checked by its type (`check::Validate`). `#[check(f)]` checks
/// the arguments together once each passed (`f(&(a, b, …))`), `#[assist(Gate)]` refuses the method
/// during a live game and `#[writes(Area, …)]` reports the profile areas it changes.
macro_rules! core_api {
    (@assist) => { None };
    (@assist $gate:ident) => { Some(Assist::$gate) };
    (@count) => { 0usize };
    (@count $head:ident $($tail:ident)*) => { 1usize + core_api!(@count $($tail)*) };
    ($(
        $(#[doc = $area_doc:literal])*
        trait $area:ident {
            $(
                $(#[doc = $doc:literal])*
                $(#[assist($gate:ident)])?
                $(#[writes($($written:ident),*)])?
                $(#[check($together:path)])?
                $call:ident = $method:literal fn $fn:ident(
                    $($arg:ident : $ty:ty),*
                    $(; $($opt:ident : $opt_ty:ty),+)?
                ) -> $answer:ty;
            )*
        }
    )*) => {
        /// Typed core calls: each names a method, its arguments in order and the shape of its
        /// answer, so a frontend that makes one cannot misspell the method, send arguments of
        /// the wrong shape or misread what comes back.
        pub mod calls {
            use serde_json::Value;
            #[allow(unused_imports)]
            use crate::types::*;
            #[allow(unused_imports)]
            use crate::check::*;

            pub use crate::api::{CoreCall, arg, decode};

            $($(
                $(#[doc = $doc])*
                #[derive(Debug, Clone, PartialEq)]
                pub struct $call {
                    $(pub $arg: $ty,)*
                    $($(pub $opt: $opt_ty,)+)?
                }

                impl CoreCall for $call {
                    const METHOD: &'static str = $method;
                    type Answer = $answer;
                    fn args(self) -> Result<Vec<Value>, crate::error::CoreError> {
                        Ok(crate::api::without_trailing_nulls(vec![
                            $(crate::api::arg($method, self.$arg)?,)*
                            $($(crate::api::arg($method, self.$opt)?,)+)?
                        ]))
                    }
                }
            )*)*
        }

        /// Each method's arguments as it receives them: their count, then each in order (the
        /// first refusal is the answer), then together.
        mod arguments {
            #[allow(unused_imports)]
            use super::*;

            $($(
                #[allow(unused_assignments, unused_mut, unused_variables, clippy::unused_unit)]
                pub fn $fn(
                    method: &str,
                    args: &[J],
                ) -> Result<($($ty,)* $($($opt_ty,)+)?), String> {
                    let required = core_api!(@count $($arg)*);
                    let total = required $(+ core_api!(@count $($opt)+))?;
                    if args.len() < required || args.len() > total {
                        return Err(count_error(method));
                    }
                    let mut at = 0;
                    $(
                        let $arg: $ty = crate::api::typed::<$ty>(args, at)?;
                        at += 1;
                    )*
                    $($(
                        let $opt: $opt_ty = crate::api::typed::<$opt_ty>(args, at)?;
                        at += 1;
                    )+)?
                    let all = ($($arg,)* $($($opt,)+)?);
                    $($together(&all)?;)?
                    Ok(all)
                }
            )*)*
        }

        $(
            $(#[doc = $area_doc])*
            pub trait $area: Sync {
                $(
                    $(#[doc = $doc])*
                    fn $fn(
                        &self,
                        $($arg: $ty,)*
                        $($($opt: $opt_ty,)+)?
                    ) -> impl ::std::future::Future<Output = Result<$answer, CoreError>> + Send;
                )*
            }
        )*

        /// The core's API: every area's methods, and what the generated dispatch asks of it.
        pub trait CoreApi: $($area +)* Sync {
            /// Whether assistance is refused now: a live human game is played, or one may be.
            fn assistance_blocked(&self) -> bool;
            /// A call was refused before it ran: its arguments did not pass their checks.
            fn refused_arguments(&self, method: &str, cause: &str);
            /// A call changed these parts of the profile.
            fn changed(&self, areas: &'static [ProfileArea]);
        }

        /// Tests: a core that answers every method with what `answer_method` gives for its name.
        #[doc(hidden)]
        pub trait AnswerEveryMethod: Sync {
            fn answer_method(&self, method: &'static str) -> Result<Value, CoreError>;
        }

        $(
            #[allow(unused_variables)]
            impl<T: AnswerEveryMethod> $area for T {
                $(
                    fn $fn(
                        &self,
                        $($arg: $ty,)*
                        $($($opt: $opt_ty,)+)?
                    ) -> impl ::std::future::Future<Output = Result<$answer, CoreError>> + Send {
                        ::std::future::ready(self.answer_method($method).and_then(|answer| {
                            serde_json::from_value(answer).map_err(|cause| {
                                CoreError::new(format!("Invalid answer of {}: {cause}", $method))
                            })
                        }))
                    }
                )*
            }
        )*

        pub const METHODS: &[Method] = &[$($(
            Method {
                name: $method,
                doc: concat!($($doc, "\n",)*),
                params: &[
                    $(Param { name: stringify!($arg), optional: false },)*
                    $($(Param { name: stringify!($opt), optional: true },)+)?
                ],
                assist: core_api!(@assist $($gate)?),
                writes: &[$($(ProfileArea::$written),*)?],
            },
        )*)*];

        /// The arguments of `method` checked: their count, then each in order (the first refusal
        /// is the answer), normalized. `None` for an unknown method.
        #[allow(unused_variables)]
        pub fn check_arguments(method: &str, args: Vec<J>) -> Option<Result<Vec<Value>, String>> {
            Some(match method {
                $($(
                    $method => arguments::$fn(method, &args).and_then(
                        |($($arg,)* $($($opt,)+)?)| {
                            Ok(vec![
                                $(crate::api::listed(&$arg)?,)*
                                $($(crate::api::listed(&$opt)?,)+)?
                            ])
                        },
                    ),
                )*)*
                _ => return None,
            })
        }

        /// Runs `method` on `api`: its arguments checked, the assistance gate before and after,
        /// the profile areas it changed reported, and its answer as JSON.
        pub async fn dispatch<A: CoreApi>(
            api: &A,
            method: &str,
            args: Vec<Value>,
        ) -> Result<Value, CoreError> {
            match method {
                $($(
                    $method => {
                        let ($($arg,)* $($($opt,)+)?) =
                            match arguments::$fn(method, &as_checked(args)) {
                                Ok(checked) => checked,
                                Err(cause) => {
                                    api.refused_arguments(method, &cause);
                                    return Err(CoreError::new(cause));
                                }
                            };
                        $(
                            if api.assistance_blocked() {
                                return Err(CoreError::new(Assist::$gate.refusal()));
                            }
                        )?
                        // Boxed: the futures of all the methods would make this one too large
                        // for a worker's stack.
                        let answered = Box::pin(api.$fn($($arg,)* $($($opt,)+)?)).await?;
                        $(
                            if api.assistance_blocked() {
                                return Err(CoreError::new(Assist::$gate.refusal_after()));
                            }
                        )?
                        $(api.changed(&[$(ProfileArea::$written),*]);)?
                        answer(answered)
                    }
                )*)*
                _ => Err(CoreError::new(format!("Unknown core method {method}."))),
            }
        }
    };
}

pub(crate) use core_api;
