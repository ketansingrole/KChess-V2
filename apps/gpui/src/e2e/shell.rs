//! The window and its sidebar.

use super::Harness;
use gpui_kit::test::TestQueryExt as _;

#[gpui_kit::test]
fn the_app_starts_on_home(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    for action in [
        "home-play",
        "home-puzzles",
        "home-analysis",
        "home-local",
        "home-online",
        "home-connect",
    ] {
        app.find(action);
    }
    app.click("account-menu");
    app.wait_for("the menu to offer connecting", |window, _| {
        window
            .try_find_by_label("Connect Lichess account")
            .is_some()
    });
}

/// The board on a first Home looks playable, so clicking it has to lead to a game.
#[gpui_kit::test]
fn the_board_on_a_first_home_starts_a_game(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    let (width, height) = super::SIZES[2];
    app.cx.simulate_window_resize(
        app.window,
        gpui_kit::size(gpui_kit::px(width), gpui_kit::px(height)),
    );
    let board = app.find("chess-board").bounds().center();
    app.click_point(board);
    let shown = app.cx.update(|cx| crate::shell::current_page(cx));
    assert_eq!(shown, Some(crate::pages::Page::Computer));
}

#[gpui_kit::test]
fn an_empty_home_offers_a_game(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.click("home-play");
    let shown = app.cx.update(|cx| crate::shell::current_page(cx));
    assert_eq!(shown, Some(crate::pages::Page::Computer));
}

#[gpui_kit::test]
fn every_sidebar_page_opens(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    for (page, label, _) in crate::shell::nav() {
        app.open(label);
        let shown = app.cx.update(|cx| crate::shell::current_page(cx));
        assert_eq!(shown, Some(page), "{label} opened another page");
    }
}

/// The keyboard goes back and forward through the pages visited, as a swipe does: ⌘[ and ⌘] on
/// macOS, Alt with an arrow elsewhere.
#[gpui_kit::test]
fn the_keyboard_goes_back_and_forward_through_the_pages(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::Page;
    let (back, forward) = if cfg!(target_os = "macos") {
        ("cmd-[", "cmd-]")
    } else {
        ("alt-left", "alt-right")
    };
    let mut app = Harness::start(cx);
    app.open("Puzzles");
    app.open("Studies");
    let shown = |app: &mut Harness| app.cx.update(|cx| crate::shell::current_page(cx));
    app.press(back);
    assert_eq!(shown(&mut app), Some(Page::Puzzles));
    app.press(back);
    assert_eq!(shown(&mut app), Some(Page::Dashboard));
    app.press(forward);
    assert_eq!(shown(&mut app), Some(Page::Puzzles));
    app.press(forward);
    assert_eq!(shown(&mut app), Some(Page::Studies));
    // Nothing further forward: the page stays.
    app.press(forward);
    assert_eq!(shown(&mut app), Some(Page::Studies));
}

/// The keyboard shortcuts open from their key and from Quick Pick, and Escape closes them.
#[gpui_kit::test]
fn the_keyboard_shortcuts_are_listed(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-/");
    app.find("shortcuts");
    app.press("escape");
    app.wait_gone("shortcuts");
    app.press("secondary-k");
    app.type_text("keyboard shortcuts");
    super::palette::wait_for_first(&mut app, "Keyboard shortcuts");
    app.press("enter");
    app.find("shortcuts");
}

/// ? lists the shortcuts too, as on Lichess, but a text field still takes it as text.
#[gpui_kit::test]
fn a_question_mark_lists_the_shortcuts_outside_a_text_field(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("?");
    app.find("shortcuts");
    app.press("escape");
    app.wait_gone("shortcuts");
    // In Quick Pick's field it is typed: ? lists the ways to search, and nothing else.
    app.press("secondary-k");
    app.press("?");
    app.wait_for("Quick Pick's help", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        let listed: Vec<String> = (0..8usize)
            .filter_map(|index| window.try_find(("palette-item", index)))
            .filter_map(|item| item.label().map(str::to_string))
            .collect();
        window.try_find("shortcuts").is_none()
            && listed
                == [
                    "Go to page",
                    "Run a command",
                    "Find a player",
                    "Change a setting",
                ]
    });
}

