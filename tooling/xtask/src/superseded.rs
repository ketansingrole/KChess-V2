//! Deletes what a build folder keeps of compiled units that no current build uses: those a newer
//! build replaced, and those left behind by a change nothing will replace them after.
//!
//! Cargo names every unit it compiles `<package>-<hash>`. The hash covers the package's version,
//! its features, its dependencies' hashes, the profile and the compiler, and Cargo never deletes
//! one: its fingerprint (`.fingerprint/<package>-<hash>`), its outputs (`deps/…-<hash>…`,
//! `build/<package>-<hash>`) and rustc's incremental cache for it (`incremental/<crate>-<id>`,
//! up to 1 GB each) stay after a release bump, a lockfile change or a new compiler has given the
//! unit a new hash. By 2026-11-10 the main checkout's `target/debug` held 104 incremental caches
//! of `kchess_service` and 74 of `kchess_core`: 76 GB of incremental caches, 31 GB of `deps`.
//!
//! Age alone says nothing about whether a unit is needed again: Cargo stamps a unit when it
//! compiles it, not when a build finds it fresh and uses it. So a unit is current when Cargo's
//! own records say a build may still use it:
//!
//! - It is the newest build of what it is (its package, target, check or build, own features and
//!   flags) among the units built on their own (nothing depends on them, or they were built
//!   after everything that does). Its older builds are replaced.
//! - Unless its package no longer has its target (a test, binary, example or bench deleted or
//!   renamed: `cargo metadata` of the workspace does not list it), or the same target was built
//!   with other features or flags more than [`LEFT_BEHIND`] after it, and this variant not since.
//!   Nothing will ever build either again under the same identity, so nothing would replace them:
//!   on 2026-11-11 the main checkout kept 86 builds of deleted tests that way.
//! - Or a current unit depends on it, by the fingerprint each fingerprint records for its
//!   dependencies. A dependency rebuilt in place since has a new fingerprint, so that one names
//!   no unit; it keeps the units it may be that were built since the dependent was. Keeping
//!   every unit of its name instead kept all 51 builds of `kchess_core` there.
//!
//! Every other unit goes: its fingerprint, outputs and incremental cache. An incremental cache
//! belongs to the unit whose compilation started it: Cargo stamps a unit's fingerprint folder
//! (`invoked.timestamp`) just before it starts rustc, which names the cache's session after its
//! own start time. A cache is deleted only when every unit of its crate started within a minute
//! before it is not current; one it cannot place is kept.
//!
//! What this gets wrong costs a build, never a wrong one: a variant built less often than every
//! [`LEFT_BEHIND`] while another is built, or a target that exists only on another branch, is
//! compiled again from scratch when next used.
//!
//! It runs with the object sweep (`sweep.rs`) under Cargo's build lock, before `check`,
//! `check-fast` and `app`. On 2026-11-10 `cargo xtask sweep --dry-run` found 922 replaced builds,
//! 63.6 GB, in the main checkout's 105 GB `target/`; on a copy of a smaller one, deleting them
//! kept `cargo build -p kchess-gpui` and `cargo test --workspace` fresh (nothing compiled).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long before an incremental session's start the unit that started it was stamped, at most.
const STARTUP: Duration = Duration::from_secs(60);
/// How far a session's start may precede its unit's stamp: the two clocks are read separately.
const SKEW: Duration = Duration::from_secs(1);
/// How much later another variant of a target (other features or flags) must have been built for
/// one not built since to be left behind: a week of building the other, so a variant used once a
/// week is kept.
pub const LEFT_BEHIND: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Each workspace package's target names, as `cargo metadata` lists them.
pub type Targets = HashMap<String, HashSet<String>>;

/// One compiled unit, from its fingerprint.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    /// The fingerprint folder's name, `<package>-<hash>`.
    pub folder: String,
    /// Its package's name.
    pub package: String,
    /// Whether it is the run of a build script, which its package's library depends on.
    pub build_run: bool,
    /// What it is: two units with the same identity are builds of one thing.
    pub identity: String,
    /// What it builds, however: the identity without its features and flags.
    pub target: String,
    /// Its fingerprint file's stem (`lib-kchess_core`, `test-integration-test-core_close`).
    pub stem: String,
    /// The crate rustc compiles for it, which names its incremental cache.
    pub krate: String,
    /// Its own fingerprint, as its dependents record it.
    pub fingerprint: Option<u64>,
    /// Its dependencies: the crate name it uses each by, and their fingerprints.
    pub dependencies: Vec<(String, u64)>,
    /// When Cargo last started compiling it.
    pub invoked: Option<SystemTime>,
}

/// What a sweep of one profile folder deletes, and how big it is.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub paths: Vec<PathBuf>,
    pub units: usize,
    pub bytes: u64,
}

/// The unit hash in a `deps` file's name (`lib<crate>-<hash>.rlib`, `<crate>-<hash>`,
/// `<crate>-<hash>.<cgu>.rcgu.o`), when it has one.
pub fn hash_of(file: &str) -> Option<&str> {
    let stem = file.split('.').next()?;
    let hash = stem.rsplit_once('-')?.1;
    (hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hash)
}

