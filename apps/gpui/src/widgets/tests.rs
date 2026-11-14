use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    App, AppContext as _, Context, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, TestAppContext, Window, div,
};

use crate::theme::{Palette, colors};

#[test]
fn a_count_of_one_takes_the_singular() {
    use super::format_counted;
    assert_eq!(format_counted(1., "request", "requests"), "1 request");
    assert_eq!(format_counted(0., "game", "games"), "0 games");
    assert_eq!(
        format_counted(2048., "request", "requests"),
        "2,048 requests"
    );
}

/// The large ticking times (the chess clock's faces, Puzzle Rush, timed practice runs) used
/// proportional figures, so their digits shifted sideways on every tick; a `timer` has
/// tabular ones (xtask's `boundaries` checks those clocks draw through it).
#[test]
fn large_ticking_times_hold_still() {
    use gpui_kit::Styled as _;
    let mut timer = super::timer("0:09.4", gpui_kit::px(32.));
    assert_eq!(
        timer.style().text.font_features,
        Some(super::tabular()),
        "tabular figures"
    );
}

/// A move list in a window whose page holds the keys, as a game page's does.
struct Page {
    focus: FocusHandle,
    chosen: Rc<Cell<Option<usize>>>,
}

impl Render for Page {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::of(&colors("kchess", &[], false), false);
        let chosen = self.chosen.clone();
        div()
            .size_full()
            .track_focus(&self.focus)
            .child(super::moves_list(
                "moves",
                super::MoveSheet {
                    sans: &["e4".into(), "e5".into()],
                    shown: 2,
                    first: super::FirstMove::default(),
                },
                "No moves".into(),
                move |ply, _, _| chosen.set(Some(ply)),
                None,
                &palette,
            ))
    }
}

/// GPUI announces nothing by itself: a move or a result was heard only when the reader went
/// back to the board. Each adapter speaks a live node's text when it changes: macOS its value,
/// Windows and Linux its name.
#[test]
fn a_live_region_is_spoken_on_every_platform() {
    use gpui_kit::{Element as _, accesskit};
    let region = super::live_region("live", "Last move e2–e4 · Black to move");
    assert_eq!(region.a11y_role(), Some(gpui_kit::Role::Status));
    let mut node = accesskit::Node::new(gpui_kit::Role::Status);
    region.write_a11y_info(&mut node);
    assert_eq!(node.live(), Some(accesskit::Live::Polite));
    assert_eq!(node.value(), Some("Last move e2–e4 · Black to move"));
    assert_eq!(node.label(), Some("Last move e2–e4 · Black to move"));
}

/// The moves were mouse-only: Tab skipped them, so a keyboard could not choose one.
#[gpui_kit::test]
fn moves_take_the_keyboard_and_leave_a_click_to_the_page(cx: &mut TestAppContext) {
    let chosen = Rc::new(Cell::new(None));
    let window = cx.add_window(|_, cx| Page {
        focus: cx.focus_handle(),
        chosen: chosen.clone(),
    });
    let page_focus = window.update(cx, |page, _, _| page.focus.clone()).unwrap();
    let mut step = |f: &mut dyn FnMut(&mut Window, &mut App)| {
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            f(window, cx);
            window.render_frame(cx);
        })
        .unwrap();
    };

    // A click chooses the move and leaves the keys with the page.
    step(&mut |window, cx| window.focus(&page_focus, cx));
    step(&mut |window, cx| window.click(("move", 1usize), cx));
    assert_eq!(chosen.take(), Some(2), "the ply after the second move");
    step(&mut |window, _| assert!(page_focus.is_focused(window)));

    // Tab reaches the first move, and Enter chooses it.
    step(&mut |window, cx| window.focus_next(cx));
    step(&mut |window, _| assert!(!page_focus.is_focused(window)));
    step(&mut |window, cx| window.press("enter", cx));
    assert_eq!(chosen.take(), Some(1));
    // Space chooses the next one.
    step(&mut |window, cx| window.focus_next(cx));
    step(&mut |window, cx| window.press("space", cx));
    assert_eq!(chosen.take(), Some(2));
}

/// Clickables drew their hover background and nothing more while held down, so a press
/// looked like a hover. A held move draws the pressed fill at once, over its keyboard focus
/// handler's `prevent_default`, and lets it go with the button.
#[gpui_kit::test]
fn a_held_move_draws_its_pressed_background(cx: &mut TestAppContext) {
    use gpui_kit::{Background, InputEvent as _, Pixels, Point};
    use gpui_kit::{Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent};

    let window = cx.add_window(|_, cx| Page {
        focus: cx.focus_handle(),
        chosen: Rc::new(Cell::new(None)),
    });
    let palette = Palette::of(&colors("kchess", &[], false), false);
    let (hover, pressed) = (palette.accented, palette.pressed(palette.accented));
    assert_ne!(hover, pressed);
    // The fills painted under `at`.
    let fills = |window: &Window, at: Point<Pixels>| -> Vec<Background> {
        let at = at.scale(window.scale_factor());
        window
            .painted_quads()
            .into_iter()
            .filter(|quad| quad.bounds.contains(&at))
            .map(|quad| quad.background)
            .collect()
    };
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        // The first move: not the current one, which has its own fill.
        let at = window
            .try_find(("move", 0usize))
            .expect("the move")
            .bounds()
            .center();
        let dispatch = |window: &mut Window, cx: &mut App, event: gpui_kit::PlatformInput| {
            window.dispatch_event(event, cx);
            window.render_frame(cx);
        };
        let mouse_move = MouseMoveEvent {
            position: at,
            pressed_button: None,
            modifiers: Modifiers::default(),
        };
        dispatch(window, cx, mouse_move.to_platform_input());
        let painted = fills(window, at);
        assert!(painted.contains(&hover.into()), "hovered: {painted:?}");
        assert!(!painted.contains(&pressed.into()));

        let down = MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        };
        dispatch(window, cx, down.to_platform_input());
        let painted = fills(window, at);
        assert!(painted.contains(&pressed.into()), "pressed: {painted:?}");

        let up = MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count: 1,
        };
        dispatch(window, cx, up.to_platform_input());
        let painted = fills(window, at);
        assert!(!painted.contains(&pressed.into()), "released: {painted:?}");
        assert!(painted.contains(&hover.into()));
    })
    .unwrap();
}

/// A clock turns red only in its last ten seconds, when it shows tenths.
#[test]
fn only_the_last_ten_seconds_are_low() {
    assert!(super::low_time("0:09.4"));
    assert!(super::low_time("0:00.0"));
    assert!(!super::low_time("0:10"));
    assert!(!super::low_time("1:05"));
    assert!(!super::low_time("1:00:05"));
    assert!(!super::low_time("2d 4h"));
}
