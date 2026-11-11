//! Exporting a game: the Export menu under a game's moves, for an animated GIF
//! of the game, a PNG of the position shown, a PGN file, or the PGN or position's FEN copied; and
//! saving an export (`saveExport`), where the person picks the place, starting in Downloads.

use kchess_contracts::calls;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::json;

use crate::assets::icon;
use crate::game_image::{self, ImageOptions};
use crate::settings::SettingsHandle;

/// Ask where to save `text` as `name`, then write it; the outcome, or `None` when cancelled.
pub fn save_text(
    name: &str,
    text: String,
    cx: &mut App,
) -> impl Future<Output = Option<Result<(), String>>> + 'static + use<> {
    save_bytes(name, text.into_bytes(), cx)
}

/// Ask where to save `data` as `name`, then write it; the outcome, or `None` when cancelled.
pub fn save_bytes(
    name: &str,
    data: Vec<u8>,
    cx: &mut App,
) -> impl Future<Output = Option<Result<(), String>>> + 'static + use<> {
    let folder = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    let chosen = cx.prompt_for_new_path(&folder, Some(name));
    let extension = std::path::Path::new(name)
        .extension()
        .map(|extension| extension.to_os_string());
    async move {
        match chosen.await {
            Ok(Ok(Some(path))) => {
                let path = with_extension(path, extension.as_deref());
                Some(std::fs::write(&path, data).map_err(|cause| cause.to_string()))
            }
            Ok(Ok(None)) | Err(_) => None,
            Ok(Err(cause)) => Some(Err(cause.to_string())),
        }
    }
}

/// `path` with the export's extension when the name typed has none: Windows' save dialog filters
/// on All files and hides known extensions, so a retyped name easily loses it. An existing file
/// the dialog did not confirm replacing is left alone, and `path` kept as chosen.
fn with_extension(path: PathBuf, extension: Option<&std::ffi::OsStr>) -> PathBuf {
    match extension {
        Some(extension) if path.extension().is_none() => {
            let mut name = path.clone().into_os_string();
            name.push(".");
            name.push(extension);
            let added = PathBuf::from(name);
            if added.exists() { path } else { added }
        }
        _ => path,
    }
}

/// A file name from the players and today's date (`ExportGame`'s): safe characters only.
pub fn game_file_name(white: &str, black: &str) -> String {
    let today = chrono::Local::now().format("%Y-%m-%d");
    format!("{white} vs {black} {today}")
        .chars()
        .filter(|c| c.is_alphanumeric() || " _.()+-".contains(*c))
        .take(80)
        .collect()
}

use crate::game::played::PlayedGame;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Gif,
    Png,
    Pgn,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Gif => "gif",
            Kind::Png => "png",
            Kind::Pgn => "pgn",
        }
    }
}

/// The game as a PGN file: its own document, else `setupPgn` of its moves, dated today.
fn game_pgn(game: &PlayedGame) -> Option<String> {
    if let Some(pgn) = &game.pgn {
        return Some(pgn.clone());
    }
    let result = kchess_games::Outcome::tag(game.result);
    let date = chrono::Local::now().format("%Y.%m.%d").to_string();
    kchess_domain::pgn::setup_pgn(
        kchess_domain::variant::Variant::from_key(&game.variant)?,
        &game.fen,
        &game.moves,
        &[
            ("Black", &game.black),
            ("Date", &date),
            ("Result", result),
            ("White", &game.white),
        ],
    )
    .ok()
}

/// The FEN of the position shown: a variation's end when it is off the moves, else the move
/// at `ply` (the last by default), with a Crazyhouse game's pockets.
fn shown_fen(game: &PlayedGame) -> Option<String> {
    let setup = json!({ "variant": game.variant, "fen": game.fen });
    let moves = match &game.shown {
        Some(shown) => shown.clone(),
        None => game.moves[..game.ply.unwrap_or(game.moves.len()).min(game.moves.len())].to_vec(),
    };
    let position = crate::game::view::position_at(&setup, &moves, moves.len())?;
    Some(shakmaty::fen::Fen::from_position(&position, shakmaty::EnPassantMode::Legal).to_string())
}

