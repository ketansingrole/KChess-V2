//! Settings: changes are saved to the profile, shown at once and kept across launches.

use super::Harness;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use serde_json::json;

#[gpui_kit::test]
fn a_switch_is_saved_and_disables_what_depends_on_it(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Sound");
    assert_eq!(app.find("sound").checked(), Some(true));
    app.click("sound");
    assert_eq!(app.find("sound").checked(), Some(false));
    app.wait_for_saved("soundEnabled", json!(false));
}

#[gpui_kit::test]
fn a_chosen_board_is_saved_and_marked(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Appearance");
    let other = crate::board::looks::boards()
        .iter()
        .map(|board| board.id.clone())
        .find(|id| *id != app.saved_settings()["boardTheme"])
        .expect("another board");
    let option = format!("board-{other}");
    // The dialog's settings scroll; the board may sit below its fold.
    let dialog = app.find(super::SETTINGS_SHOWN).bounds();
    for _ in 0..20 {
        let shown = app.with_window(|window, _| {
            gpui_kit::test::TestWindowExt::try_find(
                window,
                gpui_kit::ElementId::from(option.clone()),
            )
            .is_some_and(|e| e.visible() && e.bounds().bottom() <= dialog.bottom())
        });
        if shown {
            break;
        }
        app.with_window(|window, cx| {
            use gpui_kit::test::TestWindowExt as _;
            let by = gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-200.));
            window.scroll("settings-page", gpui_kit::ScrollDelta::Pixels(by), cx);
        });
    }
    assert_eq!(app.find(option.clone()).selected(), Some(false));
    app.click(option.clone());
    assert_eq!(app.find(option).selected(), Some(true));
    app.wait_for_saved("boardTheme", json!(other));
}

#[gpui_kit::test]
fn the_theme_choice_is_saved(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Appearance");
    let dark = ("appearance", 2usize);
    assert_eq!(app.find(dark).label(), Some("Dark"));
    app.click(dark);
    assert_eq!(app.find(dark).selected(), Some(true));
    app.wait_for_saved("appearance", json!("dark"));
}

#[gpui_kit::test]
fn settings_are_kept_when_the_app_restarts(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Sound");
    app.click("sound");
    assert_eq!(app.find("sound").checked(), Some(false));
    // Quit before the 400 ms save: quitting flushes it.
    let mut app = app.restart();
    app.open_settings("Sound");
    assert_eq!(app.find("sound").checked(), Some(false));
}

/// Settings opens as a dialog over the page it was opened from, which stays in sight under it,
/// and closing it returns there.
#[gpui_kit::test]
fn settings_opens_over_the_page_and_closes_back_to_it(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Play with computer");
    app.find("chess-board");
    app.click("settings");
    app.find(super::SETTINGS_SHOWN);
    assert!(app.shows("chess-board"));
    app.click("settings-close");
    app.wait_gone(super::SETTINGS_SHOWN);
    assert_eq!(
        app.cx.update(|cx| crate::shell::current_page(cx)),
        Some(crate::pages::Page::Computer)
    );
}

/// Escape, or a press outside the dialog, closes Settings as its close button does.
#[gpui_kit::test]
fn escape_or_a_press_outside_closes_settings(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Play with computer");
    app.click("settings");
    app.find(super::SETTINGS_SHOWN);
    app.press("escape");
    app.wait_gone(super::SETTINGS_SHOWN);
    app.click("settings");
    let dialog = app.find(super::SETTINGS_SHOWN).bounds();
    // A press inside it keeps it open.
    app.click_point(dialog.center());
    assert!(app.shows(super::SETTINGS_SHOWN));
    let outside = gpui_kit::point(dialog.left() - gpui_kit::px(8.), dialog.center().y);
    app.click_point(outside);
    app.wait_gone(super::SETTINGS_SHOWN);
    app.find("chess-board");
}

#[gpui_kit::test]
fn the_settings_shortcut_opens_settings(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-,");
    app.find(super::SETTINGS_SHOWN);
}

#[gpui_kit::test]
fn ai_settings_offer_a_chatgpt_plan(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("AI");
    assert_eq!(
        app.find("ai-connect").label(),
        Some("Continue with ChatGPT")
    );
}