/// Whether a unit was checked (`cargo check`, clippy: metadata only) or built, from its outputs'
/// file names; neither when it has none in `deps` (a build script's run, or outputs gone).
pub fn mode(outputs: &[&str]) -> &'static str {
    if outputs
        .iter()
        .any(|name| !name.ends_with(".d") && !name.ends_with(".rmeta") && !name.ends_with(".o"))
    {
        "build"
    } else if outputs.iter().any(|name| name.ends_with(".rmeta")) {
        "check"
    } else {
        "other"
    }
}

/// The crate rustc compiles for a fingerprint file's stem (`lib-kchess_core`,
/// `test-integration-test-core_close`, `bin-kchess-fake-uci`, `build-script-build-script-build`).
pub fn crate_of(stem: &str) -> String {
    let stem = stem.strip_prefix("test-").unwrap_or(stem);
    let name = [
        "integration-test-",
        "build-script-",
        "example-",
        "bench-",
        "lib-",
        "bin-",
    ]
    .iter()
    .find_map(|kind| stem.strip_prefix(kind))
    .unwrap_or(stem);
    name.replace('-', "_")
}

/// The unit a fingerprint file describes: `json` is its `<stem>.json`, `own` the hexadecimal
/// fingerprint in the file `<stem>`. `None` when it cannot be read, so the unit is never deleted.
pub fn unit(
    folder: &str,
    stem: &str,
    json: &str,
    own: Option<&str>,
    mode: &str,
    invoked: Option<SystemTime>,
) -> Option<Unit> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let package = folder.rsplit_once('-')?.0;
    let field = |name: &str| value.get(name).map(ToString::to_string).unwrap_or_default();
    let dependencies = value
        .get("deps")?
        .as_array()?
        .iter()
        .filter_map(|dependency| {
            Some((
                dependency.get(1)?.as_str()?.to_string(),
                dependency.get(3)?.as_u64()?,
            ))
        })
        .collect();
    let compile_kind = field("compile_kind");
    Some(Unit {
        folder: folder.to_string(),
        package: package.to_string(),
        build_run: stem.starts_with("run-build-script-"),
        identity: [
            package,
            stem,
            mode,
            &field("features"),
            &field("rustflags"),
            &field("config"),
            &compile_kind,
        ]
        .join(" "),
        target: [package, stem, mode, &compile_kind].join(" "),
        stem: stem.to_string(),
        krate: crate_of(stem),
        fingerprint: own.and_then(fingerprint),
        dependencies,
        invoked,
    })
}

/// A fingerprint as Cargo writes it to a file: its eight bytes, little-endian, in hexadecimal.
pub fn fingerprint(hex: &str) -> Option<u64> {
    let hex = hex.trim();
    if hex.len() != 16 || !hex.is_ascii() {
        return None;
    }
    let mut bytes = [0; 8];
    for (at, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * at..2 * at + 2], 16).ok()?;
    }
    Some(u64::from_le_bytes(bytes))
}

/// The units still in use: the newest of each identity among the units built on their own
/// (nothing depends on them, or they were built after everything that does), unless `removed`
/// says their target is gone or another variant of it left them behind, and what they depend
/// on. A dependency whose fingerprint none has any more was rebuilt since, so it is one of the
/// units it may be (its package's build script runs, or each library of its name) built since
/// the dependent was, and those are kept.
pub fn current(units: &[Unit], removed: impl Fn(&Unit) -> bool) -> HashSet<usize> {
    let mut dependents: HashMap<u64, Vec<usize>> = HashMap::new();
    let mut latest: HashMap<&str, Option<SystemTime>> = HashMap::new();
    for (index, unit) in units.iter().enumerate() {
        for (_, dependency) in &unit.dependencies {
            dependents.entry(*dependency).or_default().push(index);
        }
        let entry = latest.entry(&unit.target).or_insert(unit.invoked);
        *entry = (*entry).max(unit.invoked);
    }
    let mut on_their_own = Vec::new();
    for (index, unit) in units.iter().enumerate() {
        let users = unit
            .fingerprint
            .and_then(|own| dependents.get(&own))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let on_its_own = users.iter().all(|user| units[*user].invoked < unit.invoked);
        if on_its_own {
            on_their_own.push(index);
        }
    }
    let mut newest: HashMap<&str, Option<SystemTime>> = HashMap::new();
    for index in &on_their_own {
        let unit = &units[*index];
        let entry = newest.entry(&unit.identity).or_insert(unit.invoked);
        *entry = (*entry).max(unit.invoked);
    }
    let mut by_fingerprint: HashMap<u64, Vec<usize>> = HashMap::new();
    for (index, unit) in units.iter().enumerate() {
        if let Some(own) = unit.fingerprint {
            by_fingerprint.entry(own).or_default().push(index);
        }
    }
    let left_behind = |unit: &Unit| {
        let latest = latest.get(unit.target.as_str()).copied().flatten();
        unit.invoked.zip(latest).is_some_and(|(built, latest)| {
            latest
                .duration_since(built)
                .is_ok_and(|gap| gap > LEFT_BEHIND)
        })
    };
    let mut live: HashSet<usize> = HashSet::new();
    // Builds of one thing stamped at the same moment are all kept.
    let mut stack: Vec<usize> = on_their_own
        .into_iter()
        .filter(|index| {
            let unit = &units[*index];
            newest.get(unit.identity.as_str()) == Some(&unit.invoked)
                && !left_behind(unit)
                && !removed(unit)
        })
        .collect();
    while let Some(index) = stack.pop() {
        if !live.insert(index) {
            continue;
        }
        let unit = &units[index];
        for (name, dependency) in &unit.dependencies {
            match by_fingerprint.get(dependency) {
                Some(found) => stack.extend(found),
                None => stack.extend(units.iter().enumerate().filter_map(|(other, candidate)| {
                    let may_be = if name == "build_script_build" {
                        candidate.build_run && candidate.package == unit.package
                    } else {
                        !candidate.build_run && candidate.krate == *name
                    };
                    let since = candidate.invoked.is_none() || candidate.invoked >= unit.invoked;
                    (may_be && since).then_some(other)
                })),
            }
        }
    }
    live
}

