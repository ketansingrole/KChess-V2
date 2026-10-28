//! The engine on broadcasts (`BroadcastPanel.vue`, `useBroadcastEvaluations`): Stockfish on the
//! board followed, with its lines, arrows and evaluation bar, and short sequential estimates for
//! the grid's evaluation bars. One engine serves both, and none of it runs during the player's own
//! Lichess game.

use std::collections::HashMap;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};
use shakmaty::{Color, Position as _, Square};

use super::WatchPage;
use super::broadcasts::{Broadcasts, moves_of, setup_of};
use crate::assets::icon;
use crate::board::Arrow;
use crate::core::AppCore;
use crate::game::tree::{analysis_context, format_eval};
use crate::game::view;
use crate::online::OnlineHandle;
use crate::position_engine::PositionEngine;
use crate::theme::Palette;
use crate::widgets::Choice;

const ENGINE_ON: &str = "kchess:broadcast-engine";
const LINES: &str = "kchess:broadcast-engine-lines";
const INFINITE: &str = "kchess:broadcast-engine-infinite";
const ARROWS: &str = "kchess:broadcast-engine-arrows";
const BAR: &str = "kchess:broadcast-engine-bar";
pub(super) const GRID_EVALUATIONS: &str = "kchess:broadcast-grid-evaluations";
/// A grid estimate stops here, or after this long, and the next board's starts.
const PREVIEW_DEPTH: f64 = 12.;
const PREVIEW_TIME: Duration = Duration::from_millis(1500);

/// A board's estimate for the grid, for the position it was made in.
struct Preview {
    context: String,
    /// None when the engine could not give one.
    line: Option<Value>,
}

pub(super) struct BroadcastEngine {
    pub(super) engine: PositionEngine,
    ready: bool,
    previews: HashMap<String, Preview>,
    /// The grid board being estimated now, and its position's context.
    previewing: Option<(String, String)>,
    _preview_done: Option<Task<()>>,
    /// Moves played from an engine line over the game (Explore), from where they leave it.
    pub(super) variation_base: Option<Vec<String>>,
    pub(super) variation: Vec<String>,
    lines_select: Entity<SelectState<Vec<Choice>>>,
}

fn engine_of(page: &mut WatchPage) -> &mut PositionEngine {
    &mut page.broadcasts.engine.engine
}

/// The context a board's live position is analysed in (`analysisContext`).
fn context_of(game: &Value) -> String {
    analysis_context(
        game["startFen"].as_str().unwrap_or_default(),
        &moves_of(game),
    )
}

fn assistance(cx: &App) -> bool {
    cx.global::<OnlineHandle>().0.read(cx).assistance_allowed()
}

impl BroadcastEngine {
    pub(super) fn new(window: &mut Window, cx: &mut Context<WatchPage>) -> BroadcastEngine {
        let lines = crate::prefs::get::<usize>(cx, LINES, 3).clamp(1, 5);
        let lines_select = cx.new(|cx| {
            let choices: Vec<Choice> = (1..=5)
                .map(|count| Choice::new(count.to_string(), count.to_string()))
                .collect();
            let mut state = SelectState::new(choices, None, window, cx);
            state.set_selected_value(&SharedString::from(lines.to_string()), window, cx);
            state
        });
        cx.subscribe(
            &lines_select,
            |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                if let SelectEvent::Confirm(Some(count)) = event {
                    crate::prefs::set(cx, LINES, count.parse::<usize>().unwrap_or(3));
                    this.engine_changed(cx);
                }
            },
        )
        .detach();
        BroadcastEngine {
            engine: PositionEngine::default(),
            ready: false,
            previews: HashMap::new(),
            previewing: None,
            _preview_done: None,
            variation_base: None,
            variation: Vec::new(),
            lines_select,
        }
    }
}

impl Broadcasts {
    /// The moves on the board: the game to the move shown, or the line explored from it.
    pub(super) fn history(&self) -> Vec<String> {
        if let Some(base) = &self.engine.variation_base {
            return base.iter().chain(&self.engine.variation).cloned().collect();
        }
        let moves = self.current().map(moves_of).unwrap_or_default();
        moves[..self.shown_ply().min(moves.len())].to_vec()
    }