/// The sidebar marks your move in a Lichess game beside Play; collapsed to its rail it had no room
/// for the mark and dropped it, so a correspondence move went unseen there.
#[gpui_kit::test]
fn the_rail_still_marks_your_move(cx: &mut gpui_kit::TestAppContext) {
    // The game comes from Lichess, as a person's would. Seeded into `ongoing` by hand, it lasted
    // only until the next reload of the games in progress, which with no account connected clears
    // them: the startup check's reload, when it ended after the seed, left no mark to find. Waiting
    // for the check first still failed once in eight full runs (2026-11-06). Signed in, every
    // reload lists the game again.
    let mut app = Harness::start(cx);
    app.connect_with("Tester", false, |lichess| {
        lichess.correspondence_turn = true
    });
    app.with_window(crate::shell::toggle_sidebar);
    let mark = app.find("sidebar-game-mark").bounds();
    let play = app.find("4-0").bounds();
    assert!(
        play.contains(&mark.origin) && play.contains(&mark.bottom_right()),
        "the mark {mark:?} sits on Play on Lichess {play:?}"
    );
}

/// Windows gives a press on a drag area to the title bar, so a button left over one never hears
/// its click: the top bar's Restart to update did nothing there.
#[gpui_kit::test]
fn controls_over_the_title_bar_take_their_own_clicks(cx: &mut gpui_kit::TestAppContext) {
    use crate::updates::{Phase, UpdatesHandle};
    let mut app = Harness::start(cx);
    app.with_window(|_, cx| {
        let updates = cx.global::<UpdatesHandle>().0.clone();
        updates.update(cx, |updates, cx| {
            updates.phase = Phase::Downloaded;
            updates.version = Some("2026.10.20".into());
            cx.notify();
        });
    });
    let update = app.find("update-button").bounds();
    let toggle = app.find("sidebar-toggle").bounds();
    let search = app.with_window(|window, _| {
        let search = window.try_find_by_label("Search pages and commands");
        search.expect("the search box").bounds()
    });
    let (controls, beside) = app.with_window(|window, _| {
        let width = f32::from(window.viewport_size().width) as usize;
        let beside = (0..width).step_by(4).any(|x| {
            let at = gpui_kit::point(gpui_kit::px(x as f32), search.center().y);
            window.window_control_at(at) == Some(gpui_kit::WindowControlArea::Drag)
        });
        (
            [update, toggle, search].map(|bounds| window.window_control_at(bounds.center())),
            beside,
        )
    });
    assert_eq!(controls, [None; 3], "update, sidebar toggle and search");
    assert!(beside, "the bar still moves the window");
}

/// Windows moves the window and presses its buttons from the areas the last frame left, so a
/// frame that reuses the top bar has to keep them: opening a page left the bar unable to drag.
#[gpui_kit::test]
fn the_title_bar_works_after_a_frame_that_reuses_the_top_bar(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::WindowControlArea::{Close, Drag, Max, Min};
    let mut app = Harness::start(cx);
    let bar = app.with_window(|window, cx| {
        let search = window.try_find_by_label("Search pages and commands");
        let search = search.expect("the search box").bounds();
        window.refresh();
        super::draw(window, cx);
        let width = f32::from(window.viewport_size().width) as usize;
        (0..width)
            .step_by(4)
            .map(|x| gpui_kit::point(gpui_kit::px(x as f32), search.center().y))
            .find(|at| window.window_control_at(*at) == Some(Drag))
            .expect("the bar moves the window")
    });
    let controls = cfg!(target_os = "windows").then(|| {
        ["minimize", "maximize", "close"].map(|control| app.find(control).bounds().center())
    });
    app.open("Openings");
    let (at_bar, at_controls) = app.with_window(|window, _| {
        let at = controls.map(|controls| controls.map(|at| window.window_control_at(at)));
        (window.window_control_at(bar), at)
    });
    assert_eq!(at_bar, Some(Drag));
    if let Some(at_controls) = at_controls {
        assert_eq!(at_controls, [Some(Min), Some(Max), Some(Close)]);
    }
}

