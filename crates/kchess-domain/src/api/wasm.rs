//! `on_large_stack` for the WebAssembly build, which has no threads; its own stack
//! (`.cargo/config.toml`) is large enough.

pub fn on_large_stack<T>(
    work: impl FnOnce() -> std::result::Result<T, String>,
) -> std::result::Result<T, String> {
    work()
}