#[gpui_kit::test]
fn a_category_opened_in_a_short_window_scrolls_into_its_list(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start_with(cx, |profile| {
        let state = json!({ "x": 0, "y": 0, "width": 1024, "height": 560 });
        std::fs::write(profile.join("window-state.json"), state.to_string())
            .expect("the window state");
    });
    app.with_window(|window, cx| {
        crate::shell::open_settings(crate::pages::settings::Section::Updates, window, cx)
    });
    // Its group is named for its heading.
    let group = app.with_window(|window, _| {
        window
            .try_find_by_label("System")
            .and_then(|group| group.role())
    });
    assert_eq!(group, Some(gpui_kit::Role::Group));
    // Updates, the last category, is scrolled into the dialog's list.
    let updates = app.find("s5-1").bounds();
    let list = app.find("settings-sections").bounds();
    assert!(
        updates.bottom() <= list.bottom(),
        "{updates:?} is under {list:?}"
    );
    // Closed and opened again on it, Settings shows it again.
    app.click("settings-close");
    assert!(!app.shows(super::SETTINGS_SHOWN));
    app.click("settings");
    let updates = app.find("s5-1").bounds();
    assert!(updates.bottom() <= list.bottom());
}

/// The window's place and the interface size were files each app wrote itself under one
/// temporary name: two apps on a profile saving at once lost a save ("No such file or
/// directory"). The core keeps both now, and takes an earlier release's files once.
#[gpui_kit::test]
fn an_earlier_releases_window_and_interface_size_move_into_the_core(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::interface_zoom::{Setting, setting};
    let mut app = Harness::start_with(cx, |profile| {
        let state = json!({ "x": 0, "y": 0, "width": 1100, "height": 700 });
        std::fs::write(profile.join("window-state.json"), state.to_string())
            .expect("the window state");
        std::fs::write(profile.join("interface-zoom.json"), r#"{"zoom":125}"#)
            .expect("the interface size");
    });
    assert_eq!(app.cx.update(|cx| setting(cx)), Setting::Percent(125));
    let size = app.with_window(|window, _| window.bounds().size);
    assert_eq!((size.width.as_f32(), size.height.as_f32()), (1100., 700.));
    app.wait_for_view_preference("kchess:interface-zoom", json!(125));
    app.wait_for_view_preference(
        crate::window_state::KEY,
        json!({ "x": 0, "y": 0, "width": 1100, "height": 700,
                "maximized": false, "fullScreen": false }),
    );
    let profile = app
        .profile
        .as_ref()
        .expect("the profile")
        .path()
        .to_path_buf();
    assert!(!profile.join("window-state.json").exists());
    assert!(!profile.join("interface-zoom.json").exists());
}

/// A resized window is saved through the core, and opens at that size again.
#[gpui_kit::test]
fn the_window_opens_as_it_was_left(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::{px, size};
    let mut app = Harness::start(cx);
    app.cx
        .simulate_window_resize(app.window, size(px(1000.), px(680.)));
    let started = std::time::Instant::now();
    loop {
        app.cx.run_until_parked();
        let saved = app.request(kchess_contracts::calls::ViewPreferences {});
        let kept = saved.get(crate::window_state::KEY);
        if kept.is_some_and(|kept| kept["width"] == json!(1000) && kept["height"] == json!(680)) {
            break;
        }
        assert!(
            started.elapsed() < super::TIMEOUT,
            "the size stayed {kept:?}"
        );
        app.tick();
    }
    let profile = app
        .profile
        .as_ref()
        .expect("the profile")
        .path()
        .to_path_buf();
    assert!(!profile.join("window-state.json").exists());
    let mut app = app.restart();
    let opened = app.with_window(|window, _| window.bounds().size);
    assert_eq!(
        (opened.width.as_f32(), opened.height.as_f32()),
        (1000., 680.)
    );
}

#[gpui_kit::test]
fn moves_can_be_shown_with_piece_symbols(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Appearance");
    let symbols = ("notation", 1usize);
    assert_eq!(app.find(("notation", 0usize)).selected(), Some(true));
    app.click(symbols);
    assert_eq!(app.find(symbols).selected(), Some(true));
    app.wait_for_saved("figurineNotation", json!(true));
    assert!(app.cx.update(|cx| crate::notation::figurine(cx)).is_some());
    // Only what is shown changes: a screen reader still hears the move in letters.
    app.click("settings-close");
    app.open("Analysis board");
    app.find("chess-board");
    app.click_square("g1", true);
    app.click_square("f3", true);
    assert_eq!(app.find("tree-g1f3").label(), Some("Nf3"));
}

/// ⌘K finds a category by any row title it shows, so a new row is found as soon as it is added,
/// and can open the category at it.
#[gpui_kit::test]
fn every_settings_row_is_found_from_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::settings::{Section, section_keywords, section_rows};
    use gpui_kit::test::TestQueryExt as _;
    let mut app = Harness::start(cx);
    let mut checked = 0;
    for (section, label) in Section::ALL.map(|s| (s, s.label())) {
        app.open_settings(label);
        let titles: Vec<String> = app.with_window(|window, _| {
            window
                .elements()
                .iter()
                .filter_map(|element| match element.path().last()? {
                    gpui_kit::ElementId::NamedChild(parent, title)
                        if **parent == gpui_kit::ElementId::Name("setting-title".into()) =>
                    {
                        Some(title.to_string())
                    }
                    _ => None,
                })
                .collect()
        });
        let keywords = section_keywords(section).to_lowercase();
        let rows = section_rows(section);
        for title in titles {
            assert!(
                rows.contains(&title.as_str()),
                "{label}: {title:?} is missing from section_rows"
            );
            assert!(keywords.contains(&title.to_lowercase()));
            checked += 1;
        }
    }
    assert!(checked > 40, "only {checked} row titles were drawn");
}

