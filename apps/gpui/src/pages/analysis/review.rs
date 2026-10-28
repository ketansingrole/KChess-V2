//! The analysis board's game review (`AnalysisReview.vue`, `EvalChart.vue` and the review parts of
//! `stores/analysis.ts`): both sides' accuracy, labelled moves, centipawn loss and accuracy by
//! phase, and the evaluation chart. Reviews are the core's (`reviewRequest`); the board shows the
//! review of its main line, or of the game before moves were added to it.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_core::Level;
use serde_json::{Value, json};
use shakmaty::Color;

use super::{AnalysisPage, TOOL, stop_analysis};
use crate::assets::icon;
use crate::core::AppCore;
use crate::game::rules;
use crate::game::tree::{moves_of, path_of};
use crate::reviews::ReviewHandle;
use crate::theme::{Palette, judgment_color};
use crate::widgets;

/// What a move's label looks like (`MOVE_CLASSIFICATIONS`): its name, glyph and colour.
/// The NAG of a review's judgment: ?! dubious, ? mistake, ?? blunder.
fn tried_nag(judgment: &str) -> Option<i64> {
    match judgment {
        "inaccuracy" => Some(6),
        "mistake" => Some(2),
        "blunder" => Some(4),
        _ => None,
    }
}

pub fn classification_look(classification: &str, dark: bool) -> (&'static str, &'static str, Hsla) {
    let fixed = |hex: u32| -> Hsla { rgb(hex).into() };
    match classification {
        "brilliant" => ("Brilliant", "!!", fixed(0x168c9c)),
        "best" => ("Best", "★", fixed(0x538425)),
        "excellent" => ("Excellent", "✓", fixed(0x658443)),
        "good" => ("Good", "•", fixed(0x697f51)),
        "inaccuracy" => ("Inaccuracy", "?!", judgment_color("inaccuracy", dark)),
        "mistake" => ("Mistake", "?", judgment_color("mistake", dark)),
        _ => ("Blunder", "??", judgment_color("blunder", dark)),
    }
}

/// `analyseReview` of a review, and `reviewClassifications`: each move's label.
pub type Analysed = (Value, Vec<Option<String>>);
/// The evaluation chart: White's chances per position (the start first), and each move's label.
pub type Chart = (Vec<Option<f64>>, Vec<Option<String>>);

/// What the review says about one main-line move (`ReviewMark`).
#[derive(Clone)]
pub struct ReviewMark {
    /// The move's index in the game (0 is the first move).
    pub index: usize,
    pub color: Color,
    pub classification: Option<String>,
}

/// The review state of the board.
#[derive(Default)]
pub struct ReviewState {
    /// Where the game on the board came from (`GameOrigin`): its players, your side, its Lichess
    /// game. Nothing for a game set up or typed here.
    pub origin: Option<Value>,
    /// The review shown: of the main line, or of the game before moves were added to it.
    pub shown: String,
    /// The main line it was last looked up for.
    looked_up: String,
    /// Why the review asked for could not even be queued.
    pub request_error: Option<String>,
    /// Asked for in this visit and not started yet: another review is ahead of it.
    requested: String,
    /// The review whose player's mistakes were saved for practice.
    saved_mistakes: String,
    /// Sides asked for that had nothing to add, by review.
    tried: HashSet<String>,
    pub practice_feedback: Option<String>,
    /// The positions in practice (`MistakeExercise`), as the core keeps them.
    pub mistakes: Vec<Value>,
    /// The last `analyseReview` and `reviewClassifications`, by review and its last update.
    analysed: RefCell<Option<(String, f64, Analysed)>>,
}

impl ReviewState {
    pub fn new(mistakes: Vec<Value>) -> ReviewState {
        ReviewState {
            mistakes,
            ..ReviewState::default()
        }
    }
}

impl AnalysisPage {
    /// The main line's moves.
    pub(super) fn mainline(&self) -> Vec<String> {
        moves_of(&self.root.line_end(""))
    }

    /// `reviewKey`: the same moves from the same start share one review.
    pub fn mainline_key(&self) -> String {
        rules("reviewKey", &[json!(self.root.fen), json!(self.mainline())])
            .ok()
            .and_then(|key| key.as_str().map(str::to_string))
            .unwrap_or_default()
    }

    pub(super) fn review(&self, cx: &App) -> Option<Value> {
        cx.global::<ReviewHandle>()
            .0
            .read(cx)
            .get(&self.review.shown)
            .cloned()
    }