/// The name of the target a fingerprint file's stem builds, when it is one its package may drop
/// (a test, binary, example or bench); `None` for its library and build script, which last as
/// long as the package.
pub fn droppable(stem: &str) -> Option<&str> {
    let stem = stem.strip_prefix("test-").unwrap_or(stem);
    ["integration-test-", "example-", "bench-", "bin-"]
        .iter()
        .find_map(|kind| stem.strip_prefix(kind))
}

/// Whether `unit` builds a target its workspace package no longer has. A package not in
/// `targets` (a dependency's, or one this cannot tell) keeps every target.
pub fn removed(unit: &Unit, targets: &Targets) -> bool {
    targets
        .get(&unit.package)
        .is_some_and(|names| droppable(&unit.stem).is_some_and(|name| !names.contains(name)))
}

/// Each package's target names in `cargo metadata --no-deps` output; `None` when it cannot be
/// read.
pub fn targets_of(metadata: &str) -> Option<Targets> {
    let value: serde_json::Value = serde_json::from_str(metadata).ok()?;
    value
        .get("packages")?
        .as_array()?
        .iter()
        .map(|package| {
            let names = package
                .get("targets")?
                .as_array()?
                .iter()
                .map(|target| Some(target.get("name")?.as_str()?.to_string()))
                .collect::<Option<HashSet<String>>>()?;
            Some((package.get("name")?.as_str()?.to_string(), names))
        })
        .collect()
}

/// The targets of the workspace at `workspace`, from Cargo: `None` when it has none or Cargo
/// cannot say, so no target counts as removed.
pub fn workspace_targets(workspace: &Path) -> Option<Targets> {
    let output = std::process::Command::new("cargo")
        .args([
            "metadata",
            "--no-deps",
            "--offline",
            "--format-version",
            "1",
        ])
        .arg("--manifest-path")
        .arg(workspace.join("Cargo.toml"))
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    targets_of(&String::from_utf8_lossy(&output.stdout))
}

/// When an incremental session folder (`s-<start>-<random>[-<id>]`) started: rustc names it
/// with its start time in microseconds, in base 36.
pub fn session_start(name: &str) -> Option<SystemTime> {
    let start = name.strip_prefix("s-")?.split('-').next()?;
    let micros = u64::from_str_radix(start, 36).ok()?;
    UNIX_EPOCH.checked_add(Duration::from_micros(micros))
}

/// Whether the incremental cache of `krate` whose newest session started at `start` belongs only
/// to superseded units: every unit of that crate stamped in the minute before is one, and there
/// is at least one.
pub fn cache_superseded(
    krate: &str,
    start: SystemTime,
    units: &[Unit],
    live: &HashSet<usize>,
) -> bool {
    let mut starters = units.iter().enumerate().filter(|(_, unit)| {
        unit.krate == krate
            && unit.invoked.is_some_and(|invoked| {
                invoked <= start + SKEW
                    && start
                        .duration_since(invoked)
                        .map_or(true, |gap| gap <= STARTUP)
            })
    });
    let first = starters.next();
    first.is_some_and(|(index, _)| !live.contains(&index))
        && starters.all(|(index, _)| !live.contains(&index))
}