    /// "1-0", "0-1" or "½-½" when the position on the board has ended the game.
    pub(super) fn terminal(&self) -> Option<&'static str> {
        let game = self.current()?;
        let history = self.history();
        let position = view::position_at(&setup_of(game), &history, history.len())?;
        if position.is_checkmate() {
            Some(if position.turn() == Color::White {
                "0-1"
            } else {
                "1-0"
            })
        } else if position.is_stalemate() || position.is_insufficient_material() {
            Some("½-½")
        } else {
            None
        }
    }

    pub(super) fn engine_ready(&self) -> bool {
        self.engine.ready
    }

    /// The engine follows the board shown.
    pub(super) fn engine_active(&self, cx: &App) -> bool {
        self.current().is_some()
            && self.engine.ready
            && crate::prefs::get(cx, ENGINE_ON, false)
            && assistance(cx)
    }

    /// The engine's arrows for the board, when they are shown.
    pub(super) fn engine_arrows(&self, cx: &App) -> Vec<Arrow> {
        if !self.engine_active(cx) || !crate::prefs::get(cx, ARROWS, true) {
            return Vec::new();
        }
        self.engine
            .engine
            .lines()
            .iter()
            .enumerate()
            .filter_map(|(rank, line)| {
                let uci = line["pv"][0].as_str()?;
                let square = |range: std::ops::Range<usize>| uci.get(range)?.parse::<Square>().ok();
                Some(Arrow::engine(square(0..2)?, square(2..4)?, rank))
            })
            .collect()
    }

    /// A grid board's estimate, and whether the engine could not make one for its position.
    pub(super) fn grid_evaluation(&self, game: &Value) -> (Option<Value>, bool) {
        let id = game["id"].as_str().unwrap_or_default();
        let context = context_of(game);
        if self.current().is_none()
            && self
                .engine
                .previewing
                .as_ref()
                .is_some_and(|(previewing, at)| previewing == id && *at == context)
            && let Some(update) = &self.engine.engine.update
            && update["context"].as_str() == Some(context.as_str())
        {
            return (
                update["lines"][0]
                    .as_object()
                    .map(|_| update["lines"][0].clone()),
                false,
            );
        }
        match self.engine.previews.get(id) {
            Some(preview) if preview.context == context => {
                (preview.line.clone(), preview.line.is_none())
            }
            _ => (None, false),
        }
    }

    /// Leave the explored line for the game.
    pub(super) fn return_to_game(&mut self) {
        self.engine.variation_base = None;
        self.engine.variation.clear();
    }
}

impl WatchPage {
    /// Whether the engine can run, read when the page is shown.
    pub(super) fn read_engine_status(&mut self, cx: &mut Context<Self>) {
        let status = cx.global::<AppCore>().call("engineStatus", Vec::new());
        cx.spawn(async move |this, cx| {
            let status = status.await.unwrap_or(Value::Null);
            let _ = this.update(cx, |this, cx| {
                let ready = status["ready"].as_bool() == Some(true);
                if this.broadcasts.engine.ready != ready {
                    this.broadcasts.engine.ready = ready;
                    this.broadcasts.engine.previews.clear();
                }
                this.engine_changed(cx);
            });
        })
        .detach();
    }

    /// The search the broadcasts want now: the open board's, or the next grid board without an
    /// estimate for its position.
    fn engine_request(&self, cx: &App) -> (Option<Value>, Option<(String, String)>) {
        let broadcasts = &self.broadcasts;
        let visible = self.shown && self.tab == super::Tab::Broadcasts;
        if !visible || !broadcasts.engine.ready || !assistance(cx) {
            return (None, None);
        }
        if let Some(game) = broadcasts.current() {
            if !broadcasts.engine_active(cx) || broadcasts.terminal().is_some() {
                return (None, None);
            }
            let history = broadcasts.history();
            let position = view::position_at(&setup_of(game), &history, history.len());
            let fen = position
                .as_ref()
                .map(kchess_domain::rules::make_fen)
                .unwrap_or_default();
            let request = json!({
                "fen": fen,
                "rootFen": game["startFen"],
                "moves": history,
                "lines": crate::prefs::get::<usize>(cx, LINES, 3).clamp(1, 5),
                "infinite": crate::prefs::get(cx, INFINITE, false),
            });
            return (Some(request), None);
        }
        if broadcasts.tour.is_none() || !crate::prefs::get(cx, GRID_EVALUATIONS, true) {
            return (None, None);
        }
        let next = broadcasts.games.iter().find(|game| {
            game["ongoing"] == json!(true)
                && broadcasts
                    .engine
                    .previews
                    .get(game["id"].as_str().unwrap_or_default())
                    .is_none_or(|preview| preview.context != context_of(game))
        });
        let Some(game) = next else {
            return (None, None);
        };
        let request = json!({
            "fen": game["fen"],
            "rootFen": game["startFen"],
            "moves": moves_of(game),
            "lines": 1,
        });
        let previewing = (
            game["id"].as_str().unwrap_or_default().to_string(),
            context_of(game),
        );
        (Some(request), Some(previewing))
    }