/// Copy `text` and say so, or say what could not be copied.
fn copy(text: Option<String>, what: &str, window: &mut Window, cx: &mut App) {
    match text {
        Some(text) => {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            window.push_notification(SharedString::from(format!("{what} copied")), cx);
        }
        None => window.push_notification(
            Notification::error(format!("Could not copy the {what}")),
            cx,
        ),
    }
}

/// Copy the game as its PGN file has it.
pub fn copy_pgn(game: &PlayedGame, window: &mut Window, cx: &mut App) {
    copy(game_pgn(game), "PGN", window, cx);
}

/// Copy the position shown as a FEN.
pub fn copy_fen(game: &PlayedGame, window: &mut Window, cx: &mut App) {
    copy(shown_fen(game), "FEN", window, cx);
}

/// An export under way: its progress, and how to stop it.
#[derive(Default)]
pub struct Exporter {
    busy: bool,
    uploading: bool,
    progress: String,
    cancel: Option<Arc<AtomicBool>>,
}

impl Drop for Exporter {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Exporter {
    fn stop(&mut self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Whether `cancel` is the token of the export running now.
    fn runs(&self, cancel: &Arc<AtomicBool>) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, cancel))
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        self.busy = false;
        self.progress.clear();
        self.cancel = None;
        cx.notify();
    }

    /// Upload the game to Lichess (as the online account, if any) and copy its page's link.
    fn upload(&mut self, game: PlayedGame, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.uploading {
            return;
        }
        let link = if let Some(id) = &game.lichess_id {
            Ok(format!("https://lichess.org/{id}"))
        } else {
            let account = crate::accounts::handle(cx)
                .read(cx)
                .active(cx)
                .unwrap_or_default();
            let Some(pgn) = game_pgn(&game) else {
                window.push_notification(
                    Notification::error("That game cannot be replayed")
                        .title("Could not upload the game"),
                    cx,
                );
                return;
            };
            Err(cx
                .global::<crate::core::AppCore>()
                .request(calls::ImportGame {
                    account: json!(account),
                    pgn: json!(pgn),
                }))
        };
        self.uploading = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let url = match link {
                Ok(url) => Ok(url),
                Err(upload) => upload.await.and_then(|game| {
                    game["url"]
                        .as_str()
                        .map(str::to_string)
                        .ok_or_else(|| kchess_contracts::CoreError::new("Lichess sent no link"))
                }),
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.uploading = false;
                cx.notify();
                match url {
                    Ok(url) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                        window.push_notification(
                            Notification::new()
                                .title("Copied the Lichess link")
                                .message(url),
                            cx,
                        );
                    }
                    Err(cause) => window.push_notification(
                        Notification::error(cause.message).title("Could not upload the game"),
                        cx,
                    ),
                }
            });
        })
        .detach();
    }

    fn save(&mut self, kind: Kind, game: PlayedGame, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        // This export's own token: a cancelled export may still be finishing when the next starts.
        let own = Arc::clone(&cancel);
        self.busy = true;
        self.progress.clear();
        self.cancel = Some(cancel.clone());
        cx.notify();
        let name = format!(
            "{}.{}",
            game_file_name(&game.white, &game.black),
            kind.name()
        );
        let frontend = &cx.global::<SettingsHandle>().0.read(cx).settings.frontend;
        let options = ImageOptions {
            variant: game.variant.clone(),
            fen: game.fen.clone(),
            moves: game.moves.clone(),
            orientation: game.orientation,
            board_theme: frontend.board_theme.clone(),
            piece_set: frontend.piece_set.clone(),
            white: game.white.clone(),
            black: game.black.clone(),
            size: 360,
        };
        let (sender, mut heard) = futures::channel::mpsc::unbounded::<(usize, usize)>();
        let work = cx.background_spawn(async move {
            match kind {
                // Drawing a GIF takes a moment for long games.
                Kind::Gif => game_image::game_gif(&options, 80, &cancel, |done, total| {
                    let _ = sender.unbounded_send((done, total));
                }),
                Kind::Png => match &game.shown {
                    Some(shown) => game_image::position_png(
                        &ImageOptions {
                            moves: shown.clone(),
                            ..options
                        },
                        shown.len(),
                    ),
                    None => {
                        game_image::position_png(&options, game.ply.unwrap_or(game.moves.len()))
                    }
                },
                Kind::Pgn => game_pgn(&game)
                    .map(String::into_bytes)
                    .ok_or_else(|| "That game cannot be replayed".to_string()),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            while let Some((done, total)) = heard.next().await {
                let _ = this.update(cx, |this, cx| {
                    if this.runs(&own) {
                        this.progress = format!("{done}/{total}");
                        cx.notify();
                    }
                });
            }
            let data = work.await;
            let stopped = own.load(Ordering::Relaxed) || this.upgrade().is_none();
            let outcome = match data {
                _ if stopped => None,
                Err(cause) => Some(Err(cause)),
                Ok(data) => match cx.update(|_, cx| save_bytes(&name, data, cx)) {
                    Ok(saving) => saving.await,
                    Err(_) => None,
                },
            };
            let _ = this.update_in(cx, |this, window, cx| {
                if this.runs(&own) {
                    this.finish(cx);
                }
                match outcome {
                    Some(Ok(())) => window.push_notification(
                        SharedString::from(format!("Saved the {}", kind.name().to_uppercase())),
                        cx,
                    ),
                    Some(Err(cause)) => window.push_notification(
                        Notification::error(cause).title("Could not export the game"),
                        cx,
                    ),
                    None => {}
                }
            });
        })
        .detach();
    }
}