/// The window hides the system title bar, so on Windows the top bar draws its controls.
#[cfg(target_os = "windows")]
#[gpui_kit::test]
fn the_top_bar_has_the_window_controls(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    for control in ["minimize", "maximize", "close"] {
        app.find(control);
    }
}

#[gpui_kit::test]
fn the_sidebar_collapses_and_is_kept_collapsed(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    assert_eq!(app.find("sidebar-toggle").label(), Some("Hide sidebar"));
    app.click("sidebar-toggle");
    assert_eq!(app.find("sidebar-toggle").label(), Some("Show sidebar"));
    let mut app = app.restart();
    assert_eq!(app.find("sidebar-toggle").label(), Some("Show sidebar"));
}

/// The sidebar shows and hides from the keyboard too: ⌃⌘S on macOS, as its apps take it, and
/// Ctrl+B elsewhere.
#[gpui_kit::test]
fn the_keyboard_shows_and_hides_the_sidebar(cx: &mut gpui_kit::TestAppContext) {
    let key = if cfg!(target_os = "macos") {
        "ctrl-cmd-s"
    } else {
        "ctrl-b"
    };
    let mut app = Harness::start(cx);
    assert_eq!(app.find("sidebar-toggle").label(), Some("Hide sidebar"));
    app.press(key);
    assert_eq!(app.find("sidebar-toggle").label(), Some("Show sidebar"));
    app.press(key);
    assert_eq!(app.find("sidebar-toggle").label(), Some("Hide sidebar"));
}

/// The buttons that have a key say it in their tooltip (`tooltip_with_action`), which reads it
/// from the window's keymap as this does: the sidebar toggle's ⌃⌘S (Ctrl+B elsewhere), and New's
/// ⌘N on the game and study pages. (The harness does not register tooltips themselves.)
#[gpui_kit::test]
fn tooltips_show_the_keys(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::component::kbd::Kbd;
    let mut app = Harness::start(cx);
    let keys = app.with_window(|window, _| {
        let key = |action: &dyn gpui_kit::Action| {
            window
                .highest_precedence_binding_for_action(action)
                .and_then(|binding| {
                    binding
                        .keystrokes()
                        .first()
                        .map(|key| Kbd::format(key.inner()))
                })
        };
        (key(&crate::menu::ToggleSidebar), key(&crate::menu::New))
    });
    if cfg!(target_os = "macos") {
        assert_eq!(keys, (Some("⌃⌘S".into()), Some("⌘N".into())));
    } else {
        assert_eq!(keys, (Some("Ctrl+B".into()), Some("Ctrl+N".into())));
    }
}

/// A narrow window gives the page the room: the sidebar shows as its rail there, opens for the
/// moment from its toggle, and a wider window shows it as kept.
#[gpui_kit::test]
fn a_narrow_window_shows_the_rail(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::{px, size};
    let mut app = Harness::start(cx);
    let resize = |app: &mut Harness, width: f32| {
        app.cx
            .simulate_window_resize(app.window, size(px(width), px(900.)));
        for _ in 0..5 {
            app.tick();
        }
    };
    resize(&mut app, 760.);
    assert_eq!(app.find("sidebar-toggle").label(), Some("Show sidebar"));
    app.click("sidebar-toggle");
    assert_eq!(app.find("sidebar-toggle").label(), Some("Hide sidebar"));
    resize(&mut app, 1320.);
    assert_eq!(app.find("sidebar-toggle").label(), Some("Hide sidebar"));
    resize(&mut app, 760.);
    assert_eq!(app.find("sidebar-toggle").label(), Some("Show sidebar"));
    // Closing it on a wide window is kept; opening it on a narrow one was not.
    resize(&mut app, 1320.);
    app.click("sidebar-toggle");
    resize(&mut app, 760.);
    app.click("sidebar-toggle");
    resize(&mut app, 1320.);
    assert_eq!(app.find("sidebar-toggle").label(), Some("Show sidebar"));
}

