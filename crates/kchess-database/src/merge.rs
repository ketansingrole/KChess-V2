//! Merge into a study: the games a search finds as one chapter whose moves are all of theirs,
//! the most played the main line (`gameDbMergeGames`), in a new study or one picked; the chapter
//! then opens on the analysis board.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use kchess_app::studies::StudyHandle;
use kchess_board::analysis::Open;
use kchess_board::game_db::Chosen;
use kchess_board::widgets;
use kchess_contracts::calls;
use kchess_contracts::check::{DatabaseId, LibraryId, StudyName};
use kchess_contracts::types::{DatabaseInfo, GameDbMerged};

use crate::core::AppCore;
use crate::loading;
use crate::target;

/// Asks which study, then merges the `chosen` games of `from` into a new chapter of it.
pub fn merge(from: DatabaseInfo, chosen: Chosen, window: &mut Window, cx: &mut App) {
    let library = cx.global::<StudyHandle>().0.read(cx);
    let ask = target::Ask {
        title: "Merge into a study",
        choices: target::study_targets(library.items()),
        kind: "Study",
        first: None,
        name: library.fresh_name(&from.name),
        confirm: "Merge".to_string(),
        glyph: "book-plus",
    };
    target::open(ask, window, cx, move |into, name, window, cx| {
        start(from.id.clone(), chosen.clone(), into, name, window, cx);
    });
}

fn start(
    database: String,
    chosen: Chosen,
    study: Option<String>,
    name: Option<String>,
    window: &mut Window,
    cx: &mut App,
) {
    let (games, search) = chosen.parts();
    let request = cx.global::<AppCore>().request(calls::GameDbMergeGames {
        database: DatabaseId(database),
        games,
        search,
        study: study.map(LibraryId),
        name: name.map(StudyName),
    });
    window
        .spawn(cx, async move |cx| {
            let answer = request.await;
            cx.update(|window, cx| match answer {
                Ok(merged) => {
                    let library = cx.global::<StudyHandle>().0.clone();
                    library.update(cx, |library, cx| {
                        library.apply(&merged.saved);
                        cx.notify();
                    });
                    let name = library
                        .read(cx)
                        .study(&merged.study)
                        .map(|study| study.name.clone())
                        .unwrap_or_default();
                    window.push_notification(Notification::success(summary(&merged, &name)), cx);
                    kchess_board::analysis::open(
                        Open::StudyLine {
                            study: merged.study,
                            chapter: merged.chapter,
                            moves: Vec::new(),
                        },
                        window,
                        cx,
                    );
                }
                Err(cause) => window.push_notification(
                    Notification::error(loading::failure("database", &cause))
                        .title("Could not merge the games"),
                    cx,
                ),
            })
            .ok();
        })
        .detach();
}

/// What the merge did: "Merged 12 games into “Najdorf games”", and the games left out when some
/// were.
pub fn summary(merged: &GameDbMerged, study: &str) -> String {
    let mut text = format!(
        "Merged {} into “{study}”",
        widgets::format_counted(merged.merged as f64, "game", "games")
    );
    if merged.left_out > 0 {
        text.push_str(&format!(
            ". {} from another position left out",
            merged.left_out
        ));
    }
    if merged.no_room > 0 {
        text.push_str(&format!(
            ". {} more did not fit in one chapter",
            merged.no_room
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::summary;
    use kchess_contracts::types::GameDbMerged;

    #[test]
    fn the_summary_names_what_was_left_out_only_when_some_was() {
        let merged = |merged, left_out, no_room| GameDbMerged {
            merged,
            left_out,
            no_room,
            ..GameDbMerged::default()
        };
        assert_eq!(
            summary(&merged(12, 0, 0), "Najdorf games"),
            "Merged 12 games into “Najdorf games”"
        );
        assert_eq!(
            summary(&merged(1, 3, 0), "Mine"),
            "Merged 1 game into “Mine”. 3 from another position left out"
        );
        assert_eq!(
            summary(&merged(200, 0, 40), "Mine"),
            "Merged 200 games into “Mine”. 40 more did not fit in one chapter"
        );
    }
}
