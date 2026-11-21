use super::{Layout, PositionExplorer, Prefs, category, speed_label, toggled};
use crate::e2e::Harness;
use gpui_kit::AppContext as _;
use kchess_contracts::types::{ExplorerSpeed, PositionLookupKind};
use shakmaty::Color;

/// Choosing the player database with no name yet left a lookup for the other database
/// current: its answer then showed under the player tab, and the spinner stayed on until it.
#[gpui_kit::test]
fn a_lookup_for_another_database_does_not_land_on_the_player_tab(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut app = Harness::start(cx);
    let (busy, stale) = app.with_window(|window, cx| {
        let explorer = cx.new(|cx| PositionExplorer::new(Layout::List, window, cx));
        explorer.update(cx, |explorer, cx| {
            explorer.lookup(Some(PositionLookupKind::Masters), false, None, cx);
            let asked = explorer.epoch.current();
            explorer.lookup(Some(PositionLookupKind::Player), false, None, cx);
            (explorer.busy, explorer.epoch.is_current(asked))
        })
    });
    assert!(!busy && !stale);
}

/// A lookup refused while help was held back is made again once help is lent, told by the
/// refusal's kind. Judged by seeing help switch off and on, the explorer missed both switches
/// when they came in one notification, or while the lookup was on its way, and kept Try again
/// (Linux and Windows CI, `the_explorer_offers_connect_lichess_and_looks_up_once_connected`).
#[gpui_kit::test]
fn a_refusal_while_help_was_held_back_is_looked_up_again(cx: &mut gpui_kit::TestAppContext) {
    use kchess_contracts::{CoreError, ErrorKind};
    let mut app = Harness::start(cx);
    let (explorer, asked) = app.with_window(|window, cx| {
        assert!(crate::online::handle(cx).read(cx).assistance_allowed());
        let explorer = cx.new(|cx| PositionExplorer::new(Layout::List, window, cx));
        let asked = explorer.update(cx, |explorer, _| {
            explorer.failure = Some(CoreError::new("Held back.").with_kind(ErrorKind::Withheld));
            explorer.epoch.current()
        });
        (explorer, asked)
    });
    // Help lent, in the one notification it came with.
    app.with_window(|_, cx| crate::online::handle(cx).update(cx, |_, cx| cx.notify()));
    let looked_again = app.with_window(|_, cx| !explorer.read(cx).epoch.is_current(asked));
    assert!(looked_again, "no lookup made again");

    // Any other failure waits for Try again.
    let asked = app.with_window(|_, cx| {
        let asked = explorer.update(cx, |explorer, _| {
            explorer.failure = Some(CoreError::new("Lookup failed."));
            explorer.epoch.current()
        });
        crate::online::handle(cx).update(cx, |_, cx| cx.notify());
        asked
    });
    assert!(app.with_window(|_, cx| explorer.read(cx).epoch.is_current(asked)));
}

use crate::game::is_username;
use serde_json::json;

#[test]
fn options_leave_out_what_is_not_set() {
    let mut prefs = Prefs::default();
    assert_eq!(
        json!(prefs.options()),
        json!({ "speeds": ["blitz", "rapid", "classical"], "ratings": [] })
    );
    prefs.kind = PositionLookupKind::Masters;
    prefs.since = "2019-05".into();
    assert_eq!(json!(prefs.options()), json!({ "since": "2019" }));
    prefs.kind = PositionLookupKind::Tablebase;
    assert_eq!(json!(prefs.options()), json!({}));
}

/// The preference an earlier version stored reads as it did when its values are all known,
/// and saves back the same; a value this app does not know reads as the default.
#[test]
fn stored_preferences_still_load() {
    let stored = json!({
        "kind": "player",
        "speeds": ["bullet", "correspondence"],
        "ratings": [1600.0, 2000.0],
        "player": "DrNykterstein",
        "color": "black",
        "since": "2020-01",
        "follow": true,
    });
    let prefs: Prefs = serde_json::from_value(stored.clone()).unwrap();
    assert_eq!(prefs.kind, PositionLookupKind::Player);
    assert_eq!(
        prefs.speeds,
        [ExplorerSpeed::Bullet, ExplorerSpeed::Correspondence]
    );
    assert_eq!(prefs.color, Color::Black);
    assert_eq!(json!(prefs), stored);

    let odd: Prefs = serde_json::from_value(json!({
        "kind": "lichess",
        "speeds": ["blitz", "hyperBullet"],
        "color": "green",
    }))
    .unwrap();
    assert_eq!(odd.kind, PositionLookupKind::Opening);
    assert_eq!(odd.speeds, [ExplorerSpeed::Blitz]);
    assert_eq!(odd.color, Color::White);
    let empty: Prefs = serde_json::from_value(json!({ "speeds": "fast" })).unwrap();
    assert_eq!(empty.speeds, Vec::<ExplorerSpeed>::new());
    assert_eq!(speed_label(ExplorerSpeed::Correspondence), "Corr");
    assert_eq!(speed_label(ExplorerSpeed::UltraBullet), "UltraBullet");
}

#[test]
fn a_tablebase_result_reads_from_either_side() {
    assert_eq!(category("cursed-win", false), "cursed win");
    assert_eq!(category("win", true), "loss");
    assert_eq!(category("cursed-win", true), "blessed loss");
    assert_eq!(category("blessed-loss", true), "cursed win");
    assert_eq!(category("draw", true), "draw");
}

#[test]
fn filters_toggle_and_names_are_checked() {
    assert_eq!(toggled(&[1, 2], &2), vec![1]);
    assert_eq!(toggled(&[1], &2), vec![1, 2]);
    assert!(is_username("DrNykterstein"));
    assert!(!is_username("a"));
    assert!(!is_username("no spaces"));
}