/// The Export menu, opening towards `anchor`, and while an export runs, its progress and a way
/// to cancel it. `copy` adds Copy PGN and Copy FEN, for a page that has no copy of its own.
pub fn export_menu(
    id: &'static str,
    exporter: &Entity<Exporter>,
    game: PlayedGame,
    anchor: Anchor,
    copy: bool,
    cx: &App,
) -> impl IntoElement {
    let state = exporter.read(cx);
    let busy = state.busy;
    let progress = if state.progress.is_empty() {
        "Preparing export…".to_string()
    } else {
        state.progress.clone()
    };
    let uploading = state.uploading;
    let disabled = busy || uploading || (game.moves.is_empty() && game.ply.is_none());
    let can_upload =
        game.lichess_id.is_some() || (crate::network::online(cx) && !game.moves.is_empty());
    let target = exporter.downgrade();
    h_flex()
        .gap(px(4.))
        .when(busy, |row| {
            let cancel = target.clone();
            row.child(
                div()
                    .w(px(112.))
                    .flex_none()
                    .text_size(px(12.))
                    .child(progress),
            )
            .child(
                Button::new((id, 1usize))
                    .ghost()
                    .small()
                    .label("Cancel export")
                    .on_click(move |_, _, cx| {
                        let _ = cancel.update(cx, |exporter, cx| {
                            exporter.stop();
                            exporter.finish(cx);
                        });
                    }),
            )
        })
        .child(
            Button::new(id)
                .ghost()
                .small()
                .icon(icon("download"))
                .label("Export")
                .loading(busy || uploading)
                .disabled(disabled)
                .dropdown_menu_with_anchor(anchor, move |menu, _, _| {
                    let item = |label: &'static str, glyph, kind: Kind| {
                        let (target, game) = (target.clone(), game.clone());
                        PopupMenuItem::new(label)
                            .icon(glyph)
                            .on_click(move |_, window, cx| {
                                let _ = target.update(cx, |exporter, cx| {
                                    exporter.save(kind, game.clone(), window, cx)
                                });
                            })
                    };
                    let (upload_target, upload_game) = (target.clone(), game.clone());
                    let (pgn_game, fen_game) = (game.clone(), game.clone());
                    menu.item(item("Animated GIF of the game", icon("film"), Kind::Gif))
                        .item(item("PNG of this position", icon("image"), Kind::Png))
                        .item(item("PGN file", icon("file-text"), Kind::Pgn))
                        .when(copy, |menu| {
                            menu.separator()
                                .item(
                                    PopupMenuItem::new("Copy PGN").icon(icon("copy")).on_click(
                                        move |_, window, cx| copy_pgn(&pgn_game, window, cx),
                                    ),
                                )
                                .item(
                                    PopupMenuItem::new("Copy FEN").icon(icon("copy")).on_click(
                                        move |_, window, cx| copy_fen(&fen_game, window, cx),
                                    ),
                                )
                        })
                        .separator()
                        .item(
                            PopupMenuItem::new("Lichess link")
                                .icon(icon("link"))
                                .disabled(!can_upload)
                                .on_click(move |_, window, cx| {
                                    let _ = upload_target.update(cx, |exporter, cx| {
                                        exporter.upload(upload_game.clone(), window, cx)
                                    });
                                }),
                        )
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::{PlayedGame, game_file_name, game_pgn, shown_fen, with_extension};
    use shakmaty::Color;
    use std::ffi::OsStr;

    /// A name retyped without its extension in Windows' save dialog still saves as a GIF; one
    /// typed with any extension, or without one when that file already exists, is kept.
    #[test]
    fn a_name_without_an_extension_gets_the_exports() {
        let gif = Some(OsStr::new("gif"));
        let folder = tempfile::tempdir().unwrap();
        let typed = folder.path().join("My game");
        assert_eq!(
            with_extension(typed.clone(), gif),
            folder.path().join("My game.gif")
        );
        let named = folder.path().join("My game.png");
        assert_eq!(with_extension(named.clone(), gif), named);
        assert_eq!(with_extension(typed.clone(), None), typed);
        std::fs::write(folder.path().join("My game.gif"), b"").unwrap();
        assert_eq!(with_extension(typed.clone(), gif), typed);
    }

    fn game(variant: &str, fen: &str, moves: &[&str], ply: Option<usize>) -> PlayedGame {
        let mut game = PlayedGame::new(
            &serde_json::json!({ "variant": variant, "fen": fen }),
            moves.iter().map(|uci| uci.to_string()).collect(),
            Color::White,
            "White",
            "Black",
        );
        game.ply = ply;
        game
    }

    /// Copy FEN copies the position shown: the last by default, an earlier move's, a variation's,
    /// and a Crazyhouse position with its pockets.
    #[test]
    fn the_fen_is_the_position_shown() {
        let standard = crate::game::STANDARD_FEN;
        let moves = ["e2e4", "e7e5"];
        assert_eq!(
            shown_fen(&game("standard", standard, &moves, None)).as_deref(),
            Some("rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2")
        );
        assert_eq!(
            shown_fen(&game("standard", standard, &moves, Some(0))).as_deref(),
            Some(standard)
        );
        let mut variation = game("standard", standard, &moves, None);
        variation.shown = Some(vec!["d2d4".into()]);
        assert_eq!(
            shown_fen(&variation).as_deref(),
            Some("rnbqkbnr/pppppppp/8/8/3P4/8/PPP1PPPP/RNBQKBNR b KQkq - 0 1")
        );
        let pocket = "rnb1kbnr/ppp1pppp/8/3q4/8/8/PPPP1PPP/RNBQKBNR[Pp] w KQkq - 0 3";
        let fen = shown_fen(&game("crazyhouse", pocket, &["P@e3"], None)).expect("a FEN");
        assert!(fen.contains("[p]"), "{fen}");
    }

    #[test]
    fn a_pgn_names_its_players_and_result() {
        let mut played = game(
            "standard",
            crate::game::STANDARD_FEN,
            &["e2e4", "e7e5"],
            None,
        );
        (played.white, played.black) = ("You".into(), "Stockfish".into());
        let pgn = game_pgn(&played).unwrap();
        assert!(pgn.contains("[White \"You\"]"));
        assert!(pgn.contains("[Result \"*\"]"));
        assert!(pgn.contains("1. e4 e5"));
    }

    #[test]
    fn file_names_keep_safe_characters() {
        let name = game_file_name("You", "Stockfish (Club Player)/x");
        assert!(name.starts_with("You vs Stockfish (Club Player)x "));
        assert!(!name.contains('/'));
    }
}
