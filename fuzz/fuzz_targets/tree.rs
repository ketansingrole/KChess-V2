//! A PGN's analysis tree, read back on a 1 MiB thread as Windows' main thread is: building it on
//! the rules' large stack and then reading or dropping it on a small one aborted the app.
#![no_main]

use kchess_domain::api;
use libfuzzer_sys::fuzz_target;

#[derive(serde::Deserialize)]
struct Node {
    #[allow(dead_code)]
    children: Vec<Node>,
}

fuzz_target!(|pgn: &str| {
    let args = serde_json::json!([pgn]).to_string();
    let Ok(text) = api::call("treeFromPgn", &args) else {
        return;
    };
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            let tree = api::on_large_stack(|| {
                let mut reader = serde_json::Deserializer::from_str(&text);
                reader.disable_recursion_limit();
                serde::Deserialize::deserialize(&mut reader).map_err(|e| e.to_string())
            });
            let _: Option<Node> = tree.ok();
        })
        .unwrap()
        .join()
        .unwrap();
});
