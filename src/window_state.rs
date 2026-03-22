use std::{
    fs, io,
    path::{Path, PathBuf},
};

use gpui::{Bounds, WindowBounds, point, px, size};

const WINDOW_STATE_REL_PATH: &str = ".kchess/window_bounds.txt";

fn window_state_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(Path::new(&home).join(WINDOW_STATE_REL_PATH))
}

fn parse_window_bounds(line: &str) -> Option<WindowBounds> {
    let mut parts = line.split_whitespace();
    let mode = parts.next()?;
    let x: f32 = parts.next()?.parse().ok()?;
    let y: f32 = parts.next()?.parse().ok()?;
    let w: f32 = parts.next()?.parse().ok()?;
    let h: f32 = parts.next()?.parse().ok()?;

    let bounds = Bounds::new(
        point(px(x), px(y)),
        size(px(w.max(640.0)), px(h.max(480.0))),
    );
    match mode {
        "windowed" => Some(WindowBounds::Windowed(bounds)),
        "maximized" => Some(WindowBounds::Maximized(bounds)),
        "fullscreen" => Some(WindowBounds::Fullscreen(bounds)),
        _ => None,
    }
}

pub fn load_window_bounds() -> Option<WindowBounds> {
    let path = window_state_path()?;
    let contents = fs::read_to_string(path).ok()?;
    parse_window_bounds(contents.trim())
}

pub fn save_window_bounds(bounds: WindowBounds) -> io::Result<()> {
    let Some(path) = window_state_path() else {
        return Ok(());
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let (mode, b) = match bounds {
        WindowBounds::Windowed(b) => ("windowed", b),
        WindowBounds::Maximized(b) => ("maximized", b),
        WindowBounds::Fullscreen(b) => ("fullscreen", b),
    };

    let line = format!(
        "{} {} {} {} {}\n",
        mode,
        f32::from(b.origin.x),
        f32::from(b.origin.y),
        f32::from(b.size.width),
        f32::from(b.size.height)
    );

    fs::write(path, line)
}
