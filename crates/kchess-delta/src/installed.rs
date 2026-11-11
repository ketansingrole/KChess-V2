//! What an installation holds, by the names the packages install it under and the installed app
//! finds it by. Installed apps update in place through them, so they never change.

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
