//! What the core and its frontends read alike of the process they run in. Frontends name this
//! crate rather than `kchess-core` for it (tooling/xtask/src/boundaries.rs).

pub mod env;
mod redact;

pub use redact::redact;

/// Whether this is the Mac App Store's KChess. Only that build runs in the App Sandbox, which
/// sets `APP_SANDBOX_CONTAINER_ID` for the app and every program it starts. The App Store updates
/// it and forbids downloading programs, so its updater and the Stockfish download stay off.
pub fn app_store() -> bool {
    cfg!(target_os = "macos") && std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_some()
}

/// Whether this build honours the development overrides: the test switches (`Config::test`), and
/// the frontends' `KCHESS_ENGINE_PATH`, `KCHESS_VARIANT_ENGINE_PATH`, `KCHESS_VOICE_HELPER`,
/// `KCHESS_SERVICE_BINARY`, `KCHESS_CORE_MODE`, `KCHESS_RUNTIME_DIR` and `KCHESS_UPDATE_FEED`,
/// and the programs `cargo xtask` built in this repository's `target` folder. A packaged build
/// (`cargo xtask package` builds with the `packaged` feature) never does, so software that can
/// set the user's environment cannot make a signed KChess start its programs or send its logins
/// elsewhere.
pub const fn dev_overrides() -> bool {
    !cfg!(feature = "packaged")
}

/// The development override `name` (see `dev_overrides`), when this build honours it and it is
/// set to something.
pub fn dev_override(name: &str) -> Option<std::ffi::OsString> {
    if !dev_overrides() {
        return None;
    }
    env::var_os(name).filter(|value| !value.is_empty())
}

/// Why a frontend refused to open a link (`opens_in_browser`).
pub const NOT_A_WEB_PAGE: &str = "Only web addresses open in the browser.";

/// Whether a frontend may open `url` for `openExternal`: any `https://` page, or a plain-http
/// page on this machine's loopback address (`127.0.0.1`, as the test servers listen). The URL is
/// parsed, so `http://127.0.0.1.example.com/` or `http://127.0.0.1@example.com/` is not local,
/// and nothing with a login in it opens.
pub fn opens_in_browser(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    match parsed.scheme() {
        "https" => parsed.host_str().is_some(),
        "http" => parsed.host_str() == Some("127.0.0.1"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_pages_and_this_machine_open() {
        assert!(opens_in_browser("https://lichess.org/oauth?state=x"));
        assert!(opens_in_browser("http://127.0.0.1:8123/callback"));
        for refused in [
            "http://127.0.0.1.evil.example/",
            "http://127.0.0.1@evil.example/",
            "https://user:pass@lichess.org/",
            "http://lichess.org/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "-https://lichess.org",
            "",
        ] {
            assert!(!opens_in_browser(refused), "{refused}");
        }
    }

    #[test]
    fn an_empty_override_is_none() {
        let name = "KCHESS_HOST_UNIT_TEST";
        env::set(name, "");
        assert_eq!(dev_override(name), None);
        env::set(name, "x");
        assert_eq!(
            dev_override(name).as_deref(),
            dev_overrides().then_some(std::ffi::OsStr::new("x"))
        );
        env::reset(name);
    }
}