/// A row title is its own result, named as the row is, and opens its category there.
#[gpui_kit::test]
fn a_row_title_finds_its_category_in_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.type_text("#piece notation");
    super::palette::wait_for_first(&mut app, "Piece notation");
    app.press("enter");
    app.find(("notation", 1usize));
}

/// The row ⌘K lit on the Settings page, if any.
fn revealed(app: &mut Harness) -> Option<&'static str> {
    app.cx.update(|cx| {
        crate::shell::view::<crate::pages::settings::SettingsPage>(cx)
            .and_then(|page| page.read(cx).revealed())
    })
}

/// A row at the foot of a long category is scrolled into view and lit, then the light goes.
#[gpui_kit::test]
fn a_row_found_in_quick_pick_is_scrolled_to_and_lit(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.type_text("#board theme");
    super::palette::wait_for_first(&mut app, "Board theme");
    app.press("enter");
    assert_eq!(revealed(&mut app), Some("Board theme"));
    app.wait_for("the row in view", |window, _| {
        let (Some(page), Some(row)) = (
            window.try_find(gpui_kit::ElementId::from("settings-page")),
            window.try_find(gpui_kit::ElementId::from("revealed-row")),
        ) else {
            return false;
        };
        let (page, row) = (page.bounds(), row.bounds());
        // Board theme is the last row: the page scrolls to its end, which shows all of it.
        row.top() > page.top() && row.bottom() <= page.bottom()
    });
    app.wait_for("the light to go", |window, _| {
        window
            .try_find(gpui_kit::ElementId::from("revealed-row"))
            .is_none()
    });
    assert_eq!(revealed(&mut app), None);
}

/// A category's own name finds the category, not one of its rows.
#[gpui_kit::test]
fn a_category_name_finds_the_category(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.type_text("#gestures");
    super::palette::wait_for_first(&mut app, "Gestures");
    app.press("enter");
    assert_eq!(revealed(&mut app), None);
}

/// A toggle ⌘K switches itself stays that toggle, rather than becoming a row to open.
#[gpui_kit::test]
fn a_quick_toggle_is_not_a_row_result(cx: &mut gpui_kit::TestAppContext) {
    let toggle = crate::pages::settings::TOGGLES
        .iter()
        .find(|toggle| toggle.title == "Show possible moves")
        .expect("the toggle");
    assert!(toggle.quick.is_some());
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.type_text("#show possible moves");
    super::palette::wait_for_first(&mut app, toggle.label());
    app.with_window(|window, _| {
        let rows: Vec<_> = (0..64usize)
            .map_while(|index| window.try_find(("palette-item", index)))
            .filter_map(|item| item.label().map(str::to_string))
            .collect();
        // The toggle alone: no second result opens Gameplay at its row.
        let named = rows.iter().filter(|label| *label == toggle.label()).count();
        assert_eq!(named, 1, "{rows:?}");
    });
}

