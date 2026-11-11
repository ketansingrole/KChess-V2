//! What an installation holds, by the names the packages install it under and the installed app
//! finds it by. Installed apps update in place through them, so they never change.

use std::ffi::OsString;
use std::path::PathBuf;

/// The macOS app bundle.
pub const MAC_BUNDLE: &str = "KChess.app";
/// The programs installed beside the app, each named as here plus the platform's executable
/// suffix ([`program`]).
pub const STOCKFISH: &str = "stockfish";
pub const FAIRY_STOCKFISH: &str = "fairy-stockfish";
pub const SERVICE: &str = "kchess-service";
pub const CLI: &str = "kchess-cli";
/// The folder beside the app that holds the speech helper ([`VOICE_HELPER`]) and libvosk.
pub const VOICE: &str = "voice";
pub const VOICE_HELPER: &str = "kchess-voice";

/// `name` as this platform's executable file (`stockfish.exe` on Windows).
pub fn program(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// The folder in the platform's application data that holds the app's own profile.
pub const PROFILE: &str = "KChess";

/// Fairy-Stockfish's program folder in the repository's `target` folder, where a development
/// build finds what `cargo xtask fairy-stockfish` built.
pub const BUILT_FAIRY: &str = "fairy-stockfish/bin";
/// The speech helper with libvosk in the repository's `target` folder, where a development
/// build finds what `cargo xtask voice-runtime` staged.
pub const BUILT_VOICE: &str = "voice-runtime";

/// The app's own profile: [`PROFILE`] in this platform's application data, the environment read
/// through `var` (set but empty counts as unset): `~/Library/Application Support` on macOS,
/// `%APPDATA%` on Windows, and `$XDG_CONFIG_HOME` (when absolute) or `~/.config` elsewhere.
pub fn own_profile(var: impl Fn(&str) -> Option<OsString>) -> PathBuf {
    let var = |name: &str| var(name).filter(|value| !value.is_empty());
    let home = var("HOME")
        .or_else(|| var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support").join(PROFILE)
    } else if cfg!(windows) {
        var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"))
            .join(PROFILE)
    } else {
        var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .unwrap_or_else(|| home.join(".config"))
            .join(PROFILE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn the_own_profile_is_in_the_platforms_application_data() {
        let profile = own_profile(env(&[
            ("HOME", "/home/me"),
            ("XDG_CONFIG_HOME", "/xdg"),
            ("APPDATA", "C:/Users/me/AppData/Roaming"),
        ]));
        let expected = if cfg!(target_os = "macos") {
            "/home/me/Library/Application Support/KChess"
        } else if cfg!(windows) {
            "C:/Users/me/AppData/Roaming/KChess"
        } else {
            "/xdg/KChess"
        };
        assert_eq!(profile, PathBuf::from(expected));
        if cfg!(all(unix, not(target_os = "macos"))) {
            let relative = own_profile(env(&[("HOME", "/home/me"), ("XDG_CONFIG_HOME", "xdg")]));
            assert_eq!(relative, PathBuf::from("/home/me/.config/KChess"));
            let empty = own_profile(env(&[("HOME", "/home/me"), ("XDG_CONFIG_HOME", "")]));
            assert_eq!(empty, PathBuf::from("/home/me/.config/KChess"));
        }
    }
}
