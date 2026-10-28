//! `KCHESS_RELEASE`: the workspace version, or `KCHESS_PACKAGE_VERSION` for a test installer built
//! above every release (`windows-upgrade.yml`), as the app's own `build.rs` reads it.

fn main() {
    println!("cargo:rerun-if-env-changed=KCHESS_PACKAGE_VERSION");
    let release = std::env::var("KCHESS_PACKAGE_VERSION")
        .ok()
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=KCHESS_RELEASE={release}");
}