/// Settings → Updates links to the website, the source and the issue tracker.
#[gpui_kit::test]
fn updates_links_to_the_website_source_and_issues(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Updates");
    for (id, label) in [
        ("about-website", "Website"),
        ("about-source", "Source code"),
        ("about-issues", "Report a problem"),
    ] {
        let link = app.find(id);
        assert_eq!(link.label(), Some(label));
        assert_eq!(link.role(), Some(gpui_kit::Role::Link));
    }
}

/// Settings → Updates with a newer release: at every window size the badge beside the version
/// stays clear of the buttons and inside the dialog (the buttons once covered "Update available").
#[gpui_kit::test]
fn the_update_badge_is_never_under_the_buttons(cx: &mut gpui_kit::TestAppContext) {
    use crate::updates::{Phase, UpdatesHandle};
    use gpui_kit::{ElementId, px, size};
    let mut app = Harness::start(cx);
    app.with_window(|_, cx| {
        let updates = cx.global::<UpdatesHandle>().0.clone();
        updates.update(cx, |updates, cx| {
            updates.version = Some("2026.11.9".into());
            updates.released = Some("2026-11-22".into());
            updates.error = None;
            updates.phase = Phase::Available;
            cx.notify();
        });
    });
    app.open_settings("Updates");
    for (width, height) in super::SIZES {
        app.cx
            .simulate_window_resize(app.window, size(px(width), px(height)));
        app.settle();
        let badge = app
            .with_window(|window, _| window.try_find(ElementId::from("update-badge")))
            .expect("the badge");
        assert_eq!(badge.label(), Some("Update available"));
        assert!(
            badge.visible(),
            "at {width}×{height} the badge {:?} is clipped",
            badge.bounds()
        );
        let badge = badge.bounds();
        let dialog = app.find(super::SETTINGS_SHOWN).bounds();
        assert!(
            badge.left() >= dialog.left() && badge.right() <= dialog.right(),
            "at {width}×{height} the badge {badge:?} leaves the dialog {dialog:?}"
        );
        for button in ["update-download", "update-check"] {
            let bounds = app.with_window(|window, _| {
                window
                    .try_find(ElementId::from(button))
                    .map(|found| found.bounds())
            });
            if let Some(bounds) = bounds {
                assert!(
                    !badge.intersects(&bounds),
                    "at {width}×{height} the badge {badge:?} is under {button} {bounds:?}"
                );
            }
        }
    }
}

#[gpui_kit::test]
fn the_core_downloads_the_voice_model_from_storage(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.online();
    app.open_settings("Storage");
    // The core reports no model in the profile, so its row offers to download one.
    assert!(app.enabled("storage-voice-download"));
    app.click("storage-voice-download");
    // The tests' model cannot be had: the core's error is shown, and the download offered again.
    app.find("storage-voice-error");
    assert!(app.enabled("storage-voice-download"));
}

/// Storage lists every download in one place, with the disk's free space; one that would leave
/// the disk almost full asks first, and Cancel leaves it undownloaded.
#[gpui_kit::test]
fn storage_lists_every_download_and_asks_before_filling_the_disk(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut app = Harness::start(cx);
    app.online();
    app.open_settings("Storage");
    for row in ["storage-databases", "storage-puzzles", "storage-voice"] {
        app.find(row);
    }
    app.wait_for("the free space", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find(gpui_kit::ElementId::from("storage-free"))
            .is_some()
    });
    let free = app
        .find("storage-free")
        .label()
        .unwrap_or_default()
        .to_string();
    assert!(free.ends_with(" free on this disk"), "{free}");
    // The profile's own files are counted, whatever else is: nothing KChess keeps is left out.
    app.wait_for("the profile's files", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        (0..6usize).any(|index| {
            window
                .try_find(gpui_kit::ElementId::from(("storage-part", index)))
                .and_then(|part| part.label().map(str::to_string))
                .is_some_and(|label| label.starts_with("Logs and other files "))
        })
    });
    // No game database yet: the row leads to the Database page.
    app.click("storage-databases-open");
    app.wait_for_page(crate::pages::Page::Database);

    app.open_settings("Storage");
    app.cx
        .update(|cx| cx.set_global(crate::room::PinnedFreeSpace(1024 * 1024 * 1024)));
    app.click("db-install");
    app.wait_for("the warning", |window, _| {
        use gpui_kit::test::TestQueryExt as _;
        window.try_find_by_label("Download anyway").is_some()
    });
    app.click_label("Cancel");
    app.wait_for("the warning to go", |window, _| {
        use gpui_kit::test::TestQueryExt as _;
        window.try_find_by_label("Download anyway").is_none()
    });
    assert!(app.shows("db-install"), "nothing downloads");
}

