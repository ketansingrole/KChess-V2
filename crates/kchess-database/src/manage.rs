//! The game databases on this computer at a glance: each one's source, games, size on disk and
//! last update, with its Update and Delete, and their total, so what the downloads take is never
//! a surprise.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::{Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;
use kchess_board::game_db::GameDbHandle;
use kchess_board::widgets;
use kchess_contracts::types::{DatabaseInfo, DatabaseSource};
use kchess_ui::icon;

use crate::theme::Palette;

/// Where a database's games came from, as the page's buttons name it.
fn source_label(source: DatabaseSource) -> &'static str {
    match source {
        DatabaseSource::Imported => "Imported",
        DatabaseSource::Own => "Your games",
        DatabaseSource::Master => "Master games",
        DatabaseSource::Player => "Player's games",
        DatabaseSource::Lichess => "Lichess games",
        DatabaseSource::Tcec => "Engine games",
    }
}

/// The bytes every database takes on this computer.
pub fn total_bytes(databases: &[DatabaseInfo]) -> u64 {
    databases.iter().map(|info| info.bytes).sum()
}

/// "Player's games · 1,204 games · 3.1 MB · updated Nov 24, 2026".
fn detail(info: &DatabaseInfo) -> String {
    let mut line = format!(
        "{} · {} · {}",
        source_label(info.source),
        widgets::format_counted(info.games as f64, "game", "games"),
        widgets::format_bytes(info.bytes as f64)
    );
    if info.updated_at > 0 {
        let date = kchess_ui::dates::local_format(info.updated_at as f64, "%b %-d, %Y");
        if !date.is_empty() {
            line.push_str(&format!(" · updated {date}"));
        }
    }
    line
}

/// Asks, then deletes `info`'s database.
pub fn ask_delete(info: DatabaseInfo, window: &mut Window, cx: &mut App) {
    let games = widgets::format_counted(info.games as f64, "game", "games");
    let size = widgets::format_bytes(info.bytes as f64);
    widgets::open_alert_dialog(window, cx, move |dialog: AlertDialog, _, _| {
        let id = info.id.clone();
        dialog
            .title(format!("Delete {}?", info.name))
            .description(format!(
                "Its {games} are removed from this computer, freeing {size}."
            ))
            .ok_text("Delete")
            .ok_variant(ButtonVariant::Danger)
            .cancel_text("Cancel")
            .show_cancel(true)
            .on_ok(move |_, _, cx| {
                let db = cx.global::<GameDbHandle>().0.clone();
                db.update(cx, |db, cx| db.delete(id.clone(), cx));
                true
            })
    });
}

/// What brings `info`'s database up to date, when it has one: the same download again, or the
/// next month or season.
fn update(info: &DatabaseInfo) -> Option<(&'static str, Box<dyn Fn(&mut Window, &mut App)>)> {
    match info.source {
        DatabaseSource::Player => {
            let earlier = info.player.clone()?;
            Some((
                "Update games",
                Box::new(move |window, cx| crate::player::open(Some(earlier.clone()), window, cx)),
            ))
        }
        DatabaseSource::Lichess => {
            let rating = info.lichess_rating?;
            Some((
                "Add a month",
                Box::new(move |window, cx| crate::lichess::open(Some(rating), window, cx)),
            ))
        }
        DatabaseSource::Master => Some((
            "Update",
            Box::new(|_, cx| {
                let db = cx.global::<GameDbHandle>().0.clone();
                db.update(cx, |db, cx| db.install_masters(cx));
            }),
        )),
        DatabaseSource::Tcec => Some((
            "Update",
            Box::new(|_, cx| {
                let db = cx.global::<GameDbHandle>().0.clone();
                db.update(cx, |db, cx| db.install_tcec(cx));
            }),
        )),
        // Your games keep themselves up to date; an imported file has nothing to fetch.
        DatabaseSource::Own | DatabaseSource::Imported => None,
    }
}

/// Opens the list of game databases.
pub fn open(window: &mut Window, cx: &mut App) {
    let view = cx.new(ManageDatabases::new);
    widgets::open_dialog(window, cx, move |dialog_box, _, _| {
        dialog_box
            .title("Game databases")
            .w(px(560.))
            .child(view.clone())
    });
}

