//! Exporting a game (`ExportGame.vue`): the Export menu under a game's moves, for an animated GIF
//! of the game, a PNG of the position shown or a PGN file; and saving an export (`saveExport`),
//! where the person picks the place, starting in Downloads.

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
use shakmaty::Color;

use crate::assets::icon;
use crate::game::rules;
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
    async move {
        match chosen.await {
            Ok(Ok(Some(path))) => {
                Some(std::fs::write(&path, data).map_err(|cause| cause.to_string()))
            }
            Ok(Ok(None)) | Err(_) => None,
            Ok(Err(cause)) => Some(Err(cause.to_string())),
        }
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

/// The game an Export menu saves (`ExportGame`'s props).
#[derive(Clone)]
pub struct GameExport {
    pub variant: String,
    pub fen: String,
    pub moves: Vec<String>,
    pub orientation: Color,
    pub white: String,
    pub black: String,
    /// The position shown, for the PNG; the last one by default.
    pub ply: Option<usize>,
    pub result: Option<String>,
}

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

/// `setupPgn`: the game as a PGN file, dated today.
fn game_pgn(game: &GameExport) -> Option<String> {
    rules(
        "setupPgn",
        &[
            json!({ "variant": game.variant, "fen": game.fen }),
            json!(game.moves),
            json!({
                "White": game.white,
                "Black": game.black,
                "Result": game.result.clone().unwrap_or_else(|| "*".into()),
                "Date": chrono::Local::now().format("%Y.%m.%d").to_string(),
            }),
        ],
    )
    .ok()?
    .as_str()
    .map(str::to_string)
}

/// An export under way: its progress, and how to stop it.
#[derive(Default)]
pub struct Exporter {
    busy: bool,
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

    fn finish(&mut self, cx: &mut Context<Self>) {
        self.busy = false;
        self.progress.clear();
        self.cancel = None;
        cx.notify();
    }

    fn save(&mut self, kind: Kind, game: GameExport, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
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
                Kind::Png => {
                    game_image::position_png(&options, game.ply.unwrap_or(game.moves.len()))
                }
                Kind::Pgn => game_pgn(&game)
                    .map(String::into_bytes)
                    .ok_or_else(|| "That game cannot be replayed.".to_string()),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            while let Some((done, total)) = heard.next().await {
                let _ = this.update(cx, |this, cx| {
                    this.progress = format!("{done}/{total}");
                    cx.notify();
                });
            }
            let data = work.await;
            let stopped = this
                .read_with(cx, |this, _| {
                    this.cancel
                        .as_ref()
                        .is_none_or(|cancel| cancel.load(Ordering::Relaxed))
                })
                .unwrap_or(true);
            let outcome = match data {
                _ if stopped => None,
                Err(cause) => Some(Err(cause)),
                Ok(data) => match cx.update(|_, cx| save_bytes(&name, data, cx)) {
                    Ok(saving) => saving.await,
                    Err(_) => None,
                },
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.finish(cx);
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
/// to cancel it.
pub fn export_menu(
    id: &'static str,
    exporter: &Entity<Exporter>,
    game: GameExport,
    anchor: Anchor,
    cx: &App,
) -> impl IntoElement {
    let state = exporter.read(cx);
    let busy = state.busy;
    let progress = if state.progress.is_empty() {
        "Preparing export…".to_string()
    } else {
        state.progress.clone()
    };
    let disabled = busy || (game.moves.is_empty() && game.ply.is_none());
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
                    .xsmall()
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
                .xsmall()
                .icon(icon("download"))
                .label("Export")
                .loading(busy)
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
                    menu.item(item("Animated GIF of the game", icon("film"), Kind::Gif))
                        .item(item("PNG of this position", icon("image"), Kind::Png))
                        .item(item("PGN file", icon("file-text"), Kind::Pgn))
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::{GameExport, game_file_name, game_pgn};
    use shakmaty::Color;

    #[test]
    fn a_pgn_names_its_players_and_result() {
        let pgn = game_pgn(&GameExport {
            variant: "standard".into(),
            fen: crate::game::STANDARD_FEN.into(),
            moves: vec!["e2e4".into(), "e7e5".into()],
            orientation: Color::White,
            white: "You".into(),
            black: "Stockfish".into(),
            ply: None,
            result: None,
        })
        .unwrap();
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
