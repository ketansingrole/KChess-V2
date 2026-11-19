//! What each workspace member is: the one table `check`, `licenses`, `hack`, `mutants` and
//! `package` ask whether a member ships, is the app, or is tooling. A member it does not name is
//! a headless library: shipped only through the programs that depend on it, tested under the
//! coverage gate.

/// What a workspace member is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The GPUI app: shipped, its views checked by its end-to-end suite rather than by coverage,
    /// its tests under `cargo test` (one binary whose tests each start GPUI).
    App,
    /// A GPUI crate the app is built from (`crates/kchess-ui`, `-app`, `-board`, `-nav`): shipped
    /// inside the app, its views checked by the app's end-to-end suite rather than by coverage,
    /// its own tests run with the headless members'.
    Ui,
    /// A program every package ships beside the app.
    Program,
    /// A headless library.
    Library,
    /// This tooling, checked by running it.
    Tooling,
    /// The workspace hack (`crates/kchess-hack`): every member depends on it only so their
    /// dependencies build with one feature set; nothing ships its own dependencies.
    Hack,
}

/// The app's package.
pub const APP: &str = "kchess-gpui";

/// The app's sources from the repository root: its own and its UI crates' (`Role::Ui`), which
/// the app's rules of look (`boundaries`) and its icons (`icons`) are read from.
pub const APP_SOURCES: [&str; 10] = [
    "apps/gpui/src",
    "crates/kchess-ui/src",
    "crates/kchess-app/src",
    "crates/kchess-board/src",
    "crates/kchess-nav/src",
    "crates/kchess-puzzles/src",
    "crates/kchess-history/src",
    "crates/kchess-home/src",
    "crates/kchess-local/src",
    "crates/kchess-practice/src",
];

/// The app's sources, as cargo-mutants' file globs name them.
pub const APP_FILES: &str = "apps/gpui/**";

const ROLES: [(&str, Role); 15] = [
    (APP, Role::App),
    ("kchess-ui", Role::Ui),
    ("kchess-app", Role::Ui),
    ("kchess-board", Role::Ui),
    ("kchess-nav", Role::Ui),
    ("kchess-puzzles", Role::Ui),
    ("kchess-history", Role::Ui),
    ("kchess-home", Role::Ui),
    ("kchess-local", Role::Ui),
    ("kchess-practice", Role::Ui),
    ("kchess-service", Role::Program),
    ("kchess-cli", Role::Program),
    ("kchess-voice", Role::Program),
    ("xtask", Role::Tooling),
    ("kchess-hack", Role::Hack),
];

/// The role of the member `name`.
pub fn role(name: &str) -> Role {
    ROLES
        .iter()
        .find(|(member, _)| *member == name)
        .map_or(Role::Library, |(_, role)| *role)
}

/// The crates whose programs every package ships: the app, the core service, the command line
/// and the speech helper. They are built for release, attributed (`licenses`), and checked for
/// `unwrap` (`check`).
pub fn shipped() -> impl Iterator<Item = &'static str> {
    ROLES
        .iter()
        .filter(|(_, role)| matches!(role, Role::App | Role::Program))
        .map(|(name, _)| *name)
}

/// The members outside the coverage totals: the app's views (its own and its UI crates') are
/// checked by its end-to-end suite, and this tooling by running it. Their tests still run.
pub fn uncovered() -> impl Iterator<Item = &'static str> {
    ROLES
        .iter()
        .filter(|(_, role)| matches!(role, Role::App | Role::Ui | Role::Tooling))
        .map(|(name, _)| *name)
}

/// Whether `name` is headless: neither the app, nor its UI crates, nor tooling, nor the hack. The
/// hack never makes a headless member build the app's or xtask's dependencies.
pub fn headless(name: &str) -> bool {
    matches!(role(name), Role::Program | Role::Library)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every member the table names is in the workspace, so a renamed one cannot fall back to a
    /// library unnoticed.
    #[test]
    fn the_table_names_workspace_members() {
        let members = crate::check::members().unwrap();
        for (name, _) in ROLES {
            assert!(members.iter().any(|member| member == name), "{name}");
        }
        assert_eq!(role("kchess-core"), Role::Library);
        assert!(headless("kchess-core") && headless("kchess-cli"));
        assert!(!headless(APP) && !headless("xtask") && !headless("kchess-hack"));
        assert!(!headless("kchess-ui") && !headless("kchess-app") && !headless("kchess-board"));
        assert!(!headless("kchess-nav") && !headless("kchess-puzzles"));
        assert!(!headless("kchess-history") && !headless("kchess-home"));
        assert!(!headless("kchess-local") && !headless("kchess-practice"));
    }

    /// The app, its UI crates and this tooling are outside the coverage totals; the programs ship.
    #[test]
    fn the_app_and_tooling_are_uncovered() {
        assert_eq!(
            uncovered().collect::<Vec<_>>(),
            [
                APP,
                "kchess-ui",
                "kchess-app",
                "kchess-board",
                "kchess-nav",
                "kchess-puzzles",
                "kchess-history",
                "kchess-home",
                "kchess-local",
                "kchess-practice",
                "xtask"
            ]
        );
        assert_eq!(
            shipped().collect::<Vec<_>>(),
            [APP, "kchess-service", "kchess-cli", "kchess-voice"]
        );
    }

    #[test]
    fn the_app_sources_are_the_apps() {
        let manifest = crate::util::root()
            .join(APP_FILES.trim_end_matches("**"))
            .join("Cargo.toml");
        let text = std::fs::read_to_string(manifest).unwrap();
        assert!(text.contains(&format!("name = \"{APP}\"")));
    }
}
