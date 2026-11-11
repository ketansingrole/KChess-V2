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

/// `args` without the `null`s at its end: an optional argument not given is left out, rather
/// than sent as `null` (which the check reads alike).
pub fn without_trailing_nulls(mut args: Vec<Value>) -> Vec<Value> {
    while args.last().is_some_and(Value::is_null) {
        args.pop();
    }
    args
}

/// An argument as JSON. The contract types always serialize; `null` stands in if one did not.
pub fn arg(value: impl Serialize) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// One argument of a method.
pub struct Param {
    pub name: &'static str,
    pub optional: bool,
}

/// One method of the core's API.
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

/// What an argument check answers: the normalized argument, or why it was refused.
pub type Checked = Result<Value, String>;

/// One argument's check, as the declaration names it: a validator of the argument alone
/// (`fn(J) -> Checked`), or of the argument among the others (`fn(J, &[J]) -> Checked`), or one
/// wrapped in `Optional` or `Raw`. `Marker` only tells these apart.
pub trait ArgCheck<Marker> {
    /// The argument (`J::Undef` when absent) among all of them.
    fn check(&self, value: J, args: &[J]) -> Checked;
}

#[doc(hidden)]
pub struct Alone;
#[doc(hidden)]
pub struct Among;
#[doc(hidden)]
pub struct Skipped<M>(M);
#[doc(hidden)]
pub struct AsSent<M>(M);

impl<F: Fn(J) -> Checked> ArgCheck<Alone> for F {
    fn check(&self, value: J, _: &[J]) -> Checked {
        self(value)
    }
}

impl<F: Fn(J, &[J]) -> Checked> ArgCheck<Among> for F {
    fn check(&self, value: J, args: &[J]) -> Checked {
        self(value, args)
    }
}

/// A check run only on an argument that was given: an absent one is `null`.
pub struct Optional<C>(pub C);

impl<M, C: ArgCheck<M>> ArgCheck<Skipped<M>> for Optional<C> {
    fn check(&self, value: J, args: &[J]) -> Checked {
        match value {
            J::Undef => Ok(Value::Null),
            value => self.0.check(value, args),
        }
    }
}

/// A check whose method reads the argument as sent, not as the check normalizes it.
pub struct Raw<C>(pub C);

impl<M, C: ArgCheck<M>> ArgCheck<AsSent<M>> for Raw<C> {
    fn check(&self, value: J, args: &[J]) -> Checked {
        self.0.check(value.clone(), args)?;
        Ok(value.into_value())
    }
}

#[doc(hidden)]
pub fn run_check<M>(check: &impl ArgCheck<M>, args: &[J], at: usize) -> Checked {
    check.check(args.get(at).cloned().unwrap_or(J::Undef), args)
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

/// A normalized argument in the type the method takes.
#[doc(hidden)]
pub fn take<T: DeserializeOwned>(
    method: &str,
    at: usize,
    value: Option<Value>,
) -> Result<T, CoreError> {
    serde_json::from_value(value.unwrap_or(Value::Null)).map_err(|cause| {
        CoreError::new(format!("Invalid argument {} of {method}: {cause}", at + 1))
    })
}

/// A method's answer as it crosses to the caller.
#[doc(hidden)]
pub fn answer(value: impl Serialize) -> Result<Value, CoreError> {
    serde_json::to_value(value).map_err(|cause| CoreError::wrap("core result encoding", cause))
}

/// Declares the core's methods, grouped by area: each line names the typed call, the method,
/// the function the core implements, each argument with its type and check (the optional ones
/// after `;`), and the answer. `#[assist(Gate)]` refuses it during a live game and
/// `#[writes(Area, …)]` reports the profile areas it changes.
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
                $call:ident = $method:literal fn $fn:ident(
                    $($arg:ident : $ty:ty = $check:expr),*
                    $(; $($opt:ident : $opt_ty:ty = $opt_check:expr),+)?
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

            pub use crate::api::{CoreCall, decode};

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
                    fn args(self) -> Vec<Value> {
                        crate::api::without_trailing_nulls(vec![
                            $(crate::api::arg(self.$arg),)*
                            $($(crate::api::arg(self.$opt),)+)?
                        ])
                    }
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
                        ::std::future::ready(
                            self.answer_method($method)
                                .and_then(|answer| take($method, 0, Some(answer))),
                        )
                    }
                )*
            }
        )*

        /// Every method, in declaration order.
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
        #[allow(unused_assignments, unused_mut, unused_variables)]
        pub fn check_arguments(method: &str, args: Vec<J>) -> Option<Result<Vec<Value>, String>> {
            Some(match method {
                $($(
                    $method => (|| {
                        let required = core_api!(@count $($arg)*);
                        let total = required $(+ core_api!(@count $($opt)+))?;
                        if args.len() < required || args.len() > total {
                            return Err(count_error(method));
                        }
                        let mut checked = Vec::with_capacity(total);
                        let mut at = 0;
                        $(
                            checked.push(run_check(&$check, &args, at)?);
                            at += 1;
                        )*
                        $($(
                            checked.push(run_check(&$opt_check, &args, at)?);
                            at += 1;
                        )+)?
                        Ok(checked)
                    })(),
                )*)*
                _ => return None,
            })
        }

        /// Runs `method` on `api`: its arguments checked, the assistance gate before and after,
        /// the profile areas it changed reported, and its answer as JSON.
        #[allow(unused_assignments, unused_mut, unused_variables)]
        pub async fn dispatch<A: CoreApi>(
            api: &A,
            method: &str,
            args: Vec<Value>,
        ) -> Result<Value, CoreError> {
            let checked = match check_arguments(method, as_checked(args)) {
                None => return Err(CoreError::new(format!("Unknown core method {method}."))),
                Some(Err(cause)) => {
                    api.refused_arguments(method, &cause);
                    return Err(CoreError::new(cause));
                }
                Some(Ok(checked)) => checked,
            };
            let mut values = checked.into_iter();
            let mut at = 0;
            match method {
                $($(
                    $method => {
                        $(
                            let $arg: $ty = take(method, at, values.next())?;
                            at += 1;
                        )*
                        $($(
                            let $opt: $opt_ty = take(method, at, values.next())?;
                            at += 1;
                        )+)?
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