/// A setting saved elsewhere (another window, the command line) is shown here at once: the
/// core's `settings:saved` reads the settings again.
#[gpui_kit::test]
fn a_setting_saved_elsewhere_is_shown(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Sound");
    assert_eq!(app.find("sound").checked(), Some(true));
    let base = app.saved_settings();
    let mut edited = base.clone();
    edited["soundEnabled"] = json!(false);
    let settings = |value| serde_json::from_value(value).expect("settings");
    app.request(kchess_contracts::calls::SaveDesktopSettings {
        settings: settings(edited),
        base: Some(settings(base)),
    });
    app.wait_for("the saved setting", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find(gpui_kit::ElementId::from("sound"))
            .is_some_and(|sound| sound.checked() == Some(false))
    });
}

/// Another window logged a spoken phrase: the open voice history lists it. It stayed empty
/// until Refresh or the category was opened again.
#[gpui_kit::test]
fn voice_practice_in_another_window_shows_in_the_history(cx: &mut gpui_kit::TestAppContext) {
    use kchess_contracts::types::{
        ProfileArea, VoiceAttemptInput, VoiceOutcome, VoiceSource, VoiceWord,
    };
    let mut app = Harness::start(cx);
    app.open_settings("Voice input");
    app.find("voice-history-clear");
    assert!(!app.enabled("voice-history-clear"), "nothing logged yet");
    app.request(kchess_contracts::calls::SaveVoiceAttempt {
        attempt: VoiceAttemptInput {
            source: VoiceSource::Coordinates,
            heard: "e four".into(),
            confidence: 0.9,
            words: vec![VoiceWord {
                word: "e".into(),
                conf: 0.9,
            }],
            outcome: VoiceOutcome::Correct,
            parsed: Some("e4".into()),
            expected: Some("e4".into()),
            fen: None,
            retry_of: None,
        },
    });
    app.changed_elsewhere(&[ProfileArea::VoiceHistory]);
    app.wait_for("the logged phrase", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find(gpui_kit::ElementId::from("voice-history-clear"))
            .is_some_and(|clear| clear.disabled() != Some(true))
    });
}

/// Another window installed or deleted the engine: Engine shows what the core finds now.
#[gpui_kit::test]
fn an_engine_changed_in_another_window_is_read_again(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Chess engine");
    let settings = || crate::shell::view::<crate::pages::settings::SettingsPage>;
    app.wait_for("the engine's status", |_, cx| {
        settings()(cx).is_some_and(|page| page.read(cx).engine_status().is_some())
    });
    // Forget it, as if it were stale: only a report of the change reads it again.
    app.cx.update(|cx| {
        settings()(cx)
            .expect("the Settings page")
            .update(cx, |page, _| page.forget_engine_status())
    });
    app.changed_elsewhere(&[kchess_contracts::types::ProfileArea::Engine]);
    app.wait_for("the engine read again", |_, cx| {
        settings()(cx).is_some_and(|page| page.read(cx).engine_status().is_some())
    });
}

/// A change brings up a notice whose Undo puts the setting back as it was, saved.
#[gpui_kit::test]
fn undo_takes_back_a_settings_change(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Sound");
    app.click("sound");
    app.wait_for_saved("soundEnabled", json!(false));
    assert!(app.shows(crate::setting_notice::UNDO), "{}", app.paths());
    app.click(crate::setting_notice::UNDO);
    assert_eq!(app.find("sound").checked(), Some(true));
    app.wait_for_saved("soundEnabled", json!(true));
}

/// Changes soon after each other to one setting are one change: Undo goes back to before the
/// first, and a later change to another setting is not taken back with it.
#[gpui_kit::test]
fn undo_goes_back_to_before_a_run_of_changes(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Gameplay");
    let (ask, queen) = (("promotion", 0usize), ("promotion", 1usize));
    let first = app.saved_settings()["promotion"].clone();
    app.click(ask);
    app.click(queen);
    app.wait_for_saved("promotion", json!("queen"));
    app.click(crate::setting_notice::UNDO);
    app.wait_for_saved("promotion", first);
    assert!(!app.shows(crate::setting_notice::UNDO));
}

