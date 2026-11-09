//! Memory the app keeps as it runs for hours. GPUI keeps every picture it has decoded
//! (`img(Arc<Image>)`) and every texture it has drawn until the app releases them, so a picture
//! made afresh while the app runs must release the one it replaces.
//!
//! The benchmark below is not part of `check`: it prints how much the process grows while a
//! turned clock face counts down, on Linux, where `/proc` reports it.
//! `cargo test -p kchess-gpui e2e::memory -- --ignored --nocapture`

use super::Harness;

fn clock(cx: &mut gpui_kit::TestAppContext) -> Harness {
    let mut app = Harness::start(cx);
    app.open("Over the board");
    app.find("chess-board");
    app.click(("local-mode", 1usize));
    // Turned for the players across the table: each face is drawn as a picture.
    app.click(("clock-rotate", 1usize));
    app.click(("clock-rotate", 2usize));
    app
}

/// A press of each face in turn: each press counts a move, so a face shows something new.
fn press(app: &mut Harness, n: usize) {
    app.click(("clock-face", 1 + n % 2));
}

/// The pictures of the turned faces as the page draws them now.
fn faces(app: &mut Harness) -> Vec<std::sync::Arc<gpui_kit::RenderImage>> {
    use crate::pages::Page;
    use crate::pages::local::LocalPage;
    app.with_window(|_, cx| {
        crate::shell::view_of::<LocalPage>(Page::Local, cx)
            .expect("the page")
            .read(cx)
            .turned_faces()
    })
}

/// Regression: each new face of a turned clock (every second while it runs, every tenth in the
/// last ten seconds) was a PNG that GPUI decoded and kept for the session, with its texture: some
/// 3.5 MiB a face, 689 MiB for 200 faces in `turned_clock_faces_counting_down`.
#[gpui_kit::test]
fn a_turned_clock_face_releases_the_picture_it_replaces(cx: &mut gpui_kit::TestAppContext) {
    let mut app = clock(cx);
    let first = faces(&mut app);
    assert_eq!(first.len(), 2, "both faces are pictures");
    for n in 0..6 {
        press(&mut app, n);
    }
    let now = faces(&mut app);
    app.with_window(|window, _| {
        for face in &first {
            assert!(
                !window.has_image_atlas_entry(face),
                "a replaced face is released"
            );
        }
        for face in &now {
            assert!(
                window.has_image_atlas_entry(face),
                "the faces shown are drawn"
            );
        }
    });
    // Turned back upright (90° at a time), a face is text again and lets its picture go.
    for _ in 0..3 {
        app.click(("clock-rotate", 1usize));
    }
    let upright = faces(&mut app);
    assert_eq!(upright.len(), 1, "only Player 2's face is still a picture");
    app.with_window(|window, _| {
        assert!(
            !window.has_image_atlas_entry(&now[0]),
            "Player 1's picture is released"
        );
    });
}

/// Regression: piece images pushed out of the board's cache kept their textures, so each size
/// a window passed through while being resized stayed in the GPU atlas, twelve pieces at a time.
#[gpui_kit::test]
fn piece_sizes_a_resize_left_behind_are_released(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::{px, size};
    let mut app = Harness::start(cx);
    app.open("Over the board");
    let pixels = |app: &mut Harness| {
        let side = app.find("chess-board").bounds().size.width.as_f32() / 8.;
        let scale = app.with_window(|window, _| window.scale_factor());
        (side * scale).round() as u32
    };
    let first_size = pixels(&mut app);
    let first = app.with_window(|_, cx| crate::board::looks::kept_images(first_size, cx));
    assert_eq!(first.len(), 12, "the set at the board's first size");
    assert!(
        app.with_window(|window, _| first
            .iter()
            .any(|image| window.has_image_atlas_entry(image))),
        "the board drew them"
    );
    // Ten more board sizes push the first out of the cache (it keeps eight sizes of a set).
    for step in 1..=10 {
        let width = 1300. - 40. * step as f32;
        app.cx
            .simulate_window_resize(app.window, size(px(width), px(900.)));
        for _ in 0..4 {
            app.tick();
        }
    }
    let last_size = pixels(&mut app);
    let now = app.with_window(|_, cx| crate::board::looks::kept_images(last_size, cx));
    app.with_window(|window, _| {
        for image in &first {
            assert!(
                !window.has_image_atlas_entry(image),
                "a size left behind is released"
            );
        }
        assert!(
            now.iter().any(|image| window.has_image_atlas_entry(image)),
            "the size shown is drawn"
        );
    });
}

/// The process's resident memory, in MiB, where the system says.
fn resident_mib() -> Option<f64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: f64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096. / (1024. * 1024.))
}

#[gpui_kit::test]
#[ignore = "a benchmark: prints memory growth; run with --ignored --nocapture"]
fn turned_clock_faces_counting_down(cx: &mut gpui_kit::TestAppContext) {
    let mut app = clock(cx);
    for n in 0..20 {
        press(&mut app, n);
    }
    let Some(base) = resident_mib() else {
        println!("memory turned clock: no /proc on this system");
        return;
    };
    let mut trail = vec![0.];
    let (batches, per_batch) = (4, 50);
    for batch in 0..batches {
        for n in 0..per_batch {
            press(&mut app, batch * per_batch + n);
        }
        trail.push(resident_mib().unwrap_or(base) - base);
    }
    let presses = (batches * per_batch) as f64;
    let grown = trail.last().copied().unwrap_or_default();
    println!(
        "memory turned clock: {presses} new faces grew the process {grown:.1} MiB \
         ({:.0} KiB per face); after each {per_batch}: {}",
        grown * 1024. / presses,
        trail
            .iter()
            .map(|mib| format!("{mib:.1} MiB"))
            .collect::<Vec<_>>()
            .join(" → ")
    );
}