    /// Follow the main line's review: the cached one, else the stored one, once looked up.
    pub(super) fn follow_review(&mut self, cx: &mut Context<Self>) {
        let key = self.mainline_key();
        let store = cx.global::<ReviewHandle>().0.clone();
        if store.read(cx).get(&key).is_some() {
            self.review.shown = key.clone();
        }
        if key != self.review.looked_up {
            self.review.looked_up = key.clone();
            self.review.request_error = None;
            self.review.requested.clear();
            let moves = self.mainline();
            if store.read(cx).get(&key).is_none() && !moves.is_empty() {
                let found = cx
                    .global::<AppCore>()
                    .call("reviewGet", vec![json!(self.root.fen), json!(moves)]);
                let store = store.clone();
                cx.spawn(async move |this, cx| match found.await {
                    Ok(review) if !review.is_null() => {
                        let _ = this.update(cx, |this, cx| {
                            store.update(cx, |store, cx| {
                                store.cache.remember(review.clone());
                                cx.notify();
                            });
                            if this.mainline_key() == key {
                                this.review.shown =
                                    review["key"].as_str().unwrap_or_default().to_string();
                                this.reviewed(cx);
                            }
                        });
                    }
                    Ok(_) => {}
                    Err(cause) => crate::log::write(
                        Level::Warn,
                        "review",
                        &format!("loading review failed: {}", cause.message),
                    ),
                })
                .detach();
            }
        }
        let shown = self.review.shown.clone();
        store.update(cx, |store, _| store.pin(&shown));
        self.save_player_mistakes(cx);
    }

    /// The review changed (it streamed in, or another was found): the board's labels follow.
    pub(super) fn reviewed(&mut self, cx: &mut Context<Self>) {
        self.save_player_mistakes(cx);
        self.redraw(cx);
    }

    /// How many main-line moves the review covers (it may be of a shorter or diverging line).
    pub(super) fn reviewed_plies(&self, review: &Value) -> usize {
        let mainline = self.mainline();
        review["moves"].as_array().map_or(0, |moves| {
            moves
                .iter()
                .zip(&mainline)
                .take_while(|(reviewed, played)| reviewed.as_str() == Some(played.as_str()))
                .count()
        })
    }

    /// The summary applies: the review is of this main line, or of the start of it.
    pub(super) fn review_whole(&self, review: &Value) -> bool {
        self.reviewed_plies(review) == review["moves"].as_array().map_or(0, Vec::len)
    }

    /// `analyseReview` and `reviewClassifications` of the review shown, worked out once per update.
    pub(super) fn game_analysis(&self, cx: &App) -> Option<Analysed> {
        let review = self.review(cx)?;
        if !Self::assistance_allowed(cx) || review["fen"] != json!(self.root.fen) {
            return None;
        }
        let key = review["key"].as_str().unwrap_or_default().to_string();
        let updated = review["updatedAt"].as_f64().unwrap_or(0.);
        let mut memo = self.review.analysed.borrow_mut();
        if let Some((known, at, analysed)) = memo.as_ref()
            && *known == key
            && *at == updated
        {
            return Some(analysed.clone());
        }
        let analysis = rules("analyseReview", std::slice::from_ref(&review)).ok()?;
        if analysis.is_null() {
            return None;
        }
        let classes: Vec<Option<String>> = rules("reviewClassifications", &[review])
            .ok()
            .and_then(|classes| serde_json::from_value(classes).ok())
            .unwrap_or_default();
        *memo = Some((key, updated, (analysis.clone(), classes.clone())));
        Some((analysis, classes))
    }

    /// The review's verdict on each main-line move, by path.
    pub(super) fn review_marks(&self, cx: &App) -> HashMap<String, ReviewMark> {
        let mut marks = HashMap::new();
        let (Some(review), Some((analysis, classes))) = (self.review(cx), self.game_analysis(cx))
        else {
            return marks;
        };
        let mainline = self.mainline();
        for index in 0..self.reviewed_plies(&review) {
            let mover = &analysis["moves"][index];
            if mover.is_null() {
                break;
            }
            marks.insert(
                path_of(&mainline[..=index]),
                ReviewMark {
                    index,
                    color: if mover["color"] == json!("black") {
                        Color::Black
                    } else {
                        Color::White
                    },
                    classification: classes.get(index).cloned().flatten(),
                },
            );
        }
        marks
    }

    /// The evaluation chart: White's chances after each reviewed move (the start first), and its
    /// labels.
    pub(super) fn review_chart(&self, cx: &App) -> Option<Chart> {
        let review = self.review(cx)?;
        let (analysis, _) = self.game_analysis(cx)?;
        let plies = self.reviewed_plies(&review);
        if plies == 0 {
            return None;
        }
        let moves: Vec<&Value> = analysis["moves"].as_array()?.iter().take(plies).collect();
        let points = std::iter::once(analysis["startChances"].as_f64())
            .chain(moves.iter().map(|mover| mover["chances"].as_f64()))
            .collect();
        let judgments = moves
            .iter()
            .map(|mover| mover["judgment"].as_str().map(str::to_string))
            .collect();
        Some((points, judgments))
    }

    /// The board's position along the reviewed main line (0 is the start), or none off it.
    pub(super) fn review_ply(&self, reviewed: usize) -> Option<usize> {
        let moves = moves_of(&self.path);
        if moves.len() > reviewed {
            return None;
        }
        let mainline = self.mainline();
        moves
            .iter()
            .zip(&mainline)
            .all(|(shown, main)| shown == main)
            .then_some(moves.len())
    }