/// Everything a profile folder (`target/debug`) keeps of units no current build uses, given its
/// workspace's `targets` (`None`: no target counts as removed).
pub fn plan(profile: &Path, targets: Option<&Targets>) -> Plan {
    let deps = profile.join("deps");
    let files: Vec<String> = names(&deps);
    let mut outputs: HashMap<&str, Vec<&str>> = HashMap::new();
    for file in &files {
        if let Some(hash) = hash_of(file) {
            outputs.entry(hash).or_default().push(file);
        }
    }
    let mut units = Vec::new();
    let mut unreadable = HashSet::new();
    for folder in names(&profile.join(".fingerprint")) {
        let path = profile.join(".fingerprint").join(&folder);
        let invoked = crate::util::modified(&path.join("invoked.timestamp"));
        let hash = folder.rsplit('-').next().unwrap_or_default();
        let mode = mode(outputs.get(hash).map(Vec::as_slice).unwrap_or_default());
        for json in names(&path).iter().filter(|name| name.ends_with(".json")) {
            let stem = json.trim_end_matches(".json");
            let read = std::fs::read_to_string(path.join(json)).ok();
            let own = std::fs::read_to_string(path.join(stem)).ok();
            match read.and_then(|text| unit(&folder, stem, &text, own.as_deref(), mode, invoked)) {
                Some(unit) => units.push(unit),
                None => {
                    unreadable.insert(folder.clone());
                }
            }
        }
    }
    let live = current(&units, |unit| {
        targets.is_some_and(|targets| removed(unit, targets))
    });
    let stale: HashSet<&str> = units
        .iter()
        .map(|unit| unit.folder.as_str())
        .filter(|folder| !unreadable.contains(*folder))
        .filter(|folder| {
            units
                .iter()
                .enumerate()
                .filter(|(_, unit)| unit.folder == *folder)
                .all(|(index, _)| !live.contains(&index))
        })
        .collect();
    let mut plan = Plan {
        units: stale.len(),
        ..Plan::default()
    };
    let mut add = |path: PathBuf| {
        plan.bytes += size(&path);
        plan.paths.push(path);
    };
    for cache in names(&profile.join("incremental")) {
        let Some((krate, _)) = cache.rsplit_once('-') else {
            continue;
        };
        let path = profile.join("incremental").join(&cache);
        // A session's lock (`s-<start>-<random>.lock`) carries its start too.
        let start = names(&path)
            .iter()
            .filter_map(|session| session_start(session))
            .max();
        if start.is_some_and(|start| cache_superseded(krate, start, &units, &live)) {
            add(path);
        }
    }
    let stale_hashes: HashSet<&str> = stale
        .iter()
        .filter_map(|folder| folder.rsplit('-').next())
        .collect();
    for file in &files {
        if hash_of(file).is_some_and(|hash| stale_hashes.contains(hash)) {
            add(deps.join(file));
        }
    }
    for folder in &stale {
        let build = profile.join("build").join(folder);
        if build.exists() {
            add(build);
        }
        add(profile.join(".fingerprint").join(folder));
    }
    plan
}

/// Deletes what `plan` names: how many bytes went.
pub fn delete(plan: &Plan) -> u64 {
    let mut deleted = 0;
    for path in &plan.paths {
        let size = size(path);
        let gone = if path.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        };
        if gone.is_ok() {
            deleted += size;
        }
    }
    deleted
}

fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// A file's size, or a folder's files' sizes summed; links are not followed.
fn size(path: &Path) -> u64 {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !metadata.is_dir() {
        return metadata.len();
    }
    std::fs::read_dir(path)
        .map(|entries| entries.flatten().map(|entry| size(&entry.path())).sum())
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A second, one minute.
    fn at(seconds: u64) -> Option<SystemTime> {
        UNIX_EPOCH.checked_add(Duration::from_secs(1_794_000_000 + seconds))
    }

    /// A unit of `package` built as `stem` at `invoked`, with fingerprint `own`, depending on
    /// `dependencies` (name, fingerprint).
    fn unit_of(
        folder: &str,
        stem: &str,
        own: u64,
        dependencies: &[(&str, u64)],
        invoked: u64,
    ) -> Unit {
        let deps: Vec<String> = dependencies
            .iter()
            .map(|(name, value)| format!("[1, \"{name}\", false, {value}]"))
            .collect();
        let json = format!("{{\"features\":\"[]\",\"deps\":[{}]}}", deps.join(","));
        let hex: String = own
            .to_le_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        unit(folder, stem, &json, Some(&hex), "build", at(invoked)).unwrap()
    }

    /// Writes a unit's fingerprint folder into `profile` as Cargo does.
    pub(crate) fn write_unit(
        profile: &Path,
        folder: &str,
        stem: &str,
        json: &str,
        own: u64,
        invoked: u64,
    ) {
        let path = profile.join(".fingerprint").join(folder);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join(format!("{stem}.json")), json).unwrap();
        let hex: String = own
            .to_le_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        std::fs::write(path.join(stem), hex).unwrap();
        let stamp = path.join("invoked.timestamp");
        let file = std::fs::File::create(&stamp).unwrap();
        file.set_modified(at(invoked).unwrap()).unwrap();
    }

    /// One library built twice in `profile`, the older one's `.rlib` `size` bytes (sparse): the
    /// older build is the one a sweep deletes.
    pub(crate) fn replaced_build(profile: &Path, size: u64) {
        let deps = profile.join("deps");
        std::fs::create_dir_all(&deps).unwrap();
        let json = "{\"features\":\"[]\",\"deps\":[]}";
        write_unit(profile, "old-0000000000000001", "lib-old", json, 1, 0);
        write_unit(profile, "old-0000000000000002", "lib-old", json, 2, 10);
        let rlib = std::fs::File::create(deps.join("libold-0000000000000001.rlib")).unwrap();
        rlib.set_len(size).unwrap();
        std::fs::write(deps.join("libold-0000000000000002.rlib"), b"new").unwrap();
    }

    #[test]
    fn a_deps_file_names_its_units_hash() {
        assert_eq!(
            hash_of("libkchess_core-0123456789abcdef.rlib"),
            Some("0123456789abcdef")
        );
        assert_eq!(
            hash_of("core_close-0123456789abcdef.07k3iqkm.10lfo85.rcgu.o"),
            Some("0123456789abcdef")
        );
        assert_eq!(hash_of("KChess-0123456789abcdef"), Some("0123456789abcdef"));
        assert_eq!(hash_of("KChess"), None);
        assert_eq!(hash_of("short-0123456789abcde"), None);
        assert_eq!(hash_of("long-0123456789abcdef0"), None);
        assert_eq!(hash_of("bad-0123456789abcdeg"), None);
    }

    #[test]
    fn its_outputs_tell_a_check_from_a_build() {
        assert_eq!(mode(&["a-1.d", "liba-1.rmeta"]), "check");
        assert_eq!(mode(&["a-1.d", "liba-1.rmeta", "liba-1.rlib"]), "build");
        assert_eq!(mode(&["a-1.d", "a-1", "a-1.x.y.rcgu.o"]), "build");
        assert_eq!(mode(&["a-1.d", "a-1.x.y.rcgu.o"]), "other");
        assert_eq!(mode(&[]), "other");
    }

    #[test]
    fn a_stem_names_the_crate_rustc_compiles() {
        assert_eq!(crate_of("lib-kchess_core"), "kchess_core");
        assert_eq!(crate_of("test-lib-kchess_core"), "kchess_core");
        assert_eq!(crate_of("test-integration-test-core_close"), "core_close");
        assert_eq!(crate_of("bin-kchess-fake-uci"), "kchess_fake_uci");
        assert_eq!(crate_of("test-bin-KChess"), "KChess");
        assert_eq!(crate_of("example-fetch_stockfish"), "fetch_stockfish");
        assert_eq!(crate_of("bench-speed"), "speed");
        assert_eq!(
            crate_of("build-script-build-script-build"),
            "build_script_build"
        );
        assert_eq!(crate_of("odd"), "odd");
    }

    #[test]
    fn a_fingerprint_file_holds_little_endian_bytes() {
        assert_eq!(
            fingerprint("efcdab8967452301\n"),
            Some(0x0123_4567_89ab_cdef)
        );
        assert_eq!(fingerprint("efcdab896745230"), None);
        assert_eq!(fingerprint("efcdab89674523010"), None);
        assert_eq!(fingerprint("efcdab896745230g"), None);
        assert_eq!(fingerprint("éfcdab8967452301"), None);
    }

    #[test]
    fn a_unit_is_what_it_builds_and_how() {
        let json = "{\"features\":\"[\\\"std\\\"]\",\"rustflags\":[],\"deps\":[[7, \"serde\", false, 42]]}";
        let built = unit(
            "serde_json-0123456789abcdef",
            "lib-serde_json",
            json,
            Some("2a00000000000000"),
            "build",
            at(5),
        )
        .unwrap();
        assert_eq!(built.package, "serde_json");
        assert_eq!(built.krate, "serde_json");
        assert_eq!(built.fingerprint, Some(42));
        assert_eq!(built.dependencies, [("serde".to_string(), 42)]);
        assert_eq!(built.invoked, at(5));
        assert!(!built.build_run);
        let checked = unit(
            "serde_json-1123456789abcdef",
            "lib-serde_json",
            json,
            None,
            "check",
            None,
        )
        .unwrap();
        assert_ne!(built.identity, checked.identity, "a check is not a build");
        assert_eq!(checked.fingerprint, None);
        let other = json.replace("std", "alloc");
        let alloc = unit(
            "serde_json-2123456789abcdef",
            "lib-serde_json",
            &other,
            None,
            "build",
            None,
        )
        .unwrap();
        assert_ne!(
            built.identity, alloc.identity,
            "other features are another thing"
        );
        let again = unit(
            "serde_json-3123456789abcdef",
            "lib-serde_json",
            json,
            None,
            "build",
            None,
        )
        .unwrap();
        assert_eq!(built.identity, again.identity);
        assert!(
            unit(
                "a-1",
                "run-build-script-build-script-build",
                json,
                None,
                "other",
                None
            )
            .unwrap()
            .build_run
        );
        assert_eq!(unit("a-1", "lib-a", "{", None, "build", None), None);
        assert_eq!(unit("a-1", "lib-a", "{}", None, "build", None), None);
        assert_eq!(unit("nohash", "lib-a", json, None, "build", None), None);
    }

    #[test]
    fn a_newer_build_of_the_same_thing_replaces_the_older_and_what_only_it_used() {
        let units = [
            unit_of("dep-0000000000000001", "lib-dep", 11, &[], 0),
            unit_of("app-0000000000000001", "bin-app", 12, &[("dep", 11)], 1),
            unit_of("dep-0000000000000002", "lib-dep", 21, &[], 10),
            unit_of("app-0000000000000002", "bin-app", 22, &[("dep", 21)], 11),
            // Something nothing replaced: kept, however old.
            unit_of("tool-0000000000000001", "bin-tool", 31, &[("dep", 11)], 0),
        ];
        let live = current(&units, |_| false);
        assert_eq!(live, HashSet::from([2, 3, 4, 0]));
    }

    #[test]
    fn a_unit_built_on_its_own_after_its_users_is_current() {
        let units = [
            unit_of("lib-0000000000000001", "lib-lib", 11, &[], 0),
            unit_of("app-0000000000000001", "bin-app", 12, &[("lib", 11)], 1),
            unit_of("app-0000000000000002", "bin-app", 13, &[], 5),
            // The same library again, built after its old user: current.
            unit_of("lib-0000000000000002", "lib-lib", 14, &[], 9),
            // Built before its user, but that user is replaced: not.
            unit_of("lib-0000000000000003", "lib-lib", 15, &[], 0),
            unit_of("app-0000000000000003", "bin-app", 16, &[("lib", 15)], 2),
        ];
        assert_eq!(current(&units, |_| false), HashSet::from([2, 3]));
        assert_eq!(current(&[], |_| false), HashSet::new());
    }

    #[test]
    fn a_unit_used_since_or_at_once_is_not_built_on_its_own() {
        let units = [
            // Built after its only user, which another build replaced: current.
            unit_of("lib-0000000000000001", "lib-lib", 11, &[], 9),
            unit_of("app-0000000000000001", "bin-app", 12, &[("lib", 11)], 1),
            unit_of("app-0000000000000002", "bin-app", 13, &[], 5),
            // Stamped with its replaced user at the same moment: not.
            unit_of("dep-0000000000000001", "lib-dep", 14, &[], 3),
            unit_of("use-0000000000000001", "bin-use", 15, &[("dep", 14)], 3),
            unit_of("use-0000000000000002", "bin-use", 16, &[], 4),
        ];
        assert_eq!(current(&units, |_| false), HashSet::from([0, 2, 5]));
    }

    #[test]
    fn builds_of_one_thing_stamped_at_once_are_all_kept() {
        let units = [
            unit_of("a-0000000000000001", "bin-a", 1, &[], 7),
            unit_of("a-0000000000000002", "bin-a", 2, &[], 7),
            unit_of("a-0000000000000003", "bin-a", 3, &[], 6),
        ];
        assert_eq!(current(&units, |_| false), HashSet::from([0, 1]));
    }

    #[test]
    fn a_dependency_no_fingerprint_names_keeps_what_it_may_be_built_since() {
        let run = "run-build-script-build-script-build";
        let units = [
            // Built before the library below, used by a replaced program: not what it uses.
            unit_of("ring-0000000000000001", "lib-ring", 3, &[], 0),
            unit_of("a-0000000000000001", run, 1, &[], 0),
            unit_of(
                "x-0000000000000001",
                "bin-x",
                50,
                &[("build_script_build", 1), ("ring", 3)],
                50,
            ),
            unit_of("x-0000000000000002", "bin-x", 60, &[], 60),
            // Built since the library (or in its second), used by another replaced program.
            unit_of("ring-0000000000000002", "lib-ring", 5, &[], 80),
            unit_of("a-0000000000000002", run, 6, &[], 75),
            unit_of("ring-0000000000000003", "lib-ring", 7, &[], 70),
            // Since too, but another package's build script and a build script's run.
            unit_of("b-0000000000000001", run, 8, &[], 80),
            unit_of("ring-0000000000000004", run, 9, &[], 80),
            unit_of(
                "z-0000000000000001",
                "bin-z",
                95,
                &[
                    ("ring", 5),
                    ("build_script_build", 6),
                    ("ring", 7),
                    ("build_script_build", 8),
                    ("build_script_build", 9),
                ],
                95,
            ),
            unit_of("z-0000000000000002", "bin-z", 96, &[], 96),
            // Its build script and `ring` were rebuilt since it was.
            unit_of(
                "a-0000000000000003",
                "lib-a",
                70,
                &[("build_script_build", 99), ("ring", 98)],
                70,
            ),
        ];
        assert_eq!(
            current(&units, |_| false),
            HashSet::from([3, 4, 5, 6, 10, 11])
        );
        assert_eq!(current(&units[..11], |_| false), HashSet::from([3, 10]));
    }

    #[test]
    fn a_variant_another_left_behind_goes_and_one_still_built_stays() {
        let day = 24 * 60 * 60;
        let features = |folder: &str, features: &str, invoked: u64| {
            let json = format!("{{\"features\":\"{features}\",\"deps\":[[1, \"dep\", false, 1]]}}");
            unit(folder, "test-lib-core", &json, None, "build", at(invoked)).unwrap()
        };
        let units = [
            unit_of("dep-0000000000000001", "lib-dep", 1, &[], 0),
            // The tests built with other features, before a change made every build use one set.
            features("core-0000000000000001", "[a]", 0),
            // Still built within the week since the newest.
            features("core-0000000000000002", "[b]", 7 * day - 1),
            features("core-0000000000000003", "[c]", 8 * day),
        ];
        assert_eq!(current(&units, |_| false), HashSet::from([0, 2, 3]));
        // A unit no time is known for is never left behind.
        let mut unknown = units.clone();
        unknown[1].invoked = None;
        assert_eq!(current(&unknown, |_| false), HashSet::from([0, 1, 2, 3]));
        // Only another build of the same target leaves one behind.
        assert_eq!(current(&units[..2], |_| false), HashSet::from([0, 1]));
    }

    #[test]
    fn a_target_its_package_no_longer_has_is_not_built_again() {
        let units = [
            unit_of("core-0000000000000001", "lib-core", 1, &[], 0),
            unit_of(
                "core-0000000000000002",
                "test-integration-test-gone",
                2,
                &[("core", 1)],
                1,
            ),
            unit_of(
                "core-0000000000000003",
                "test-integration-test-kept",
                3,
                &[("core", 1)],
                1,
            ),
        ];
        let targets = Targets::from([(
            "core".to_string(),
            HashSet::from(["core".to_string(), "kept".to_string()]),
        )]);
        let live = current(&units, |unit| removed(unit, &targets));
        assert_eq!(live, HashSet::from([0, 2]));
        assert!(
            !removed(&units[0], &targets),
            "a library lasts as its package"
        );
        assert!(
            !removed(&units[1], &Targets::new()),
            "a package it cannot tell keeps all"
        );
    }

    #[test]
    fn a_stem_names_the_targets_a_package_may_drop() {
        assert_eq!(
            droppable("test-integration-test-core_close"),
            Some("core_close")
        );
        assert_eq!(droppable("integration-test-core_close"), Some("core_close"));
        assert_eq!(droppable("test-bin-KChess"), Some("KChess"));
        assert_eq!(droppable("bin-kchess-fake-uci"), Some("kchess-fake-uci"));
        assert_eq!(droppable("example-fetch"), Some("fetch"));
        assert_eq!(droppable("test-bench-speed"), Some("speed"));
        assert_eq!(droppable("lib-kchess_core"), None);
        assert_eq!(droppable("test-lib-kchess_core"), None);
        assert_eq!(droppable("build-script-build-script-build"), None);
        assert_eq!(droppable("run-build-script-build-script-build"), None);
    }

    #[test]
    fn cargo_metadata_names_each_packages_targets() {
        let metadata = r#"{"packages":[
            {"name":"kchess-core","targets":[{"name":"kchess_core"},{"name":"core_close"}]},
            {"name":"xtask","targets":[{"name":"xtask"}]}
        ]}"#;
        assert_eq!(
            targets_of(metadata),
            Some(Targets::from([
                (
                    "kchess-core".to_string(),
                    HashSet::from(["kchess_core".to_string(), "core_close".to_string()])
                ),
                ("xtask".to_string(), HashSet::from(["xtask".to_string()])),
            ]))
        );
        assert_eq!(targets_of("{"), None);
        assert_eq!(targets_of("{}"), None);
        assert_eq!(targets_of(r#"{"packages":[{"name":"a"}]}"#), None);
        assert_eq!(targets_of(r#"{"packages":[{"targets":[]}]}"#), None);
        assert_eq!(
            targets_of(r#"{"packages":[{"name":"a","targets":[{}]}]}"#),
            None
        );
    }

    #[test]
    fn cargo_lists_a_workspaces_targets() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("tests/close.rs"), "").unwrap();
        assert_eq!(
            workspace_targets(root),
            Some(Targets::from([(
                "probe".to_string(),
                HashSet::from(["probe".to_string(), "close".to_string()])
            )]))
        );
        assert_eq!(workspace_targets(&root.join("missing")), None);
    }

    #[test]
    fn rustc_names_a_session_after_its_start() {
        let start = session_start("s-ho14srlmjg-0raw5x5-dc25jcfuxll1ymvw9buf264yd").unwrap();
        assert_eq!(
            start.duration_since(UNIX_EPOCH).unwrap(),
            Duration::from_micros(1_794_314_711_847_004)
        );
        assert_eq!(
            session_start("s-ho14srlmjg-0raw5x5.lock"),
            session_start("s-ho14srlmjg")
        );
        assert_eq!(session_start("ho14srlmjg"), None);
        assert_eq!(session_start("s-!!"), None);
    }

    #[test]
    fn a_cache_goes_only_when_every_unit_that_may_have_started_it_is_replaced() {
        let units = [
            unit_of("a-0000000000000001", "lib-a", 1, &[], 100),
            unit_of("a-0000000000000002", "test-lib-a", 2, &[], 100),
            unit_of("a-0000000000000003", "lib-a", 3, &[], 300),
            unit_of("b-0000000000000001", "lib-b", 4, &[], 100),
        ];
        let start = |seconds| at(seconds).unwrap();
        let live = HashSet::from([1, 2]);
        assert!(
            !cache_superseded("a", start(101), &units, &live),
            "the test's may be"
        );
        assert!(cache_superseded(
            "a",
            start(101),
            &units,
            &HashSet::from([2])
        ));
        assert!(
            cache_superseded("a", start(160), &units, &HashSet::from([2])),
            "a minute after"
        );
        assert!(
            !cache_superseded("a", start(161), &units, &HashSet::new()),
            "too late to place"
        );
        assert!(
            !cache_superseded("a", start(98), &units, &HashSet::new()),
            "before any"
        );
        assert!(
            cache_superseded("a", start(100), &units, &HashSet::from([2])),
            "the same second"
        );
        assert!(
            !cache_superseded("c", start(101), &units, &HashSet::new()),
            "nothing of its crate"
        );
        assert!(cache_superseded("b", start(101), &units, &live));
    }

    #[test]
    fn a_profile_folder_loses_exactly_what_a_newer_build_replaced() {
        let profile = tempfile::tempdir().unwrap();
        let profile = profile.path();
        replaced_build(profile, 1000);
        let deps = profile.join("deps");
        for name in [
            "old-0000000000000001.d",
            "old-0000000000000001.x.y.rcgu.o",
            "old-0000000000000002.d",
            "unknown-00000000000000ff.d",
            "plain.txt",
        ] {
            std::fs::write(deps.join(name), b"1").unwrap();
        }
        // The build script of the old build, and an unreadable unit, which is kept.
        std::fs::create_dir_all(profile.join("build/old-0000000000000001/out")).unwrap();
        std::fs::write(profile.join("build/old-0000000000000001/out/x"), b"12").unwrap();
        write_unit(profile, "odd-0000000000000003", "lib-odd", "{", 7, 0);
        // Each build's incremental cache, started just after it, and one no unit started.
        let session = |seconds: u64| {
            let micros = (1_794_000_000 + seconds) * 1_000_000 + 50_000;
            let mut digits = Vec::new();
            let mut left = micros;
            while left > 0 {
                digits.push(char::from_digit((left % 36) as u32, 36).unwrap());
                left /= 36;
            }
            format!("s-{}-abc-def", digits.iter().rev().collect::<String>())
        };
        for (cache, seconds) in [("old-aaa", 0), ("old-bbb", 10), ("old-ccc", 500)] {
            let path = profile
                .join("incremental")
                .join(cache)
                .join(session(seconds));
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("dep-graph.bin"), b"1234").unwrap();
            std::fs::write(path.with_extension("lock"), b"").unwrap();
        }
        std::fs::create_dir_all(profile.join("incremental/nodash")).unwrap();
        let planned = plan(profile, None);
        let mut gone: Vec<String> = planned
            .paths
            .iter()
            .map(|path| {
                // One separator on every platform, so the list below holds on Windows too.
                let relative = path.strip_prefix(profile).unwrap();
                let parts: Vec<_> = relative.iter().map(|part| part.to_string_lossy()).collect();
                parts.join("/")
            })
            .collect();
        gone.sort();
        assert_eq!(
            gone,
            [
                ".fingerprint/old-0000000000000001",
                "build/old-0000000000000001",
                "deps/libold-0000000000000001.rlib",
                "deps/old-0000000000000001.d",
                "deps/old-0000000000000001.x.y.rcgu.o",
                "incremental/old-aaa",
            ]
        );
        assert_eq!(planned.units, 1);
        // The rlib, both one-byte files, the build script's output, the fingerprint (`{…}`
        // json, 16-digit fingerprint, empty stamp) and the cache's graph.
        let json = "{\"features\":\"[]\",\"deps\":[]}".len() as u64;
        assert_eq!(planned.bytes, 1000 + 2 + 2 + json + 16 + 4);
        assert_eq!(delete(&planned), planned.bytes);
        assert!(planned.paths.iter().all(|path| !path.exists()));
        assert!(deps.join("libold-0000000000000002.rlib").exists());
        assert!(profile.join("incremental/old-bbb").exists());
        assert!(profile.join(".fingerprint/odd-0000000000000003").exists());
        assert_eq!(plan(profile, None), Plan::default(), "nothing more to go");
        assert_eq!(plan(&profile.join("missing"), None), Plan::default());
    }
}
