#[cfg(feature = "gpui")]
fn main() {
    kchess_board::gpui_view::launch_demo();
}

#[cfg(not(feature = "gpui"))]
fn main() {
    eprintln!("Enable the `gpui` feature to run this example: cargo run --example gpui_demo --features gpui");
}