/// Tab and Shift-Tab walk a page's controls in opposite orders, also after frames in which only
/// the shell redrew and the page's last frame, its tab stops included, was reused.
#[gpui_kit::test]
fn tab_and_shift_tab_walk_the_same_controls(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Openings");
    let step = |app: &mut Harness, forward: bool| {
        app.with_window(|window, cx| {
            if forward {
                window.focus_next(cx);
            } else {
                window.focus_prev(cx);
            }
        });
        app.with_window(|window, cx| window.focused(cx))
            .expect("a control has the keys")
    };
    let forward: Vec<_> = (0..6).map(|_| step(&mut app, true)).collect();
    for (ix, focus) in forward.iter().enumerate() {
        assert!(
            !forward[..ix].contains(focus),
            "Tab came back to a control after {ix} steps"
        );
    }
    app.cx.update(crate::shell::notify_shell);
    let back: Vec<_> = (0..5).map(|_| step(&mut app, false)).collect();
    let expected: Vec<_> = forward[..5].iter().rev().cloned().collect();
    assert_eq!(back, expected, "Shift-Tab retraces Tab");
    app.cx.update(crate::shell::notify_shell);
    let again: Vec<_> = (0..5).map(|_| step(&mut app, true)).collect();
    assert_eq!(again, forward[1..], "Tab after the shell redrew");
}

/// A connected account's section: its record, Sync games, and rating cards that choose the
/// chart's speed, the speed played most first.
#[gpui_kit::test]
fn a_rating_card_puts_its_speed_on_the_chart(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, _lichess) = Harness::signed_in_showcase(cx);
    app.find("home-perf-bullet");
    app.find("home-record");
    // Signing in syncs the account's games once, with Sync games disabled meanwhile.
    app.wait_for("Sync games", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find(gpui_kit::ElementId::from("home-sync"))
            .is_some_and(|sync| sync.disabled() != Some(true))
    });
    assert_eq!(app.find("home-perf-blitz").selected(), Some(true));
    app.click("home-perf-puzzle");
    app.wait_for("Puzzles to be chosen", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find(gpui_kit::ElementId::from("home-perf-puzzle"))
            .is_some_and(|card| card.selected() == Some(true))
    });
    assert_eq!(app.find("home-perf-blitz").selected(), Some(false));
}

/// The account menu sat under the sidebar's own Settings button and offered Settings again, above
/// a header that repeated the button's name. It now goes to the account's profile and to the
/// accounts section of Settings.
#[gpui_kit::test]
fn the_account_menu_opens_the_profile_and_the_accounts_settings(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, _lichess) = Harness::signed_in_showcase(cx);

    app.click("account-menu");
    app.click_label("Manage accounts");
    app.wait_for("Settings to open at its accounts", |_, cx| {
        crate::shell::current_page(cx) == Some(crate::pages::Page::Settings)
            && crate::shell::view_of::<crate::pages::settings::SettingsPage>(
                crate::pages::Page::Settings,
                cx,
            )
            .is_some_and(|page| {
                page.read(cx).section() == crate::pages::settings::Section::Accounts
            })
    });

    app.click("account-menu");
    app.click_label("Profile");
    app.wait_for("the account's own profile", |_, cx| {
        crate::shell::current_page(cx) == Some(crate::pages::Page::Players)
            && crate::shell::view_of::<crate::pages::players::PlayersPage>(
                crate::pages::Page::Players,
                cx,
            )
            .and_then(|page| page.read(cx).username())
            .is_some_and(|name| name.eq_ignore_ascii_case("Showcase"))
    });
}

/// The window's title names the page, so Mission Control and the Window menu tell it apart.
#[gpui_kit::test]
fn the_window_title_names_the_page(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    let title = |app: &mut Harness| app.with_window(|_, cx| crate::shell::window_title_shown(cx));
    assert_eq!(title(&mut app).as_deref(), Some("KChess"));
    app.open("Puzzles");
    assert_eq!(title(&mut app).as_deref(), Some("Puzzles — KChess"));
    app.open("Analysis board");
    assert_eq!(title(&mut app).as_deref(), Some("Analysis board — KChess"));
}