/// A new profile promotes a premoved pawn to a queen, as Lichess does, and asks otherwise.
#[gpui_kit::test]
fn a_new_profile_queens_premoves_and_asks_otherwise(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    assert_eq!(app.saved_settings()["promotion"], json!("premove"));
    app.open_settings("Gameplay");
    let premove = ("promotion", 2usize);
    assert_eq!(app.find(premove).label(), Some("Queen when premoving"));
    assert_eq!(app.find(premove).selected(), Some(true));
}

/// A profile that chose Ask before the default changed keeps asking.
#[gpui_kit::test]
fn a_stored_promotion_choice_outlives_the_new_default(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Gameplay");
    app.click(("promotion", 0usize));
    app.wait_for_saved("promotion", json!("ask"));
    let mut app = app.restart();
    assert_eq!(app.saved_settings()["promotion"], json!("ask"));
}

/// Restore defaults shows while a category differs from a new profile: it puts the category's
/// settings back, and Undo brings the person's own again.
#[gpui_kit::test]
fn restore_defaults_puts_a_category_back_and_undo_returns_it(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Gameplay");
    assert!(
        !app.shows("restore-defaults"),
        "a new profile is at its defaults"
    );
    app.click("premove");
    app.click(("promotion", 0usize));
    app.wait_for_saved("promotion", json!("ask"));
    app.click("restore-defaults");
    assert_eq!(app.find("premove").checked(), Some(true));
    app.wait_for_saved("promotion", json!("premove"));
    app.wait_gone("restore-defaults");
    app.click(crate::setting_notice::UNDO);
    assert_eq!(app.find("premove").checked(), Some(false));
    app.wait_for_saved("promotion", json!("ask"));
    assert!(app.shows("restore-defaults"));
}

/// A changed setting's row shows a reset that names the default: it puts the default back,
/// saved, and Undo brings the person's choice again. A row at its default shows none.
#[gpui_kit::test]
fn a_row_resets_its_setting_and_undo_returns_it(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Gameplay");
    assert!(
        !app.shows("reset-promotion"),
        "a new profile is at its defaults"
    );
    app.click(("promotion", 0usize));
    app.wait_for_saved("promotion", json!("ask"));
    assert_eq!(
        app.find("reset-promotion").label(),
        Some("Reset Pawn promotion to default: Queen when premoving")
    );
    assert!(!app.shows("reset-premove"), "{}", app.paths());
    app.click("reset-promotion");
    assert_eq!(app.find(("promotion", 2usize)).selected(), Some(true));
    app.wait_for_saved("promotion", json!("premove"));
    app.wait_gone("reset-promotion");
    app.click(crate::setting_notice::UNDO);
    assert_eq!(app.find(("promotion", 0usize)).selected(), Some(true));
    app.wait_for_saved("promotion", json!("ask"));
    assert!(app.shows("reset-promotion"));
}

/// A switch turned away from its default resets to it, the other switches showing no reset.
#[gpui_kit::test]
fn a_switch_resets_to_its_default(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Notifications");
    app.click("notify-sound");
    assert_eq!(
        app.find("reset-notifySound").label(),
        Some("Reset System sound to default: Off")
    );
    assert!(!app.shows("reset-notificationsEnabled"));
    app.click("reset-notifySound");
    assert_eq!(app.find("notify-sound").checked(), Some(false));
    app.wait_for_saved("notifySound", json!(false));
    app.wait_gone("reset-notifySound");
}

/// Regression: the Appearance pickers scrolled straight to the dialog's bottom edge, and GPUI
/// clips to a rectangle, so cards and labels were drawn over its rounded corners. The page ends
/// a corner's height above the dialog's outer edge.
#[gpui_kit::test]
fn the_settings_page_ends_above_the_dialogs_rounded_corners(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Appearance");
    let dialog = app.find(super::SETTINGS_SHOWN).bounds();
    let page = app.find("settings-page").bounds();
    assert!(
        page.bottom() <= dialog.bottom() - crate::theme::radii::BOX,
        "the page ({page:?}) reaches into the dialog's ({dialog:?}) rounded corners",
    );
}