    /// The main-line position after `ply` moves.
    pub(super) fn go_to_ply(&mut self, ply: usize, cx: &mut Context<Self>) {
        let mainline = self.mainline();
        let path = path_of(&mainline[..ply.min(mainline.len())]);
        self.go_to(&path, cx);
    }

    /// The review queue is working on this game.
    pub(super) fn reviewing(&self, cx: &App) -> Option<Value> {
        cx.global::<ReviewHandle>()
            .0
            .read(cx)
            .progress(&self.mainline_key())
            .cloned()
    }

    /// Review the main line now (`requestReview`); the engine makes way for it.
    pub fn request_review(&mut self, cx: &mut Context<Self>) {
        crate::prefs::set(cx, TOOL, "review");
        self.review.request_error = None;
        let moves = self.mainline();
        if moves.is_empty() {
            return;
        }
        self.review.requested = self.mainline_key();
        self.wanted.clear();
        stop_analysis(false, cx);
        let origin = self.review.origin.clone().unwrap_or(Value::Null);
        let mut request = json!({ "fen": self.root.fen, "moves": moves });
        if let Some(id) = origin["gameId"].as_str() {
            request["gameId"] = json!(id);
            request["account"] = origin["account"].clone();
        }
        let asked = cx.global::<AppCore>().call("reviewRequest", vec![request]);
        let fallback = self.mainline_key();
        let store = cx.global::<ReviewHandle>().0.clone();
        cx.spawn(async move |this, cx| {
            let asked = asked.await;
            let _ = this.update(cx, |this, cx| {
                match asked {
                    Ok(stored) => {
                        let key = stored["key"].as_str().map(str::to_string);
                        store.update(cx, |store, cx| {
                            store.cache.remember(stored);
                            cx.notify();
                        });
                        this.review.shown = key.unwrap_or(fallback);
                    }
                    Err(cause) => {
                        crate::log::write(
                            Level::Warn,
                            "analysis",
                            &format!("Review request failed: {}", cause.message),
                        );
                        this.review.requested.clear();
                        this.review.request_error = Some(cause.message);
                    }
                }
                this.reviewed(cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn cancel_review(&mut self, cx: &mut Context<Self>) {
        self.review.requested.clear();
        let cancel = cx
            .global::<AppCore>()
            .call("reviewCancel", vec![json!(self.mainline_key())]);
        cx.background_executor()
            .spawn(async move {
                if let Err(cause) = cancel.await {
                    crate::log::write(
                        Level::Warn,
                        "review",
                        &format!("cancelling review failed: {}", cause.message),
                    );
                }
            })
            .detach();
        cx.notify();
    }

    /// Your mistakes in a reviewed game you played are saved for practice once its review is done.
    fn save_player_mistakes(&mut self, cx: &mut Context<Self>) {
        let Some(review) = self.review(cx) else {
            return;
        };
        let Some(player) = self
            .review
            .origin
            .as_ref()
            .and_then(|origin| origin["player"].as_str().map(str::to_string))
        else {
            return;
        };
        let key = review["key"].as_str().unwrap_or_default().to_string();
        if review["complete"] != json!(true)
            || !self.review_whole(&review)
            || self.review.saved_mistakes == key
        {
            return;
        }
        self.review.saved_mistakes = key.clone();
        let added = cx
            .global::<AppCore>()
            .call("addMistakes", vec![json!(key), json!(player)]);
        cx.spawn(async move |this, cx| {
            let added = added.await;
            let _ = this.update(cx, |this, cx| match added {
                Ok(result) => {
                    this.review.mistakes = result["items"].as_array().cloned().unwrap_or_default();
                    cx.notify();
                }
                Err(cause) => {
                    crate::log::write(
                        Level::Warn,
                        "analysis",
                        &format!("Saving your mistakes failed: {}", cause.message),
                    );
                    this.review.saved_mistakes.clear();
                }
            });
        })
        .detach();
    }

    /// This review's positions already in practice, and the sides they are from.
    fn saved_positions(&self, key: &str) -> (usize, HashSet<&'static str>) {
        let prefix = format!("{key}:");
        let items: Vec<&Value> = self
            .review
            .mistakes
            .iter()
            .filter(|item| {
                item["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with(&prefix))
            })
            .collect();
        let sides = items
            .iter()
            .map(|item| {
                if item["fen"].as_str().and_then(|fen| fen.split(' ').nth(1)) == Some("b") {
                    "black"
                } else {
                    "white"
                }
            })
            .collect();
        (items.len(), sides)
    }

    /// The sides whose labelled moves the review puts into practice: yours, or both in a game you
    /// did not play.
    fn practice_sides(&self, analysis: &Value) -> Vec<&'static str> {
        let player = self
            .review
            .origin
            .as_ref()
            .and_then(|origin| origin["player"].as_str())
            .map(|player| if player == "black" { "black" } else { "white" });
        let sides: Vec<&'static str> = match player {
            Some(player) => vec![player],
            None => vec!["white", "black"],
        };
        sides
            .into_iter()
            .filter(|color| {
                let side = &analysis[*color];
                ["inaccuracy", "mistake", "blunder"]
                    .iter()
                    .any(|kind| side[*kind].as_f64().unwrap_or(0.) > 0.)
            })
            .collect()
    }

    /// Save whichever of those sides is not in practice yet, then practise them on the board.
    fn practice_mistakes(&mut self, cx: &mut Context<Self>) {
        let (Some(review), Some((analysis, _))) = (self.review(cx), self.game_analysis(cx)) else {
            return;
        };
        let key = review["key"].as_str().unwrap_or_default().to_string();
        self.review.practice_feedback = None;
        let (_, saved) = self.saved_positions(&key);
        let sides: Vec<&'static str> = self
            .practice_sides(&analysis)
            .into_iter()
            .filter(|color| {
                !saved.contains(color) && !self.review.tried.contains(&format!("{key}:{color}"))
            })
            .collect();
        let core = cx.global::<AppCore>().clone();
        cx.spawn(async move |this, cx| {
            let mut failure = None;
            for color in sides {
                match core
                    .call("addMistakes", vec![json!(key), json!(color)])
                    .await
                {
                    Ok(result) => {
                        let added = result["added"].as_f64().unwrap_or(0.);
                        let items = result["items"].as_array().cloned().unwrap_or_default();
                        let _ = this.update(cx, |this, _| {
                            this.review.mistakes = items;
                            if added == 0. {
                                this.review.tried.insert(format!("{key}:{color}"));
                            }
                        });
                    }
                    Err(cause) => {
                        failure = Some(cause.message);
                        break;
                    }
                }
            }
            let _ = this.update(cx, |this, cx| {
                if let Some(cause) = failure {
                    crate::log::write(
                        Level::Warn,
                        "analysis-review",
                        &format!("Adding mistakes failed: {cause}"),
                    );
                    this.review.practice_feedback = Some(cause);
                } else if this.saved_positions(&key).0 == 0 {
                    this.review.practice_feedback =
                        Some("No verified mistakes to practice.".into());
                } else {
                    this.start_practice(&key, cx);
                    // The move list beside the practice card shows the moves around each position.
                    crate::prefs::set(cx, TOOL, "moves");
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Open a game on the board (`loadPgn`) at a move, facing as it was played, with where it came
    /// from, and review it when asked (`startReview`).
    pub fn open_game(
        &mut self,
        pgn: &str,
        ply: usize,
        orientation: Option<Color>,
        origin: Option<Value>,
        review: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.load_pgn(pgn, cx) {
            return false;
        }
        if let Some(orientation) = orientation {
            self.orientation = orientation;
        }
        self.review.origin = origin;
        self.go_to_ply(ply, cx);
        if review {
            self.request_review(cx);
        }
        true
    }

    /// Study a puzzle (`usePuzzleAnalysis().open`): its solution is the line, each move tried
    /// instead in that attempt a variation beside it marked "Your move", and the solution is
    /// reviewed like a game.
    pub fn open_puzzle(
        &mut self,
        puzzle: &Value,
        wrong_moves: &[Value],
        cx: &mut Context<Self>,
    ) -> bool {
        let solution: Vec<String> =
            serde_json::from_value(puzzle["solution"].clone()).unwrap_or_default();
        let Ok(Value::String(pgn)) = crate::game::rules(
            "setupPgn",
            &[
                serde_json::json!({ "variant": "standard", "fen": puzzle["fen"] }),
                serde_json::json!(solution),
                serde_json::json!({}),
            ],
        ) else {
            return false;
        };
        if !self.load_pgn(&pgn, cx) {
            return false;
        }
        self.tried_labels.clear();
        self.tried_done.clear();
        let mut tried = Vec::new();
        for wrong in wrong_moves {
            let index = crate::game::whole(&wrong["index"]) as usize;
            let Some(uci) = wrong["uci"].as_str() else {
                continue;
            };
            if index > solution.len() || solution.get(index).map(String::as_str) == Some(uci) {
                continue;
            }
            let before = crate::game::tree::path_of(&solution[..index]);
            let fen = self.root.node_at(&before).fen.clone();
            if let Some(path) = self.root.add_move(&before, uci) {
                self.root.node_at_mut(&path).comments = Some(vec!["Your move".into()]);
                tried.push((path, fen, uci.to_string()));
            }
        }
        self.orientation = crate::game::puzzle::OnBoard::start(puzzle).player_color();
        self.review.origin = None;
        self.go_to_ply(0, cx);
        self.request_review(cx);
        self.changed(cx);
        let start = self.root.fen.clone();
        for (path, fen, uci) in tried {
            self.label_tried(start.clone(), path, fen, uci, cx);
        }
        true
    }

    /// Judge a move tried instead of the puzzle's solution from the position before it, and label
    /// its variation (?!, ?, ??) once that review is done. Without an engine the move stays marked
    /// as yours, unlabelled.
    fn label_tried(
        &mut self,
        start: String,
        path: String,
        fen: String,
        uci: String,
        cx: &mut Context<Self>,
    ) {
        let asked = cx
            .global::<AppCore>()
            .call("reviewRequest", vec![json!({ "fen": fen, "moves": [uci] })]);
        let store = cx.global::<ReviewHandle>().0.clone();
        cx.spawn(async move |this, cx| {
            let stored = match asked.await {
                Ok(stored) => stored,
                Err(cause) => {
                    crate::log::write(
                        Level::Warn,
                        "puzzle-analysis",
                        &format!("Judging a tried move failed: {}", cause.message),
                    );
                    return;
                }
            };
            let _ = this.update(cx, |this, cx| {
                if this.apply_tried(&start, &path, &stored, cx) {
                    return;
                }
                let key = stored["key"].as_str().unwrap_or_default().to_string();
                store.update(cx, |store, cx| {
                    store.cache.remember(stored);
                    cx.notify();
                });
                let watching = cx.observe(&store, move |this, store, cx| {
                    let Some(review) = store.read(cx).get(&key).cloned() else {
                        return;
                    };
                    // Once labelled, the move is done; its subscription goes with the next puzzle.
                    this.apply_tried(&start, &path, &review, cx);
                });
                this.tried_labels.push(watching);
            });
        })
        .detach();
    }

    /// Label the tried move at `path` from its completed review; false while it is not complete.
    fn apply_tried(
        &mut self,
        start: &str,
        path: &str,
        review: &Value,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.tried_done.iter().any(|done| done == path) {
            return true;
        }
        if review["complete"] != json!(true) {
            return false;
        }
        self.tried_done.push(path.to_string());
        let judgment = rules("analyseReview", std::slice::from_ref(review))
            .ok()
            .and_then(|analysis| {
                analysis["moves"][0]["judgment"]
                    .as_str()
                    .map(str::to_string)
            });
        // The board still holds this puzzle and the variation was not deleted.
        let held =
            self.root.fen == start && self.root.nodes_along(path).len() == moves_of(path).len() + 1;
        if let (Some(nag), true) = (judgment.as_deref().and_then(tried_nag), held) {
            let node = self.root.node_at_mut(path);
            node.nags = super::notes::with_glyph(node.nags.as_deref(), Some(nag));
            self.redraw(cx);
            self.schedule_save(cx);
            cx.notify();
        }
        true
    }

    /// Practise a reviewed game's positions on the board (`usePracticeGame`): `game` is a
    /// `MistakeGame`, `items` the positions in practice. A game already on the board is kept as
    /// it is, variations and all.
    pub fn practise_game(
        &mut self,
        game: &Value,
        items: Vec<Value>,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = game["key"].as_str().unwrap_or_default().to_string();
        if self.mainline_key() != key {
            let mut headers = serde_json::Map::new();
            for (field, header) in [("white", "White"), ("black", "Black")] {
                if let Some(name) = game[field].as_str() {
                    headers.insert(header.into(), json!(name));
                }
            }
            let pgn = rules(
                "setupPgn",
                &[
                    json!({ "variant": "standard", "fen": game["fen"] }),
                    game["moves"].clone(),
                    Value::Object(headers),
                ],
            )
            .ok()
            .and_then(|pgn| pgn.as_str().map(str::to_string));
            let Some(pgn) = pgn else {
                return false;
            };
            if !self.load_pgn(&pgn, cx) {
                return false;
            }
            self.review.origin = Some(json!({
                "gameId": game["gameId"],
                "account": game["account"],
                "white": game["white"],
                "black": game["black"],
                "player": game["player"],
            }));
        }
        self.review.mistakes = items;
        self.start_practice(&key, cx);
        crate::prefs::set(cx, TOOL, "moves");
        true
    }

    /// The players' names: from where the game came from, or else its PGN headers.
    pub(super) fn player_name(&self, color: Color) -> String {
        let key = if color == Color::White {
            "white"
        } else {
            "black"
        };
        self.review
            .origin
            .as_ref()
            .and_then(|origin| origin[key].as_str())
            .map(str::to_string)
            .or_else(|| {
                let header = self.header(if color == Color::White {
                    "White"
                } else {
                    "Black"
                });
                (!header.is_empty()).then_some(header)
            })
            .unwrap_or_else(|| {
                if color == Color::White {
                    "White"
                } else {
                    "Black"
                }
                .into()
            })
    }

    /// Jump to the next move of `color` with this label, after the one on the board (wrapping).
    fn next_labelled(&mut self, color: Color, label: &str, cx: &mut Context<Self>) {
        let marks = self.review_marks(cx);
        let mut indices: Vec<usize> = marks
            .values()
            .filter(|mark| mark.color == color && mark.classification.as_deref() == Some(label))
            .map(|mark| mark.index)
            .collect();
        indices.sort_unstable();
        let from = self
            .review(cx)
            .and_then(|review| self.review_ply(self.reviewed_plies(&review)))
            .unwrap_or(0);
        if let Some(index) = indices
            .iter()
            .find(|index| **index + 1 > from)
            .or(indices.first())
        {
            self.go_to_ply(index + 1, cx);
        }
    }

    /// The Review tab (`AnalysisReview.vue`).
    pub(super) fn render_review(&self, cx: &mut Context<Self>, palette: &Palette) -> AnyElement {
        let key = self.mainline_key();
        let review = self.review(cx);
        let store = cx.global::<ReviewHandle>().0.read(cx);
        let progress = store.progress(&key).cloned();
        let failure = store
            .failure(&key)
            .or_else(|| self.review.request_error.clone());
        let complete = review
            .as_ref()
            .is_some_and(|review| review["complete"] == json!(true) && self.review_whole(review));
        let queued =
            self.review.requested == key && progress.is_none() && !complete && failure.is_none();
        // A Lichess game can be reviewed from Lichess's analysis even without a local engine.
        let can_review = self.engine_ready
            || self
                .review
                .origin
                .as_ref()
                .is_some_and(|origin| origin["gameId"].is_string());
        let reviewed = review
            .as_ref()
            .map_or(0, |review| self.reviewed_plies(review));
        let source = match &review {
            Some(review) if reviewed > 0 => {
                if review["source"] == json!("lichess") {
                    Some("From Lichess")
                } else if review["complete"] != json!(true) && progress.is_none() {
                    Some("Partly reviewed")
                } else {
                    None
                }
            }
            _ => None,
        };
        let head = h_flex()
            .gap(px(8.))
            .min_h(px(24.))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::BOLD)
                    .child("Game review"),
            )
            .children(source.map(|source| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child(source)
            }))
            .child(div().flex_1())
            .when(progress.is_some() || queued, |head| {
                head.child(
                    Button::new("review-stop")
                        .ghost()
                        .xsmall()
                        .icon(icon("square"))
                        .label("Stop")
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_review(cx))),
                )
            })
            .when(progress.is_none() && !queued && !complete, |head| {
                head.child(
                    Button::new("review-start")
                        .primary()
                        .xsmall()
                        .icon(icon("sparkles"))
                        .label(if review.is_some() && reviewed > 0 {
                            "Finish review"
                        } else {
                            "Review game"
                        })
                        .tooltip(if can_review {
                            "Label each move and score both sides"
                        } else {
                            "Stockfish is needed"
                        })
                        .disabled(!can_review)
                        .on_click(cx.listener(|this, _, _, cx| this.request_review(cx))),
                )
            });
        let note = |text: String, error: bool| {
            div()
                .text_size(px(12.))
                .text_color(if error {
                    palette.error
                } else {
                    palette.text_muted
                })
                .child(text)
        };
        let progress_row = progress.as_ref().map(|current| {
            let done = current["done"].as_f64().unwrap_or(0.);
            let total = current["total"].as_f64().unwrap_or(1.).max(1.);
            h_flex()
                .gap(px(8.))
                .text_size(px(11.))
                .text_color(palette.text_muted)
                .child("Reviewing…")
                .child(
                    div()
                        .flex_1()
                        .h(px(6.))
                        .rounded_full()
                        .bg(palette.accented)
                        .child(
                            div()
                                .h_full()
                                .rounded_full()
                                .bg(palette.primary)
                                .w(relative((done / total).clamp(0., 1.) as f32)),
                        ),
                )
        });
        let analysis = self.game_analysis(cx);
        let marks = self.review_marks(cx);
        let whole = review
            .as_ref()
            .is_some_and(|review| self.review_whole(review));
        let table = analysis.as_ref().filter(|_| whole).map(|(analysis, _)| {
            self.render_review_table(
                &review.clone().unwrap_or_default(),
                analysis,
                &marks,
                cx,
                palette,
            )
        });
        let can_practice = complete
            && analysis.as_ref().is_some_and(|(analysis, _)| {
                let (count, _) = self.saved_positions(&key);
                count > 0
                    || self
                        .practice_sides(analysis)
                        .iter()
                        .any(|color| !self.review.tried.contains(&format!("{key}:{color}")))
            })
            && self.practice.is_none();
        let chart = self.review_chart(cx).map(|(points, judgments)| {
            let current = review
                .as_ref()
                .and_then(|review| self.review_ply(self.reviewed_plies(review)));
            eval_chart(points, judgments, current, cx.entity().downgrade(), palette)
        });
        v_flex()
            .gap(px(8.))
            .child(head)
            .children(progress_row)
            .when(progress.is_none() && queued, |panel| {
                panel.child(note("Waiting for another review to finish…".into(), false))
            })
            .when(progress.is_none() && !queued, |panel| {
                panel.children(failure.clone().map(|failure| note(failure, true)))
            })
            .children(table)
            .when(can_practice, |panel| {
                panel.child(
                    Button::new("practice-review-mistakes")
                        .outline()
                        .w_full()
                        .icon(icon("target"))
                        .label(
                            if self
                                .review
                                .origin
                                .as_ref()
                                .is_some_and(|origin| origin["player"].is_string())
                            {
                                "Practice your mistakes"
                            } else {
                                "Practice mistakes"
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.practice_mistakes(cx))),
                )
            })
            .children(
                self.review
                    .practice_feedback
                    .clone()
                    .map(|text| note(text, false)),
            )
            .when(self.practice.is_none(), |panel| panel.children(chart))
            .when(review.is_none() && !can_review, |panel| {
                panel.child(note(
                    "Set up Stockfish in Settings to review games.".into(),
                    false,
                ))
            })
            .into_any_element()
    }

    fn render_review_table(
        &self,
        review: &Value,
        analysis: &Value,
        marks: &HashMap<String, ReviewMark>,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let sides = [Color::White, Color::Black];
        let side_of = |color: Color| {
            &analysis[if color == Color::White {
                "white"
            } else {
                "black"
            }]
        };
        let brilliant = |color: Color| {
            marks
                .values()
                .filter(|mark| {
                    mark.color == color && mark.classification.as_deref() == Some("brilliant")
                })
                .count()
        };
        let percent =
            |value: Option<f64>| value.map_or_else(|| "—".to_string(), |value| format!("{value}%"));
        let label = |text: String, color: Option<Hsla>| {
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(color.unwrap_or(palette.text_muted))
                .child(text)
        };
        let cell = |content: AnyElement| {
            div()
                .w(relative(0.33))
                .flex()
                .justify_center()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(content)
        };
        let names = h_flex()
            .items_end()
            .child(div().flex_1())
            .children(sides.map(|color| {
                cell(
                    v_flex()
                        .items_center()
                        .gap(px(3.))
                        .max_w_full()
                        .child(widgets::side_dot(color))
                        .child(
                            div()
                                .max_w_full()
                                .truncate()
                                .text_size(px(12.5))
                                .child(self.player_name(color)),
                        )
                        .into_any_element(),
                )
            }));
        let accuracy = h_flex()
            .pb(px(6.))
            .child(label("Accuracy".into(), Some(palette.text)))
            .children(sides.map(|color| {
                cell(
                    div()
                        .text_size(px(16.))
                        .font_weight(FontWeight::BOLD)
                        .child(percent(side_of(color)["accuracy"].as_f64()))
                        .into_any_element(),
                )
            }));
        let dark = palette.dark;
        let kinds: Vec<(&'static str, &'static str)> = [
            ("brilliant", "Brilliant"),
            ("inaccuracy", "Inaccuracies"),
            ("mistake", "Mistakes"),
            ("blunder", "Blunders"),
        ]
        .into_iter()
        // Brilliant moves are rare: their row shows only when there is one.
        .filter(|(kind, _)| *kind != "brilliant" || sides.iter().any(|color| brilliant(*color) > 0))
        .collect();
        let counts = kinds.into_iter().map(|(kind, title)| {
            let (_, _, color) = classification_look(kind, dark);
            h_flex()
                .child(label(title.into(), Some(color)))
                .children(sides.map(|side| {
                    let count = if kind == "brilliant" {
                        brilliant(side)
                    } else {
                        side_of(side)[kind].as_f64().unwrap_or(0.) as usize
                    };
                    cell(
                        div()
                            .id(SharedString::from(format!("count-{kind}-{side:?}")))
                            .min_w(px(30.))
                            .px(px(6.))
                            .py(px(1.))
                            .rounded(px(5.))
                            .flex()
                            .justify_center()
                            .when(count > 0, |button| {
                                button
                                    .text_color(color)
                                    .cursor_pointer()
                                    .hover(|button| button.bg(palette.accented))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.next_labelled(side, kind, cx)
                                    }))
                            })
                            .when(count == 0, |button| {
                                button
                                    .text_color(palette.dimmed)
                                    .font_weight(FontWeight::NORMAL)
                            })
                            .child(count.to_string())
                            .into_any_element(),
                    )
                }))
        });
        let acpl = h_flex()
            .pb(px(6.))
            .child(label("Avg. centipawn loss".into(), None))
            .children(sides.map(|color| {
                cell(
                    div()
                        .child(
                            side_of(color)["acpl"]
                                .as_f64()
                                .map_or_else(|| "—".to_string(), |acpl| acpl.to_string()),
                        )
                        .into_any_element(),
                )
            }));
        let phases = rules("reviewPhases", std::slice::from_ref(review)).unwrap_or(Value::Null);
        let phase_rows: Vec<AnyElement> = [
            ("opening", "Opening"),
            ("middlegame", "Middlegame"),
            ("endgame", "Endgame"),
        ]
        .into_iter()
        .filter(|(key, _)| !phases["white"][key].is_null() || !phases["black"][key].is_null())
        .map(|(key, title)| {
            h_flex()
                .child(label(title.into(), None))
                .children(sides.map(|color| {
                    let side = if color == Color::White {
                        "white"
                    } else {
                        "black"
                    };
                    cell(
                        div()
                            .child(percent(phases[side][key].as_f64()))
                            .into_any_element(),
                    )
                }))
                .into_any_element()
        })
        .collect();
        let group = |rows: Vec<AnyElement>| {
            v_flex()
                .pt(px(6.))
                .gap(px(3.))
                .border_t_1()
                .border_color(palette.border)
                .children(rows)
        };
        v_flex()
            .gap(px(3.))
            .child(names)
            .child(accuracy)
            .child(group(
                counts
                    .map(IntoElement::into_any_element)
                    .chain(std::iter::once(acpl.into_any_element()))
                    .collect(),
            ))
            .when(!phase_rows.is_empty(), |table| {
                table.child(group(phase_rows))
            })
            .into_any_element()
    }
}

/// `EvalChart.vue`: the game's evaluation over time, Lichess-style. White's winning chances fill
/// from the bottom, the review's labels sit on the curve, and a click goes to that move.
pub(super) fn eval_chart(
    points: Vec<Option<f64>>,
    judgments: Vec<Option<String>>,
    current: Option<usize>,
    page: WeakEntity<AnalysisPage>,
    palette: &Palette,
) -> impl IntoElement {
    let (primary, dark) = (palette.primary, palette.dark);
    let count = points.len();
    let bounds_cell = std::rc::Rc::new(std::cell::Cell::new(Bounds::<Pixels>::default()));
    let measured = bounds_cell.clone();
    div()
        .id("eval-chart")
        .w_full()
        .h(px(64.))
        .rounded(px(6.))
        .overflow_hidden()
        .cursor_pointer()
        .on_click(move |event, _, cx| {
            let bounds = bounds_cell.get();
            let width = bounds.size.width.as_f32();
            if width <= 0. || count < 2 {
                return;
            }
            let ratio = ((event.position().x - bounds.origin.x).as_f32() / width).clamp(0., 1.);
            let ply = (ratio * (count - 1) as f32).round() as usize;
            let _ = page.update(cx, |page, cx| page.go_to_ply(ply, cx));
        })
        .child(
            canvas(
                move |bounds, _, _| measured.set(bounds),
                move |bounds, (), window, _| {
                    window.paint_quad(fill(bounds, rgb(0x403d39)));
                    if count < 2 {
                        return;
                    }
                    let (width, height) = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
                    // Kept off the edges so a dot for a won position stays whole.
                    let pad = 4.;
                    let x = |index: usize| {
                        bounds.origin.x + px(width * index as f32 / (count - 1) as f32)
                    };
                    let y = |chances: f64| {
                        bounds.origin.y
                            + px(pad + ((1. - chances as f32) / 2.) * (height - 2. * pad))
                    };
                    // White's area, one shape per run of known positions.
                    let mut index = 0;
                    while index < count {
                        if points[index].is_none() {
                            index += 1;
                            continue;
                        }
                        let start = index;
                        while index < count && points[index].is_some() {
                            index += 1;
                        }
                        if index - start < 2 {
                            continue;
                        }
                        let mut area = PathBuilder::fill();
                        area.move_to(point(x(start), bounds.bottom()));
                        for (at, chances) in points.iter().enumerate().take(index).skip(start) {
                            area.line_to(point(x(at), y(chances.unwrap_or(0.))));
                        }
                        area.line_to(point(x(index - 1), bounds.bottom()));
                        area.close();
                        if let Ok(path) = area.build() {
                            window.paint_path(path, rgb(0xf2f0ea));
                        }
                    }
                    let middle = bounds.origin.y + px(height / 2.);
                    window.paint_quad(fill(
                        Bounds::new(
                            point(bounds.origin.x, middle),
                            size(bounds.size.width, px(1.)),
                        ),
                        rgba(0xd64f008c),
                    ));
                    if let Some(current) = current {
                        window.paint_quad(fill(
                            Bounds::new(
                                point(x(current) - px(1.), bounds.origin.y),
                                size(px(2.), bounds.size.height),
                            ),
                            primary,
                        ));
                    }
                    for (move_index, judgment) in judgments.iter().enumerate() {
                        let (Some(judgment), Some(Some(chances))) =
                            (judgment, points.get(move_index + 1))
                        else {
                            continue;
                        };
                        let center = point(x(move_index + 1), y(*chances));
                        let radius = px(3.2);
                        window.paint_quad(
                            fill(
                                Bounds::new(
                                    point(center.x - radius, center.y - radius),
                                    size(radius * 2., radius * 2.),
                                ),
                                judgment_color(judgment, dark),
                            )
                            .corner_radii(radius)
                            .border_widths(px(1.))
                            .border_color(rgb(0x403d39)),
                        );
                    }
                },
            )
            .size_full(),
        )
}

#[cfg(test)]
mod tests {
    use super::tried_nag;
    use crate::game::rules;
    use serde_json::json;

    /// A tried move is labelled as Electron labels it (`NAGS` in `stores/puzzleAnalysis.ts`), and
    /// the glyph the rules give that NAG is the one the move list shows.
    #[test]
    fn a_judged_tried_move_gets_its_glyph() {
        for (judgment, glyph) in [("inaccuracy", "?!"), ("mistake", "?"), ("blunder", "??")] {
            let nag = tried_nag(judgment).unwrap();
            let shown = rules("moveGlyph", &[json!([nag])]).unwrap();
            assert_eq!(shown["glyph"], json!(glyph), "{judgment}");
        }
        assert_eq!(tried_nag("good"), None);
    }
}