    /// Search what the broadcasts want now, after anything that may change it.
    pub(super) fn engine_changed(&mut self, cx: &mut Context<Self>) {
        let (request, previewing) = self.engine_request(cx);
        if previewing != self.broadcasts.engine.previewing {
            self.broadcasts.engine.previewing = previewing.clone();
            // An estimate ends after a while even if the engine never reaches its depth.
            self.broadcasts.engine._preview_done = previewing.map(|_| {
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(PREVIEW_TIME).await;
                    let _ = this.update(cx, |this, cx| this.finish_preview(cx));
                })
            });
        }
        let engine = &mut self.broadcasts.engine.engine;
        engine.search(request, engine_of, cx);
        self.broadcasts.redraw(cx);
        cx.notify();
    }

    /// Keep the estimate of the grid board being estimated, and go on to the next.
    fn finish_preview(&mut self, cx: &mut Context<Self>) {
        let engine = &mut self.broadcasts.engine;
        let Some((id, context)) = engine.previewing.take() else {
            return;
        };
        engine._preview_done = None;
        let line = engine
            .engine
            .update
            .as_ref()
            .filter(|update| update["context"].as_str() == Some(context.as_str()))
            .and_then(|update| update["lines"].as_array()?.first().cloned());
        engine.previews.insert(id, Preview { context, line });
        self.engine_changed(cx);
    }

    /// An `engine:analysis` event.
    pub(super) fn engine_update(&mut self, update: &Value, cx: &mut Context<Self>) {
        if !self.broadcasts.engine.engine.receive(update) {
            return;
        }
        let previewing = self.broadcasts.engine.previewing.clone();
        if let Some((_, context)) = previewing {
            let engine = &self.broadcasts.engine.engine;
            let reached = update["context"].as_str() == Some(context.as_str())
                && (update["depth"].as_f64().unwrap_or_default() >= PREVIEW_DEPTH
                    || update["done"] == json!(true));
            if reached || engine.error.is_some() {
                return self.finish_preview(cx);
            }
        }
        self.broadcasts.redraw(cx);
        cx.notify();
    }

    /// The board's boards changed: estimates for boards gone are forgotten.
    pub(super) fn engine_games_changed(&mut self, cx: &mut Context<Self>) {
        let ids: std::collections::HashSet<String> = self
            .broadcasts
            .games
            .iter()
            .filter_map(|game| game["id"].as_str().map(str::to_string))
            .collect();
        self.broadcasts
            .engine
            .previews
            .retain(|id, _| ids.contains(id));
        self.engine_changed(cx);
    }

    /// L turns the board's engine on and off.
    pub(super) fn toggle_broadcast_engine(&mut self, cx: &mut Context<Self>) -> bool {
        if self.broadcasts.current().is_none() || !self.broadcasts.engine.ready || !assistance(cx) {
            return false;
        }
        let on = crate::prefs::get(cx, ENGINE_ON, false);
        crate::prefs::set(cx, ENGINE_ON, !on);
        self.engine_changed(cx);
        true
    }

    /// Play an engine line's moves over the board, up to the one at `at` (Explore).
    fn explore(&mut self, pv: &[String], at: usize, cx: &mut Context<Self>) {
        if !assistance(cx) {
            return;
        }
        if self.broadcasts.engine.variation_base.is_none() {
            self.broadcasts.engine.variation_base = Some(self.broadcasts.history());
        }
        let end = (at + 1).min(pv.len());
        self.broadcasts
            .engine
            .variation
            .extend_from_slice(&pv[..end]);
        self.engine_changed(cx);
    }

    fn engine_status(&self, cx: &App) -> String {
        let broadcasts = &self.broadcasts;
        if !assistance(cx) {
            return "Analysis paused during your game".into();
        }
        if !broadcasts.engine.ready {
            return "Engine unavailable".into();
        }
        if let Some(error) = &broadcasts.engine.engine.error {
            return error.clone();
        }
        if !crate::prefs::get(cx, ENGINE_ON, false) {
            return "Off".into();
        }
        if let Some(result) = broadcasts.terminal() {
            return if result == "½-½" {
                "Drawn position".into()
            } else {
                "Checkmate".into()
            };
        }
        match &broadcasts.engine.engine.update {
            None => "Starting…".into(),
            Some(update) => format!(
                "Depth {}{}",
                update["depth"].as_f64().unwrap_or_default(),
                if broadcasts.engine.engine.busy {
                    "…"
                } else {
                    ""
                }
            ),
        }
    }

    /// The board's evaluation bar, when the engine follows it and the bar is wanted.
    pub(super) fn broadcast_eval_bar(&self, cx: &App) -> Option<AnyElement> {
        let broadcasts = &self.broadcasts;
        (broadcasts.engine_active(cx) && crate::prefs::get(cx, BAR, true)).then(|| {
            crate::engine_view::eval_bar(
                broadcasts.engine.engine.best(),
                broadcasts.orientation,
                broadcasts.terminal(),
            )
            .into_any_element()
        })
    }

    /// The engine section above the moves (`.broadcast-engine`).
    pub(super) fn broadcast_engine_panel(
        &self,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> impl IntoElement {
        let broadcasts = &self.broadcasts;
        let active = broadcasts.engine_active(cx);
        let allowed = assistance(cx) && broadcasts.engine.ready;
        let on = crate::prefs::get(cx, ENGINE_ON, false);
        let terminal = broadcasts.terminal();
        let score = if active {
            terminal
                .map(str::to_string)
                .unwrap_or_else(|| format_eval(broadcasts.engine.engine.best()))
        } else {
            "—".into()
        };
        let lines_select = broadcasts.engine.lines_select.clone();
        let page = cx.entity().downgrade();
        let settings = Popover::new("broadcast-engine-settings")
            .anchor(Anchor::TopRight)
            .trigger(
                Button::new("broadcast-engine-settings-button")
                    .ghost()
                    .xsmall()
                    .icon(icon("settings-2"))
                    .accessibility_label("Broadcast analysis settings"),
            )
            .content(move |_, _, cx| {
                let toggle = |id: &'static str, key: &'static str, label: &'static str, default| {
                    let page = page.clone();
                    h_flex()
                        .gap(px(10.))
                        .child(
                            Switch::new(id)
                                .small()
                                .checked(crate::prefs::get(cx, key, default))
                                .accessibility_label(label)
                                .on_click(move |on, _, cx| {
                                    crate::prefs::set(cx, key, *on);
                                    let _ = page.update(cx, |page, cx| page.engine_changed(cx));
                                }),
                        )
                        .child(div().text_size(px(14.)).child(label))
                };
                v_flex()
                    .w(px(240.))
                    .gap(px(12.))
                    .p(px(4.))
                    .child(
                        h_flex()
                            .justify_between()
                            .child(div().text_size(px(14.)).child("Lines"))
                            .child(
                                div().w(px(96.)).child(
                                    Select::new(&lines_select)
                                        .small()
                                        .accessibility_label("Broadcast engine lines"),
                                ),
                            ),
                    )
                    .child(toggle("broadcast-bar", BAR, "Evaluation bar", true))
                    .child(toggle("broadcast-arrows", ARROWS, "Best move arrows", true))
                    .child(toggle(
                        "broadcast-infinite",
                        INFINITE,
                        "Infinite analysis",
                        false,
                    ))
            });
        let error = broadcasts.engine.engine.error.is_some();
        let head = h_flex()
            .gap(px(8.))
            .child(
                Switch::new("broadcast-engine")
                    .checked(on && allowed)
                    .disabled(!allowed)
                    .accessibility_label("Broadcast engine analysis")
                    .tooltip("Toggle engine (L)")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_broadcast_engine(cx);
                    })),
            )
            .child(
                div()
                    .min_w(px(60.))
                    .text_size(px(20.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(if active { palette.text } else { palette.dimmed })
                    .child(score),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Stockfish"),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .truncate()
                            .text_color(if error {
                                palette.error
                            } else {
                                palette.text_muted
                            })
                            .child(self.engine_status(cx)),
                    ),
            )
            .child(settings);
        let page = cx.entity().downgrade();
        v_flex()
            .id("broadcast-engine")
            .flex_none()
            .gap(px(10.))
            .aria_label("Broadcast engine analysis")
            .child(head)
            .when(error && active, |panel| {
                panel.child(
                    Button::new("broadcast-engine-retry")
                        .outline()
                        .small()
                        .icon(icon("refresh-cw"))
                        .label("Retry engine")
                        .on_click(cx.listener(|this, _, _, cx| {
                            let request = this.broadcasts.engine.engine.wanted().cloned();
                            let engine = &mut this.broadcasts.engine.engine;
                            engine.restart(request, engine_of, cx);
                            cx.notify();
                        })),
                )
            })
            .when(active, |panel| {
                let fen = {
                    let history = broadcasts.history();
                    broadcasts
                        .current()
                        .and_then(|game| {
                            view::position_at(&setup_of(game), &history, history.len())
                        })
                        .as_ref()
                        .map(kchess_domain::rules::make_fen)
                        .unwrap_or_default()
                };
                panel.child(crate::engine_view::engine_lines(
                    &fen,
                    &broadcasts.engine.engine.lines(),
                    crate::prefs::get::<usize>(cx, LINES, 3).clamp(1, 5),
                    terminal.map(|_| "Nothing to analyse."),
                    std::rc::Rc::new(move |pv: &[String], at, _: &mut Window, cx: &mut App| {
                        let _ = page.update(cx, |page, cx| page.explore(pv, at, cx));
                    }),
                    palette,
                ))
            })
            .when(broadcasts.engine.variation_base.is_some(), |panel| {
                panel.child(
                    Button::new("broadcast-return")
                        .small()
                        .icon(icon("undo-2"))
                        .label("Return to game")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.broadcasts.return_to_game();
                            this.engine_changed(cx);
                        })),
                )
            })
    }
}
