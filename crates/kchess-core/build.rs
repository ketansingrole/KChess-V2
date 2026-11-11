//! The tests that need a real engine run only when its path is set when they are built
//! (`KCHESS_ENGINE_PATH` for Stockfish, `KCHESS_VARIANT_ENGINE_PATH` for Fairy-Stockfish); without
//! it they are ignored, so the run reports them as skipped rather than passed.

fn main() {
    for (variable, cfg) in [
        ("KCHESS_ENGINE_PATH", "kchess_stockfish"),
        ("KCHESS_VARIANT_ENGINE_PATH", "kchess_fairy_stockfish"),
    ] {
        println!("cargo::rerun-if-env-changed={variable}");
        println!("cargo::rustc-check-cfg=cfg({cfg})");
        if std::env::var_os(variable).is_some_and(|path| !path.is_empty()) {
            println!("cargo::rustc-cfg={cfg}");
        }
    }
}
