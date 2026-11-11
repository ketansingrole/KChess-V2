//! The `KCHESS_*` switches as this process reads them: the process environment, under values a
//! test sets in memory.
//!
//! Since edition 2024 `std::env::set_var` is `unsafe` (another thread may be reading the
//! environment), so tests never change the environment: they set an override here, which every
//! read of a switch in the core, the service and the apps sees. A service process started from
//! this one inherits the overrides through its command (`forward`).

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::sync::{LazyLock, PoisonError, RwLock};

/// `Some` sets a switch, `None` unsets it, whatever the environment holds.
type Overrides = BTreeMap<String, Option<OsString>>;

static OVERRIDES: LazyLock<RwLock<Overrides>> = LazyLock::new(RwLock::default);

/// The switch `name`: the override, or else the process environment.
pub fn var_os(name: &str) -> Option<OsString> {
    let overrides = OVERRIDES.read().unwrap_or_else(PoisonError::into_inner);
    match overrides.get(name) {
        Some(value) => value.clone(),
        None => std::env::var_os(name),
    }
}

/// The switch `name` as text; `None` when it is unset or not valid Unicode.
pub fn var(name: &str) -> Option<String> {
    var_os(name).and_then(|value| value.into_string().ok())
}

/// Sets the switch `name` for this process, in place of the environment.
pub fn set(name: &str, value: impl AsRef<OsStr>) {
    let mut overrides = OVERRIDES.write().unwrap_or_else(PoisonError::into_inner);
    overrides.insert(name.to_string(), Some(value.as_ref().to_os_string()));
}

/// Unsets the switch `name` for this process, whatever the environment holds.
pub fn unset(name: &str) {
    let mut overrides = OVERRIDES.write().unwrap_or_else(PoisonError::into_inner);
    overrides.insert(name.to_string(), None);
}

/// Drops the override of `name`: the environment's value applies again.
pub fn reset(name: &str) {
    let mut overrides = OVERRIDES.write().unwrap_or_else(PoisonError::into_inner);
    overrides.remove(name);
}

/// Gives a child process the overrides, so it reads the switches this process does.
pub fn forward(command: &mut std::process::Command) {
    let overrides = OVERRIDES.read().unwrap_or_else(PoisonError::into_inner);
    for (name, value) in overrides.iter() {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_win_over_the_environment_until_reset() {
        let name = "KCHESS_ENV_UNIT_TEST";
        assert_eq!(var(name), None);
        set(name, "on");
        assert_eq!(var(name).as_deref(), Some("on"));
        let mut command = std::process::Command::new("unused");
        forward(&mut command);
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == name && value == Some(OsStr::new("on")))
        );
        unset(name);
        assert_eq!(var_os(name), None);
        // PATH is set in every test environment; unsetting hides it, resetting restores it.
        unset("PATH");
        assert_eq!(var_os("PATH"), None);
        reset("PATH");
        assert_eq!(var_os("PATH"), std::env::var_os("PATH"));
        reset(name);
    }
}
