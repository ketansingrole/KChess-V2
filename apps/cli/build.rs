//! An 8 MiB main-thread stack on Windows, as macOS and Linux give: MSVC's default is 1 MiB, and
//! the command line reads game trees (up to 1024 plies deep) on its main thread.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/STACK:8388608");
    } else if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg-bins=-Wl,--stack,8388608");
    }
}
