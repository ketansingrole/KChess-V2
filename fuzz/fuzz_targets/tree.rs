//! A PGN's analysis tree, built on one thread and written and dropped on a 1 MiB thread as
//! Windows' main thread is: a tree that recursed once a ply aborted the app there.
#![no_main]

use kchess_domain::tree;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|pgn: &str| {
    let Some(built) = tree::tree_from_pgn(pgn) else {
        return;
    };
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            let _ = built.to_pgn();
            drop(built);
        })
        .unwrap()
        .join()
        .unwrap();
});