pub struct ManageDatabases {
    _subscriptions: Vec<Subscription>,
}

impl ManageDatabases {
    fn new(cx: &mut Context<Self>) -> Self {
        let db = cx.global::<GameDbHandle>().0.clone();
        // A database deleted or grown shows at once.
        ManageDatabases {
            _subscriptions: vec![cx.observe(&db, |_, _, cx| cx.notify())],
        }
    }
}

impl Render for ManageDatabases {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let db = cx.global::<GameDbHandle>().0.read(cx);
        let busy = db.busy();
        let databases = db.databases().to_vec();
        if databases.is_empty() {
            return v_flex().child(widgets::empty_state(
                icon("database"),
                "No game databases",
                None,
                &palette,
            ));
        }
        let total = total_bytes(&databases);
        let rows = databases
            .into_iter()
            .enumerate()
            .map(|(index, info)| row(index, info, busy, &palette));
        v_flex().gap(px(4.)).child(v_flex().children(rows)).child(
            h_flex()
                .id("manage-total")
                .test_support()
                .aria_label(format!("Total {}", widgets::format_bytes(total as f64)))
                .pt(px(8.))
                .justify_between()
                .text_size(px(13.))
                .child(div().text_color(palette.text_muted).child("Total"))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(widgets::format_bytes(total as f64)),
                ),
        )
    }
}

/// One database: its name and what it holds, then its Update and Delete (not while a download
/// or import runs, as on the page).
fn row(index: usize, info: DatabaseInfo, busy: bool, palette: &Palette) -> impl IntoElement {
    let update = update(&info).filter(|_| !busy).map(|(label, run)| {
        Button::new(("manage-update", index))
            .ghost()
            .small()
            .icon(icon("refresh-cw"))
            .label(label)
            .on_click(move |_, window, cx| {
                window.close_dialog(cx);
                run(window, cx);
            })
    });
    let delete = (!busy).then(|| {
        let info = info.clone();
        Button::new(("manage-delete", index))
            .ghost()
            .small()
            .icon(icon("trash-2"))
            .accessibility_label(format!("Delete {}", info.name))
            .on_click(move |_, window, cx| ask_delete(info.clone(), window, cx))
    });
    h_flex()
        .id(("manage-database", index))
        .test_support()
        .aria_label(format!("{}, {}", info.name, detail(&info)))
        .gap(px(12.))
        .py(px(8.))
        .border_b_1()
        .border_color(palette.border)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(info.name.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(palette.text_muted)
                        .child(detail(&info)),
                ),
        )
        .children(update)
        .children(delete)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(source: DatabaseSource, games: u64, bytes: u64) -> DatabaseInfo {
        DatabaseInfo {
            id: "d".into(),
            name: "D".into(),
            source,
            games,
            bytes,
            updated_at: 0,
            index_depth: 0,
            attribution: None,
            player: None,
            lichess_rating: None,
        }
    }

    #[test]
    fn a_database_reads_as_its_source_games_and_size() {
        let line = detail(&info(DatabaseSource::Tcec, 1204, 3 * 1024 * 1024));
        assert!(line.starts_with("Engine games · 1,204 games · "), "{line}");
        assert!(!line.contains("updated"), "never updated: {line}");
    }

    #[test]
    fn the_total_is_every_database_s_size() {
        let all = [
            info(DatabaseSource::Own, 1, 100),
            info(DatabaseSource::Master, 1, 900),
        ];
        assert_eq!(total_bytes(&all), 1000);
        assert_eq!(total_bytes(&[]), 0);
    }

    #[test]
    fn only_a_database_that_can_fetch_more_offers_an_update() {
        assert!(update(&info(DatabaseSource::Own, 1, 1)).is_none());
        assert!(update(&info(DatabaseSource::Imported, 1, 1)).is_none());
        // A player's database made before downloads were kept has nothing to repeat.
        assert!(update(&info(DatabaseSource::Player, 1, 1)).is_none());
        assert!(update(&info(DatabaseSource::Master, 1, 1)).is_some());
    }
}