/// Quitting during a real-time Lichess game asks first: its clock keeps running on Lichess.
#[gpui_kit::test]
fn quitting_during_a_live_game_asks_first(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.wait_for("the startup check to finish", |_, cx| {
        crate::online::handle(cx).read(cx).phase != crate::online::Phase::Checking
    });
    // The phase is set where the key is pressed: a later snapshot of the core's replaces it.
    app.with_window(|window, cx| {
        use gpui_kit::test::TestWindowExt as _;
        crate::online::handle(cx).update(cx, |online, cx| online.set_phase("playing", cx));
        assert!(crate::online::handle(cx).read(cx).live_game_on(), "on");
        window.press("secondary-q", cx);
    });
    app.cx.run_until_parked();
    assert!(app.cx.has_pending_prompt(), "asked before quitting");
    app.cx.simulate_prompt_answer("Cancel");
    app.cx.run_until_parked();
    assert!(!app.cx.has_pending_prompt());
    // Still open.
    app.open("Home");
    app.find("home-play");
}

/// The sidebar's shortcut, menu item and command opens and closes it at once, as a command used
/// many times a day should; a click on its toggle eases it, so the eye can follow where it went.
/// The ease runs on the test's clock, so it holds still until the test moves the clock.
#[gpui_kit::test]
fn only_a_click_eases_the_sidebar(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.cx.update(|cx| cx.set_reduce_motion(false));
    let width = |app: &mut Harness| f32::from(app.find("sidebar").bounds().size.width);
    let open = width(&mut app);

    app.with_window(crate::shell::toggle_sidebar);
    let rail = width(&mut app);
    assert!(
        rail < open,
        "the shortcut closes it at once: {rail} of {open}"
    );
    app.with_window(crate::shell::toggle_sidebar);
    assert_eq!(width(&mut app), open, "and opens it at once");

    app.click("sidebar-toggle");
    assert_eq!(width(&mut app), open, "a click eases it from where it was");
    app.cx
        .executor()
        .advance_clock(std::time::Duration::from_secs(1));
    assert_eq!(width(&mut app), rail);
}

/// On Windows and Linux a system notification is KChess's own: sent under its name, a game's with
/// Go to board, and a click on it brings KChess forward on that game's page. On Windows the
/// notification library sent them as PowerShell, and a click went nowhere.
#[cfg(any(windows, target_os = "linux"))]
#[gpui_kit::test]
fn a_clicked_notification_opens_its_game(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    assert_eq!(
        app.cx.app_identity(),
        Some((
            crate::notifications::APP_ID.into(),
            gpui_kit::SharedString::from("KChess")
        ))
    );
    app.with_window(|_, cx| {
        crate::notifications::post(
            "computerMove",
            "Stockfish moved",
            "e5. Your move.",
            false,
            cx,
        )
    });
    let shown = app.cx.shown_system_notifications();
    let last = shown.last().expect("a notification");
    assert_eq!(
        (last.title.as_ref(), last.body.as_ref()),
        ("Stockfish moved", "e5. Your move.")
    );
    assert_eq!(
        last.actions
            .iter()
            .map(|action| action.label.as_ref())
            .collect::<Vec<_>>(),
        ["Go to board"]
    );
    app.cx
        .simulate_system_notification_response(gpui_kit::SystemNotificationResponse {
            tag: last.tag.clone(),
            action_id: None,
        });
    app.cx.run_until_parked();
    let shown = app.cx.update(|cx| crate::shell::current_page(cx));
    assert_eq!(shown, Some(crate::pages::Page::Computer));
    // A notification about no game has nothing to go to.
    app.with_window(|_, cx| crate::notifications::post("test", "KChess", "Hello", false, cx));
    let shown = app.cx.shown_system_notifications();
    assert!(shown.last().expect("a notification").actions.is_empty());
}
