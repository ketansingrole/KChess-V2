//! Where a frontend finds what it starts the core with: the profile folder, and the programs a
//! package installs beside the app (by the names `kchess_delta::installed` gives them). Both the
//! app and the command line read it here, so a profile and its engines mean the same to both.
//!
//! A profile other than the app's own (`KCHESS_USER_DATA_DIR`, a test's, `--data-dir`) is
//! isolated: it keeps its own downloaded engine and never imports an earlier release's database,
//! so working in it never touches the person's.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use kchess_core::{Config, TestSwitches};
use kchess_delta::installed;

use crate::handle::Mode;

/// The product name, which names the profile folder.
pub const APP_NAME: &str = installed::PROFILE;

/// How a variable of the environment is read.
type Var = Box<dyn Fn(&str) -> Option<OsString>>;

/// What the layout is read from: the environment, the running executable's folder and this
/// repository's `target` folder (development builds find what `cargo xtask` built there).
pub struct Layout {
    var: Var,
    /// The folder of the running executable.
    programs: Option<PathBuf>,
    /// This repository's build folder, where a development run finds what `cargo xtask` built.
    target: PathBuf,
    /// Whether the development overrides are honoured (`kchess_host::dev_overrides`).
    dev: bool,
}

impl Layout {
    /// This process's layout.
    pub fn current() -> Layout {
        Layout {
            var: Box::new(kchess_host::env::var_os),
            programs: std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(Path::to_path_buf)),
            target: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"),
            dev: kchess_host::dev_overrides(),
        }
    }

    /// A variable; set but empty counts as unset.
    fn var(&self, name: &str) -> Option<OsString> {
        (self.var)(name).filter(|value| !value.is_empty())
    }

    /// A development override (`kchess_host::dev_override`).
    fn dev_path(&self, name: &str) -> Option<PathBuf> {
        self.dev
            .then(|| self.var(name))
            .flatten()
            .map(PathBuf::from)
    }

    fn home(&self) -> PathBuf {
        self.var("HOME")
            .or_else(|| self.var("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_default()
    }

    /// The app's own profile (`kchess_delta::installed::own_profile`).
    pub fn own_profile(&self) -> PathBuf {
        installed::own_profile(|name| self.var(name))
    }

    /// The profile a frontend opens unless told another: `KCHESS_USER_DATA_DIR`, else its own.
    pub fn profile(&self) -> PathBuf {
        self.var("KCHESS_USER_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.own_profile())
    }

    /// Whether `profile` is another than the app's own.
    pub fn isolated(&self, profile: &Path) -> bool {
        profile != self.own_profile()
    }

    /// `name` beside the running executable, when it is there.
    fn beside(&self, name: impl AsRef<Path>) -> Option<PathBuf> {
        let path = self.programs.as_ref()?.join(name);
        path.exists().then_some(path)
    }

    /// `name` in this repository's build folder, when it was built and this is a development
    /// run: a packaged build never starts a program from the build machine's folders.
    fn built(&self, path: impl AsRef<Path>) -> Option<PathBuf> {
        if !self.dev {
            return None;
        }
        let path = self.target.join(path);
        path.exists().then_some(path)
    }

    /// The core's configuration over `profile`. `system_credentials`: the core seals logins with
    /// the profile's own credentials.
    pub fn config(&self, profile: &Path, system_credentials: bool) -> Config {
        let isolated = self.isolated(profile);
        let home = self.home();
        let stockfish = installed::program(installed::STOCKFISH);
        let fairy = installed::program(installed::FAIRY_STOCKFISH);
        let voice = Path::new(installed::VOICE).join(installed::program(installed::VOICE_HELPER));
        Config {
            managed_engine_dir: Some(if isolated {
                profile.join("engines/stockfish/current")
            } else {
                home.join(".kchess/engines/stockfish/current")
            }),
            // Beside the app (`Contents/MacOS` in the macOS bundle); development builds ship none.
            bundled_engine_path: self
                .dev_path("KCHESS_ENGINE_PATH")
                .or_else(|| Some(self.programs.as_ref()?.join(&stockfish))),
            variant_engine_path: self
                .dev_path("KCHESS_VARIANT_ENGINE_PATH")
                .or_else(|| self.beside(&fairy))
                .or_else(|| self.built(Path::new(installed::BUILT_FAIRY).join(&fairy))),
            // Beside the executable, in a Mac bundle's Resources, or staged by
            // `cargo xtask voice-runtime`.
            voice_helper_path: self
                .dev_path("KCHESS_VOICE_HELPER")
                .or_else(|| self.beside(&voice))
                .or_else(|| self.beside(Path::new("../Resources").join(&voice)))
                .or_else(|| {
                    self.built(
                        Path::new(installed::BUILT_VOICE)
                            .join(installed::program(installed::VOICE_HELPER)),
                    )
                }),
            legacy_database_path: legacy_database(&home, isolated, cfg!(target_os = "macos")),
            system_credentials,
            test: TestSwitches::from_vars(self.dev, |name| {
                self.var(name).and_then(|value| value.into_string().ok())
            }),
            data_dir: profile.to_path_buf(),
        }
    }

    /// The `kchess-service` program: `KCHESS_SERVICE_BINARY`, else the one beside the app.
    pub fn service_binary(&self) -> Option<PathBuf> {
        self.dev_path("KCHESS_SERVICE_BINARY")
            .or_else(|| self.beside(installed::program(installed::SERVICE)))
    }

    /// Where a frontend's core runs: in its own process when `embedded` asks for it or no
    /// service ships beside it, else the profile's shared service. A development run may choose
    /// with `KCHESS_CORE_MODE` (`embedded` or `service`).
    pub fn mode(&self, embedded: bool) -> Result<Mode, String> {
        let service = self.service_binary().map(|binary| Mode::Service { binary });
        let chosen = self.dev_path("KCHESS_CORE_MODE");
        match chosen.as_ref().and_then(|mode| mode.to_str()) {
            _ if embedded => Ok(Mode::Embedded),
            Some("embedded") => Ok(Mode::Embedded),
            Some("service") => service.ok_or_else(|| "The KChess core service is missing".into()),
            _ => Ok(service.unwrap_or(Mode::Embedded)),
        }
    }
}

/// The database a macOS release before the profile folders kept, imported once into the person's
/// own profile; an isolated profile imports nothing.
fn legacy_database(home: &Path, isolated: bool, macos: bool) -> Option<PathBuf> {
    (macos && !isolated).then(|| home.join(".kchess/kchess.db"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(pairs: &[(&str, &str)], programs: &Path) -> Layout {
        let pairs: Vec<(String, OsString)> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), OsString::from(value)))
            .collect();
        Layout {
            var: Box::new(move |name| {
                pairs
                    .iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.clone())
            }),
            programs: Some(programs.to_path_buf()),
            target: programs.join("target"),
            dev: true,
        }
    }

    const HOME: &str = "/home/someone";

    fn home() -> PathBuf {
        PathBuf::from(HOME)
    }

    /// `KCHESS_USER_DATA_DIR` names the profile; set but empty, it is ignored.
    #[test]
    fn the_profile_is_the_variable_else_the_apps_own() {
        let programs = tempfile::tempdir().expect("programs");
        let own = layout(&[("HOME", HOME)], programs.path()).profile();
        assert!(own.ends_with(APP_NAME), "{own:?}");
        assert!(own.starts_with(home()), "{own:?}");
        let scratch = layout(
            &[("HOME", HOME), ("KCHESS_USER_DATA_DIR", "/scratch")],
            programs.path(),
        );
        assert_eq!(scratch.profile(), PathBuf::from("/scratch"));
        let empty = layout(
            &[("HOME", HOME), ("KCHESS_USER_DATA_DIR", "")],
            programs.path(),
        );
        assert_eq!(empty.profile(), own);
    }

    /// 2026-11-11: `kchess-cli --data-dir <scratch>` installed and deleted the engine in the
    /// person's own `~/.kchess/engines`, which the app's scratch profiles never touch. Any
    /// profile but the app's own keeps its engine, and imports no earlier database.
    #[test]
    fn a_scratch_profile_keeps_its_own_engine() {
        let programs = tempfile::tempdir().expect("programs");
        let layout = layout(&[("HOME", HOME)], programs.path());
        let scratch = std::env::temp_dir().join("scratch-profile");
        let config = layout.config(&scratch, true);
        assert_eq!(config.data_dir, scratch);
        assert_eq!(
            config.managed_engine_dir,
            Some(scratch.join("engines/stockfish/current"))
        );
        assert_eq!(config.legacy_database_path, None);
        assert!(config.system_credentials);

        let own = layout.config(&layout.own_profile(), false);
        assert_eq!(
            own.managed_engine_dir,
            Some(home().join(".kchess/engines/stockfish/current"))
        );
        assert_eq!(
            own.legacy_database_path,
            cfg!(target_os = "macos").then(|| home().join(".kchess/kchess.db"))
        );
    }

    /// Only macOS releases kept an earlier database, and only the person's own profile imports it.
    #[test]
    fn only_a_macos_profile_of_ones_own_imports_the_earlier_database() {
        let home = home();
        for (isolated, macos, wanted) in [
            (false, true, Some(home.join(".kchess/kchess.db"))),
            (true, true, None),
            (false, false, None),
            (true, false, None),
        ] {
            assert_eq!(
                legacy_database(&home, isolated, macos),
                wanted,
                "isolated {isolated}, macOS {macos}"
            );
        }
    }

    /// The development overrides win; a packaged build ignores every one of them.
    #[test]
    fn the_programs_are_the_overrides_else_the_installed_ones() {
        let programs = tempfile::tempdir().expect("programs");
        let fairy = installed::program(installed::FAIRY_STOCKFISH);
        std::fs::write(programs.path().join(&fairy), b"").expect("fairy");
        let voice = programs.path().join(installed::VOICE);
        std::fs::create_dir_all(&voice).expect("voice");
        let helper = voice.join(installed::program(installed::VOICE_HELPER));
        std::fs::write(&helper, b"").expect("helper");
        let service = programs.path().join(installed::program(installed::SERVICE));
        std::fs::write(&service, b"").expect("service");
        let pairs = [
            ("HOME", HOME),
            ("KCHESS_ENGINE_PATH", "/engines/stockfish"),
            ("KCHESS_VARIANT_ENGINE_PATH", "/engines/fairy"),
            ("KCHESS_STORE_DEBUG", "1"),
            ("KCHESS_TEST_LICHESS_BASE", "http://127.0.0.1:9/"),
        ];
        let mut overridden = layout(&pairs, programs.path());
        let config = overridden.config(Path::new("/profile"), false);
        assert_eq!(
            config.bundled_engine_path,
            Some(PathBuf::from("/engines/stockfish"))
        );
        assert_eq!(
            config.variant_engine_path,
            Some(PathBuf::from("/engines/fairy"))
        );
        assert_eq!(config.voice_helper_path, Some(helper.clone()));
        assert_eq!(
            config.test.and_then(|test| test.lichess_origin).as_deref(),
            Some("http://127.0.0.1:9")
        );
        assert_eq!(overridden.service_binary(), Some(service.clone()));

        overridden.dev = false;
        let packaged = overridden.config(Path::new("/profile"), false);
        assert_eq!(
            packaged.bundled_engine_path,
            Some(
                programs
                    .path()
                    .join(installed::program(installed::STOCKFISH))
            )
        );
        assert_eq!(
            packaged.variant_engine_path,
            Some(programs.path().join(&fairy))
        );
        assert_eq!(packaged.test, None);
        assert_eq!(overridden.service_binary(), Some(service));
    }

    fn mode_name(mode: Result<Mode, String>) -> String {
        match mode {
            Ok(Mode::Embedded) => "embedded".into(),
            Ok(Mode::Service { binary }) => binary.display().to_string(),
            Err(message) => message,
        }
    }

    /// `KCHESS_CORE_MODE` chooses in a development run, and a frontend's own request to embed
    /// wins; else the service beside the app, when there is one.
    #[test]
    fn the_core_runs_in_the_service_beside_the_app_unless_told_otherwise() {
        let programs = tempfile::tempdir().expect("programs");
        let none = layout(
            &[("HOME", HOME), ("KCHESS_CORE_MODE", "service")],
            programs.path(),
        );
        assert_eq!(
            mode_name(none.mode(false)),
            "The KChess core service is missing"
        );
        assert_eq!(mode_name(none.mode(true)), "embedded");
        let service = programs.path().join(installed::program(installed::SERVICE));
        std::fs::write(&service, b"").expect("service");
        let shared = layout(&[("HOME", HOME)], programs.path());
        assert_eq!(mode_name(shared.mode(false)), service.display().to_string());
        assert_eq!(mode_name(shared.mode(true)), "embedded");
        let mut embedded = layout(
            &[("HOME", HOME), ("KCHESS_CORE_MODE", "embedded")],
            programs.path(),
        );
        assert_eq!(mode_name(embedded.mode(false)), "embedded");
        // 2026-11-11: a packaged app honoured `KCHESS_CORE_MODE`, a development switch.
        embedded.dev = false;
        assert_eq!(
            mode_name(embedded.mode(false)),
            service.display().to_string()
        );
    }

    /// Without a program beside the app, development builds use what `cargo xtask` built.
    #[test]
    fn a_development_build_finds_what_xtask_built() {
        let programs = tempfile::tempdir().expect("programs");
        let layout = layout(&[("HOME", HOME)], programs.path());
        assert_eq!(
            layout.config(Path::new("/p"), true).variant_engine_path,
            None
        );
        let built = programs
            .path()
            .join("target/fairy-stockfish/bin")
            .join(installed::program(installed::FAIRY_STOCKFISH));
        std::fs::create_dir_all(built.parent().expect("bin")).expect("bin");
        std::fs::write(&built, b"").expect("built");
        assert_eq!(
            layout.config(Path::new("/p"), true).variant_engine_path,
            Some(built)
        );
        assert_eq!(layout.service_binary(), None);
    }

    /// 2026-11-11: a packaged build without Fairy-Stockfish or the voice helper beside it looked
    /// for them in the build machine's `target` folder, at a fixed path on the user's disk.
    #[test]
    fn a_packaged_build_never_looks_in_the_build_folder() {
        let programs = tempfile::tempdir().expect("programs");
        let mut layout = layout(&[("HOME", HOME)], programs.path());
        layout.dev = false;
        let fairy = programs
            .path()
            .join("target/fairy-stockfish/bin")
            .join(installed::program(installed::FAIRY_STOCKFISH));
        let helper = programs
            .path()
            .join("target/voice-runtime")
            .join(installed::program(installed::VOICE_HELPER));
        for built in [&fairy, &helper] {
            std::fs::create_dir_all(built.parent().expect("folder")).expect("folder");
            std::fs::write(built, b"").expect("built");
        }
        let config = layout.config(Path::new("/p"), true);
        assert_eq!(config.variant_engine_path, None);
        assert_eq!(config.voice_helper_path, None);
    }
}
