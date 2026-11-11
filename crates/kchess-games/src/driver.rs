//! The one session driver behind `ComputerGame` and `LocalGame`: it keeps the state a session's
//! transitions write and the runtime they carry, and runs each transition with the session's own
//! host reads.

use serde_json::{Value, json};

use crate::{Transition, assign_written, transition};

pub(crate) struct Driver {
    /// The session's document, as the rules wrote it.
    pub state: Value,
    runtime: Value,
}

impl Driver {
    /// A session from `state` (the rules' reading of a saved one), started by `init`.
    pub fn start(
        state: Value,
        init: Transition,
        read: impl FnMut(&str) -> Result<Value, String>,
        what: &str,
    ) -> Result<Driver, String> {
        let runtime = transition(init, &[], read)?
            .get("runtime")
            .cloned()
            .ok_or_else(|| format!("A {what} could not start."))?;
        Ok(Driver { state, runtime })
    }

    /// Run `step` after the state and runtime, keep what it wrote, and hand back the finished
    /// transition (its `value` and `effects`).
    pub fn apply(
        &mut self,
        step: Transition,
        input: &[Value],
        read: impl FnMut(&str) -> Result<Value, String>,
    ) -> Result<Value, String> {
        let mut args = vec![self.state.clone(), self.runtime.clone()];
        args.extend_from_slice(input);
        let mut done = transition(step, &args, read)?;
        if let Some(next) = done.get("state") {
            assign_written(&mut self.state, next, &done["written"]);
        }
        if let Some(runtime) = done.get_mut("runtime") {
            self.runtime = runtime.take();
        }
        Ok(done)
    }

    /// A getter of the session (`view` with `{ view: name, …query }`); nothing changes.
    pub fn view(
        &self,
        step: Transition,
        query: Value,
        read: impl FnMut(&str) -> Result<Value, String>,
    ) -> Value {
        transition(
            step,
            &[self.state.clone(), self.runtime.clone(), query],
            read,
        )
        .ok()
        .and_then(|mut done| done.get_mut("value").map(Value::take))
        .unwrap_or(Value::Null)
    }
}

/// A finished transition's value.
pub(crate) fn value(done: &Value) -> Value {
    done.get("value").cloned().unwrap_or(Value::Null)
}

/// A view query by name.
pub(crate) fn named(view: &str) -> Value {
    json!({ "view": view })
}
