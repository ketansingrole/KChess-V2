//! Repertoire (`OpeningTrainer.vue`, `useOpeningDrill.ts`): drill a repertoire. Any saved study is a
//! tree of moves; KChess plays the other side's moves from it (favouring the lines you get wrong)
//! and you answer with one of yours. One chapter can be drilled, or all of them: chapters from the
//! same starting position are one tree. An opening chosen on the Openings page is drilled without a
//! study: its named line, then the explorer's popular moves for a few more.

use std::collections::VecDeque;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_core::Level;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use shakmaty::{Color, Position as _};

use super::run;
use crate::assets::icon;
use crate::board::{Arrow, BoardEvent, BoardState, ChessBoard};
use crate::core::AppCore;
use crate::game::tree::{TreeNode, child_path, comment_text};
use crate::game::{STANDARD_FEN, random_unit, rules, view};
use crate::pages::Page;
use crate::pages::openings::OpeningChoice;
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout};
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::theme::Palette;
use crate::widgets::{self, Pill, Tone};

const STUDY: &str = "kchess:repertoire-study";
const OPENING_CHOICE: &str = "kchess:repertoire-opening";
const COLOR: &str = "kchess:repertoire-color";
const CHAPTER: &str = "kchess:repertoire-chapter";
/// The repertoire's value when it is the chosen opening rather than a study.
const OPENING: &str = "opening";
const ALL_CHAPTERS: &str = "all";
/// How far past the named line an opening drill goes, in half-moves…
const EXTRA_PLIES: usize = 6;
/// …and how many positions it asks the explorer about to get there.
const LOOKUP_BUDGET: usize = 16;
const REPLY: Duration = Duration::from_millis(450);
const FIRST_REPLY: Duration = Duration::from_millis(400);
const NEXT_LINE: Duration = Duration::from_millis(1400);

#[derive(Clone, Deserialize)]
struct Chapter {
    id: String,
    name: String,
    pgn: String,
}

#[derive(Clone, Deserialize)]
struct Study {
    id: String,
    name: String,
    chapters: Vec<Chapter>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Ok,
    Wrong,
    Done,
    Info,
}

struct Feedback {
    kind: Kind,
    text: String,
    detail: Option<String>,
}

/// A select's entry, known by its value: a study and the opening may share a name.
#[derive(Clone)]
struct Choice {
    value: SharedString,
    label: SharedString,
}

impl SearchableListItem for Choice {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &SharedString {
        &self.value
    }
}

/// e1g1 and e1h1 are the same castling move.
fn same_castle(a: &str, b: &str) -> bool {
    let king_move = |uci: &str| match uci {
        "e1h1" => "e1g1".to_string(),
        "e1a1" => "e1c1".to_string(),
        "e8h8" => "e8g8".to_string(),
        "e8a8" => "e8c8".to_string(),
        other => other.to_string(),
    };
    king_move(a) == king_move(b)
}

/// Adds another chapter's moves (and comments) from the same position to a tree.
fn merge(into: &mut TreeNode, from: TreeNode) {
    for comment in from.comments.unwrap_or_default() {
        let comments = into.comments.get_or_insert_with(Vec::new);
        if !comments.contains(&comment) {
            comments.push(comment);
        }
    }
    for child in from.children {
        match into
            .children
            .iter_mut()
            .find(|entry| entry.uci == child.uci)
        {
            Some(same) => merge(same, child),
            None => into.children.push(child),
        }
    }
}

/// How often the moves at and below a node were missed: the trainer steers towards weak lines.
fn weakness(start: &TreeNode, table: &Value) -> f64 {
    let mut total = 0.;
    let mut stack = vec![start];
    while let Some(next) = stack.pop() {
        total += table[&next.fen].as_f64().unwrap_or(0.);
        stack.extend(next.children.iter());
    }
    total
}

/// One of the nodes, more often those with weak lines below them; `draw` is in [0, 1).
fn pick_weak(nodes: &[&TreeNode], table: &Value, draw: f64) -> Option<usize> {
    let weights: Vec<f64> = nodes
        .iter()
        .map(|node| 1. + 2. * weakness(node, table))
        .collect();
    let mut pick = draw * weights.iter().sum::<f64>();
    for (index, weight) in weights.iter().enumerate() {
        pick -= weight;
        if pick <= 0. {
            return Some(index);
        }
    }
    (!nodes.is_empty()).then_some(0)
}

/// The trees drilled from a study: one per starting position among the chosen chapters.
fn study_roots(chapters: &[Chapter]) -> Vec<TreeNode> {
    let mut roots: Vec<TreeNode> = Vec::new();
    for chapter in chapters {
        let Some(tree) = TreeNode::from_pgn(&chapter.pgn) else {
            continue;
        };
        match roots.iter_mut().find(|root| root.fen == tree.fen) {
            Some(same) => merge(same, tree),
            None => roots.push(tree),
        }
    }
    roots
}

fn turn_of(fen: &str) -> Color {
    if fen.split(' ').nth(1) == Some("b") {
        Color::Black
    } else {
        Color::White
    }
}

fn color_name(color: Color) -> &'static str {
    if color == Color::White {
        "white"
    } else {
        "black"
    }
}

pub struct Repertoire {
    studies: Vec<Study>,
    misses: Value,
    /// An opening's drill tree, and the opening and side it was built for.
    drill: Option<TreeNode>,
    drill_for: Option<Value>,
    preparing: bool,
    note: String,
    drill_task: Option<Task<()>>,
    roots: Vec<TreeNode>,
    /// The position on the board: a tree and the path in it.
    at: Option<(usize, String)>,
    feedback: Option<Feedback>,
    hint: Vec<Arrow>,
    lines: u32,
    correct: u32,
    wrong: u32,
    saving: bool,
    reply: Option<Task<()>>,
    board: Entity<ChessBoard>,
    layout: PlayLayout,
    move_input: Entity<InputState>,
    study_select: Entity<SelectState<Vec<Choice>>>,
    chapter_select: Entity<SelectState<Vec<Choice>>>,
    /// The selects' items are rebuilt on the next frame, where there is a window.
    stale_selects: bool,
    /// The choices the trees were built for: another page (Openings' Train) may change them.
    built_for: Value,
    _subscriptions: Vec<Subscription>,
}

impl Repertoire {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Repertoire {
        let board = cx.new(|_| {
            ChessBoard::new(BoardState::of(
                &kchess_domain::rules::setup_start("standard", STANDARD_FEN).unwrap_or_default(),
                Color::White,
                None,
                None,
            ))
        });
        let move_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Type a move: Nf3 or g1f3"));
        let study_select = cx.new(|cx| SelectState::new(Vec::<Choice>::new(), None, window, cx));
        let chapter_select = cx.new(|cx| SelectState::new(Vec::<Choice>::new(), None, window, cx));
        let settings = cx.global::<SettingsHandle>().0.clone();
        let subscriptions = vec![
            cx.subscribe(&board, |this, _, event: &BoardEvent, cx| {
                if let BoardEvent::Move(uci) = event {
                    this.play(uci, cx);
                }
            }),
            cx.subscribe_in(
                &move_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.typed_move(window, cx);
                    }
                },
            ),
            cx.subscribe(
                &study_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        crate::prefs::set(cx, STUDY, value.to_string());
                        this.stale_selects = true;
                        this.rebuild(cx);
                    }
                },
            ),
            cx.subscribe(
                &chapter_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        crate::prefs::set(cx, CHAPTER, value.to_string());
                        this.rebuild(cx);
                    }
                },
            ),
            // The board follows blindfold mode.
            cx.subscribe(&settings, |this, _, _: &SettingsChanged, cx| {
                this.redraw(cx)
            }),
        ];
        let mut drill = Repertoire {
            studies: Vec::new(),
            misses: Value::Null,
            drill: None,
            drill_for: None,
            preparing: false,
            note: String::new(),
            drill_task: None,
            roots: Vec::new(),
            at: None,
            feedback: None,
            hint: Vec::new(),
            lines: 0,
            correct: 0,
            wrong: 0,
            saving: false,
            reply: None,
            board,
            layout: PlayLayout::default(),
            move_input,
            study_select,
            chapter_select,
            stale_selects: true,
            built_for: Value::Null,
            _subscriptions: subscriptions,
        };
        drill.load_library(true, cx);
        drill.rebuild(cx);
        drill
    }

    /// The studies and the missed moves, as the core keeps them; `restart` starts a line from them.
    fn load_library(&mut self, restart: bool, cx: &mut Context<Self>) {
        let library = cx.global::<AppCore>().call("library", Vec::new());
        cx.spawn(async move |this, cx| match library.await {
            Ok(library) => {
                let _ = this.update(cx, |this, cx| {
                    this.studies =
                        serde_json::from_value(library["studies"].clone()).unwrap_or_default();
                    this.misses = library["repertoireMisses"].clone();
                    this.stale_selects = true;
                    if restart {
                        this.rebuild(cx);
                    }
                    cx.notify();
                });
            }
            Err(cause) => crate::log::write(
                Level::Warn,
                "repertoire",
                &format!("Reading the studies failed: {}", cause.message),
            ),
        })
        .detach();
    }

    /* ── What is drilled ──────────────────────────────────────────────── */

    fn opening(cx: &App) -> Option<OpeningChoice> {
        crate::prefs::get::<Option<OpeningChoice>>(cx, OPENING_CHOICE, None)
    }

    fn color(cx: &App) -> Color {
        if crate::prefs::get::<String>(cx, COLOR, "white".into()) == "black" {
            Color::Black
        } else {
            Color::White
        }
    }

    fn study_id(cx: &App) -> String {
        crate::prefs::get::<String>(cx, STUDY, String::new())
    }

    fn drills_opening(cx: &App) -> bool {
        Self::study_id(cx) == OPENING && Self::opening(cx).is_some()
    }

    fn study(&self, cx: &App) -> Option<&Study> {
        let id = Self::study_id(cx);
        self.studies.iter().find(|study| study.id == id)
    }

    /// The chosen chapter; one that is gone (or from another study) means all chapters.
    fn chapter(&self, cx: &App) -> Option<&Chapter> {
        let id = crate::prefs::get::<String>(cx, CHAPTER, ALL_CHAPTERS.into());
        self.study(cx)?
            .chapters
            .iter()
            .find(|chapter| chapter.id == id)
    }

    /// Where missed moves are counted: by study (or opening) and side.
    fn miss_key(&self, cx: &App) -> String {
        let color = color_name(Self::color(cx));
        match Self::opening(cx).filter(|_| Self::drills_opening(cx)) {
            Some(opening) => format!(
                "opening:{}:{color}",
                opening.name.chars().take(180).collect::<String>()
            ),
            None => format!("{}:{color}", Self::study_id(cx)),
        }
    }

    fn choices(cx: &App) -> Value {
        json!([
            Self::study_id(cx),
            crate::prefs::get::<String>(cx, CHAPTER, ALL_CHAPTERS.into()),
            color_name(Self::color(cx)),
            Self::opening(cx).map(|opening| json!(opening)),
        ])
    }

    /// The trees for what is chosen, the opening's built first when it changed; then a new line.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.built_for = Self::choices(cx);
        if Self::drills_opening(cx) {
            let wanted = json!([
                Self::opening(cx).map(|o| json!(o)),
                color_name(Self::color(cx))
            ]);
            if self.drill_for.as_ref() != Some(&wanted) {
                self.drill_for = Some(wanted);
                self.build_drill(cx);
            }
            self.roots = self.drill.clone().into_iter().collect();
        } else {
            self.drill_for = None;
            self.drill_task = None;
            self.drill = None;
            self.preparing = false;
            self.roots = match self.chapter(cx) {
                Some(chapter) => study_roots(std::slice::from_ref(chapter)),
                None => self
                    .study(cx)
                    .map(|study| study_roots(&study.chapters))
                    .unwrap_or_default(),
            };
        }
        self.start_line(cx);
    }

    /// `useOpeningDrill`: the named line, then the explorer's most played move for the player and
    /// its popular replies for the opponent. Without the explorer the named line alone is drilled,
    /// and the note says why.
    fn build_drill(&mut self, cx: &mut Context<Self>) {
        self.drill = None;
        self.note.clear();
        let Some(opening) = Self::opening(cx) else {
            self.preparing = false;
            self.drill_task = None;
            return;
        };
        let color = Self::color(cx);
        let mut root = TreeNode::new(None);
        let mut line = String::new();
        for uci in &opening.moves {
            match root.add_move(&line, uci) {
                Some(next) => line = next,
                None => break,
            }
        }
        self.preparing = true;
        let core = cx.global::<AppCore>().clone();
        self.drill_task = Some(cx.spawn(async move |this, cx| {
            let mut queue = VecDeque::from([(line.clone(), 0)]);
            let mut note = String::new();
            for _ in 0..LOOKUP_BUDGET {
                let Some((path, depth)) = queue.pop_front() else {
                    break;
                };
                let fen = root.node_at(&path).fen.clone();
                let found = core
                    .call(
                        "positionLookup",
                        vec![json!("opening"), json!(fen), json!({})],
                    )
                    .await;
                let moves = match found {
                    Ok(found) => found["moves"].clone(),
                    Err(cause) => {
                        crate::log::write(
                            Level::Warn,
                            "opening-drill",
                            &format!("Looking up popular replies failed: {}", cause.message),
                        );
                        // The named line is still worth drilling; a drill cut short says why.
                        note = if path == line {
                            format!("Only the named line: {}", cause.message)
                        } else {
                            format!("Some popular replies are missing: {}", cause.message)
                        };
                        break;
                    }
                };
                let counts: Vec<Value> = moves
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|entry| {
                        json!({
                            "uci": entry["uci"],
                            "white": entry["white"],
                            "draws": entry["draws"],
                            "black": entry["black"],
                        })
                    })
                    .collect();
                let yours = turn_of(&fen) == color;
                let replies: Vec<String> = rules("openingReplies", &[json!(counts), json!(yours)])
                    .ok()
                    .and_then(|replies| serde_json::from_value(replies).ok())
                    .unwrap_or_default();
                for uci in replies {
                    if let Some(child) = root.add_move(&path, &uci)
                        && depth + 1 < EXTRA_PLIES
                    {
                        queue.push_back((child, depth + 1));
                    }
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.drill = Some(root);
                this.note = note;
                this.preparing = false;
                this.roots = this.drill.clone().into_iter().collect();
                this.start_line(cx);
            });
        }));
    }

    /* ── The drill ────────────────────────────────────────────────────── */

    fn node(&self) -> Option<&TreeNode> {
        let (root, path) = self.at.as_ref()?;
        Some(self.roots.get(*root)?.node_at(path))
    }

    fn your_turn(&self, cx: &App) -> bool {
        self.node()
            .is_some_and(|node| turn_of(&node.fen) == Self::color(cx))
    }

    fn after(
        &mut self,
        delay: Duration,
        step: fn(&mut Self, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) {
        self.reply = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, step);
        }));
    }

    fn start_line(&mut self, cx: &mut Context<Self>) {
        self.reply = None;
        self.hint.clear();
        let table = self.misses[self.miss_key(cx)].clone();
        let roots: Vec<&TreeNode> = self.roots.iter().collect();
        self.at = pick_weak(&roots, &table, random_unit()).map(|index| (index, String::new()));
        if self.at.is_some() {
            self.feedback = Some(Feedback {
                kind: Kind::Info,
                text: "New line.".into(),
                detail: None,
            });
            if !self.your_turn(cx) {
                self.after(FIRST_REPLY, Self::opponent_move, cx);
            }
        }
        self.redraw(cx);
    }

    fn opponent_move(&mut self, cx: &mut Context<Self>) {
        let Some((root, path)) = self.at.clone() else {
            return;
        };
        let table = self.misses[self.miss_key(cx)].clone();
        let node = self.roots[root].node_at(&path);
        let children: Vec<&TreeNode> = node.children.iter().collect();
        let Some(next) = pick_weak(&children, &table, random_unit()).map(|index| children[index])
        else {
            return self.finish_line(cx);
        };
        let (uci, san, last) = (next.uci.clone(), next.san.clone(), next.children.is_empty());
        self.at = Some((root, child_path(&path, &uci)));
        crate::sound::move_sound(&san, cx);
        if last {
            self.finish_line(cx);
        }
        self.redraw(cx);
    }

    fn finish_line(&mut self, cx: &mut Context<Self>) {
        self.lines += 1;
        self.feedback = Some(Feedback {
            kind: Kind::Done,
            text: "End of this line".into(),
            detail: Some("Next one in a moment…".into()),
        });
        self.after(NEXT_LINE, Self::start_line, cx);
        self.redraw(cx);
    }

    fn play(&mut self, uci: &str, cx: &mut Context<Self>) {
        let Some((root, path)) = self.at.clone() else {
            return;
        };
        if !self.your_turn(cx) {
            self.redraw(cx);
            return;
        }
        let node = self.roots[root].node_at(&path);
        let found = node
            .children
            .iter()
            .find(|child| child.uci == uci || same_castle(&child.uci, uci))
            .map(|child| {
                (
                    child.uci.clone(),
                    child.san.clone(),
                    child.children.is_empty(),
                )
            });
        let Some((uci, san, last)) = found else {
            self.wrong += 1;
            let fen = node.fen.clone();
            let expected: Vec<&str> = node
                .children
                .iter()
                .map(|child| child.san.as_str())
                .collect();
            self.feedback = Some(Feedback {
                kind: Kind::Wrong,
                text: "Not in your repertoire".into(),
                detail: Some(format!(
                    "Expected {}.",
                    if expected.is_empty() {
                        "nothing (line ended)".to_string()
                    } else {
                        expected.join(" or ")
                    }
                )),
            });
            self.hint = Self::arrows(node, Arrow::solution);
            self.record_miss(fen, cx);
            // The wrong move goes back.
            self.redraw(cx);
            return;
        };
        self.correct += 1;
        self.hint.clear();
        self.at = Some((root, child_path(&path, &uci)));
        crate::sound::move_sound(&san, cx);
        self.feedback = Some(Feedback {
            kind: Kind::Ok,
            text: format!("{san} ✓"),
            detail: None,
        });
        if last {
            self.finish_line(cx);
        } else {
            self.after(REPLY, Self::opponent_move, cx);
        }
        self.redraw(cx);
    }

    fn typed_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.node() else {
            return;
        };
        let text = self.move_input.read(cx).value().trim().to_string();
        let setup = json!({ "variant": "standard", "fen": node.fen });
        let typed = kchess_domain::rules::setup_start("standard", &node.fen)
            .and_then(|position| view::typed_uci(&setup, &position, &text));
        if let Some(uci) = typed {
            self.move_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.play(&uci, cx);
        }
    }

    fn arrows(
        node: &TreeNode,
        brush: fn(shakmaty::Square, shakmaty::Square) -> Arrow,
    ) -> Vec<Arrow> {
        node.children
            .iter()
            .filter_map(|child| {
                Some(brush(
                    child.uci.get(0..2)?.parse().ok()?,
                    child.uci.get(2..4)?.parse().ok()?,
                ))
            })
            .collect()
    }

    fn show_answer(&mut self, cx: &mut Context<Self>) {
        if !self.your_turn(cx) {
            return;
        }
        if let Some(node) = self.node() {
            self.hint = Self::arrows(node, Arrow::hint);
        }
        self.redraw(cx);
    }

    fn record_miss(&mut self, fen: String, cx: &mut Context<Self>) {
        let key = self.miss_key(cx);
        if !self.misses.is_object() {
            self.misses = json!({});
        }
        let table = &mut self.misses[&key];
        if !table.is_object() {
            *table = json!({});
        }
        table[&fen] = json!(table[&fen].as_f64().unwrap_or(0.) + 1.);
        let saved = cx
            .global::<AppCore>()
            .call("recordRepertoireMiss", vec![json!(key), json!(fen)]);
        self.keep_misses(saved, "saving a repertoire miss", cx);
    }

    fn clear_misses(&mut self, cx: &mut Context<Self>) {
        let key = self.miss_key(cx);
        if let Some(misses) = self.misses.as_object_mut() {
            misses.remove(&key);
        }
        let saved = cx
            .global::<AppCore>()
            .call("clearRepertoireMisses", vec![json!(key)]);
        self.keep_misses(saved, "clearing repertoire misses", cx);
    }

    fn keep_misses(
        &mut self,
        saved: impl std::future::Future<Output = Result<Value, kchess_core::CoreError>> + 'static,
        what: &'static str,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| match saved.await {
            Ok(misses) => {
                let _ = this.update(cx, |this, _| this.misses = misses);
            }
            Err(cause) => crate::log::write(
                Level::Warn,
                "studies",
                &format!("{what} failed: {}", cause.message),
            ),
        })
        .detach();
    }

    /// A name no study has yet (`freshName`).
    fn fresh_name(&self, base: &str) -> String {
        (1..)
            .map(|n| {
                if n == 1 {
                    base.to_string()
                } else {
                    format!("{base} {n}")
                }
            })
            .find(|name| !self.studies.iter().any(|study| &study.name == name))
            .unwrap_or_else(|| base.to_string())
    }

    /// Keep the drilled opening, with the replies found for it, as a study to edit or drill later.
    fn save_opening(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(mut tree), Some(opening)) = (self.drill.clone(), Self::opening(cx)) else {
            return;
        };
        let mut headers = Map::new();
        headers.insert("Event".into(), json!(opening.name));
        tree.headers = Some(headers);
        let name = self.fresh_name(&opening.name);
        self.saving = true;
        cx.notify();
        let saved = cx.global::<AppCore>().call(
            "studyCommand",
            vec![
                json!({ "op": "save", "name": name, "pgn": tree.to_pgn() }),
                json!(true),
            ],
        );
        cx.spawn_in(window, async move |this, cx| {
            let saved = saved.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match saved {
                    Ok(_) => {
                        window.push_notification("Saved to Studies", cx);
                        this.load_library(false, cx);
                    }
                    Err(cause) => {
                        crate::log::write(
                            Level::Warn,
                            "opening-trainer",
                            &format!("Saving the opening failed: {}", cause.message),
                        );
                        window.push_notification("Could not save the study", cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn redraw(&mut self, cx: &mut Context<Self>) {
        let color = Self::color(cx);
        let state = self.node().map(|node| {
            let position =
                kchess_domain::rules::setup_start("standard", &node.fen).unwrap_or_default();
            let last = node
                .uci
                .get(0..2)
                .zip(node.uci.get(2..4))
                .and_then(|(from, to)| Some((from.parse().ok()?, to.parse().ok()?)));
            let active = position.turn() == color;
            let mut state = BoardState::of(&position, color, active.then_some(color), last);
            state.arrows = self.hint.clone();
            state.blindfold = cx
                .global::<SettingsHandle>()
                .0
                .read(cx)
                .settings
                .frontend
                .blindfold;
            state
        });
        if let Some(state) = state {
            self.board
                .update(cx, |board, cx| board.set_state(state, cx));
        }
        cx.notify();
    }

    /// The selects' items and choices, as the studies and preferences now are.
    fn refresh_selects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let choice = |value: &str, label: String| Choice {
            value: value.to_string().into(),
            label: label.into(),
        };
        let mut studies: Vec<Choice> = Self::opening(cx)
            .map(|opening| choice(OPENING, opening.name))
            .into_iter()
            .collect();
        studies.extend(
            self.studies
                .iter()
                .map(|study| choice(&study.id, study.name.clone())),
        );
        let chosen = SharedString::from(Self::study_id(cx));
        self.study_select.update(cx, |state, cx| {
            state.set_items(studies, window, cx);
            state.set_selected_value(&chosen, window, cx);
        });

        let mut chapters = vec![choice(ALL_CHAPTERS, "All chapters".into())];
        if let Some(study) = self.study(cx) {
            chapters.extend(study.chapters.iter().enumerate().map(|(index, chapter)| {
                choice(&chapter.id, format!("{}. {}", index + 1, chapter.name))
            }));
        }
        let chosen = SharedString::from(
            self.chapter(cx)
                .map_or(ALL_CHAPTERS.to_string(), |chapter| chapter.id.clone()),
        );
        self.chapter_select.update(cx, |state, cx| {
            state.set_items(chapters, window, cx);
            state.set_selected_value(&chosen, window, cx);
        });
    }

    fn empty_actions() -> impl IntoElement {
        h_flex()
            .gap(px(8.))
            .flex_wrap()
            .justify_center()
            .child(
                Button::new("browse-openings")
                    .primary()
                    .label("Browse openings")
                    .on_click(|_, window, cx| crate::shell::open(Page::Openings, window, cx)),
            )
            .child(
                Button::new("open-studies")
                    .outline()
                    .label("Open studies")
                    .on_click(|_, window, cx| crate::shell::open(Page::Studies, window, cx)),
            )
    }

    fn empty(title: &'static str, description: Option<&'static str>, palette: &Palette) -> Div {
        widgets::empty_state(
            icon("book-marked"),
            title,
            description.map(SharedString::from),
            palette,
        )
        .py(px(32.))
        .child(Self::empty_actions())
    }
}

impl Render for Repertoire {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.built_for != Self::choices(cx) {
            self.stale_selects = true;
            self.rebuild(cx);
        }
        if std::mem::take(&mut self.stale_selects) {
            self.refresh_selects(window, cx);
        }
        let palette = *cx.global::<Palette>();
        let opening = Self::opening(cx);
        if self.studies.is_empty() && opening.is_none() {
            return div()
                .size_full()
                .px(crate::play_layout::side_padding(window))
                .pt(px(20.))
                .child(widgets::card(&palette).child(Self::empty(
                    "Nothing to drill yet",
                    Some("Drill an opening, or lines you saved as a study."),
                    &palette,
                )))
                .into_any_element();
        }
        let drills_opening = Self::drills_opening(cx);
        let color = Self::color(cx);
        let your_turn = self.your_turn(cx);
        let frontend = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .clone();
        let board_size = self.layout.board_size(window);
        let board_column = v_flex().w(board_size).flex_none().gap(widgets::STACK_GAP);
        let board_column = if self.at.is_some() {
            board_column
                .child(self.board.clone())
                .child(widgets::move_entry(&self.move_input, your_turn, &palette).h(px(36.)))
        } else if !(drills_opening && self.preparing) {
            board_column.child(widgets::card(&palette).child(Self::empty(
                "Choose a repertoire",
                None,
                &palette,
            )))
        } else {
            board_column
        };
        let how = if drills_opening {
            "After the named line you play the most played move on Lichess, and KChess answers with popular replies. Lines you got wrong come up more often."
        } else {
            "KChess answers with moves from your study. Lines you got wrong come up more often."
        };
        let help = Popover::new("repertoire-help")
            .anchor(Anchor::TopRight)
            .trigger(
                Button::new("repertoire-help-button")
                    .ghost()
                    .xsmall()
                    .icon(icon("circle-help"))
                    .accessibility_label("How replies are chosen"),
            )
            .content(move |_, _, _| {
                div()
                    .max_w(px(288.))
                    .p(px(12.))
                    .text_size(px(14.))
                    .child(how)
            });
        let chapters = self.study(cx).map_or(0, |study| study.chapters.len());
        let total = self.correct + self.wrong;
        let comment = self.node().map(|node| {
            node.comments
                .iter()
                .flatten()
                .map(|comment| comment_text(comment))
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        });
        let panel = widgets::side_panel(&palette)
            .h(self.layout.panel_height(board_size))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .gap(px(14.))
            .child(
                h_flex()
                    .items_start()
                    .justify_between()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight(650.))
                            .text_color(palette.highlighted)
                            .child("Repertoire"),
                    )
                    .child(
                        h_flex()
                            .gap(px(4.))
                            .child(help)
                            .child(widgets::view_toggles(frontend.blindfold, frontend.zen_mode)),
                    ),
            )
            .child(run::field(
                "Lines from",
                Select::new(&self.study_select)
                    .small()
                    .placeholder("Choose an opening or study")
                    .accessibility_label("Lines from"),
                &palette,
            ))
            .when(!drills_opening && chapters > 1, |panel| {
                panel.child(run::field(
                    "Chapter",
                    Select::new(&self.chapter_select)
                        .small()
                        .accessibility_label("Chapter"),
                    &palette,
                ))
            })
            .child(run::field(
                "You play",
                widgets::pills(
                    "repertoire-color",
                    ["White", "Black"]
                        .into_iter()
                        .map(|label| Pill {
                            icon: None,
                            label: label.into(),
                            badge: None,
                            disabled: false,
                        })
                        .collect(),
                    usize::from(color == Color::Black),
                    {
                        let page = cx.entity().downgrade();
                        move |index, _, cx| {
                            crate::prefs::set(
                                cx,
                                COLOR,
                                if index == 1 { "black" } else { "white" },
                            );
                            let _ = page.update(cx, |page, cx| page.rebuild(cx));
                        }
                    },
                    false,
                    &palette,
                ),
                &palette,
            ))
            .when(drills_opening && self.preparing, |panel| {
                panel.child(div().text_size(px(14.)).child("Loading popular replies…"))
            })
            .when(
                drills_opening && !self.preparing && !self.note.is_empty(),
                |panel| {
                    panel.child(
                        div()
                            .text_size(px(14.))
                            .text_color(palette.text_muted)
                            .child(self.note.clone()),
                    )
                },
            )
            .children(
                self.feedback
                    .as_ref()
                    .filter(|_| self.at.is_some())
                    .map(|feedback| {
                        widgets::status_banner(
                            match feedback.kind {
                                Kind::Ok | Kind::Done => Tone::Win,
                                Kind::Wrong => Tone::Loss,
                                Kind::Info => Tone::Plain,
                            },
                            match feedback.kind {
                                Kind::Wrong => icon("x"),
                                Kind::Info => icon("info"),
                                Kind::Ok | Kind::Done => icon("check"),
                            },
                            feedback.text.clone().into(),
                            feedback.detail.clone().map(SharedString::from),
                            None,
                            &palette,
                        )
                    }),
            )
            .child(div().text_size(px(14.)).child(format!(
                "Lines finished: {} · correct {} · wrong {}{}",
                self.lines,
                self.correct,
                self.wrong,
                if total == 0 {
                    String::new()
                } else {
                    format!(
                        " · {}%",
                        (f64::from(self.correct) * 100. / f64::from(total)).round()
                    )
                }
            )))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .pt(px(14.))
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        Button::new("show-move")
                            .outline()
                            .icon(icon("lightbulb"))
                            .label("Show move")
                            .disabled(!your_turn)
                            .on_click(cx.listener(|this, _, _, cx| this.show_answer(cx))),
                    )
                    .child(
                        Button::new("restart-line")
                            .outline()
                            .icon(icon("rotate-ccw"))
                            .label("Restart line")
                            .on_click(cx.listener(|this, _, _, cx| this.start_line(cx))),
                    )
                    .child(
                        Button::new("forget-mistakes")
                            .ghost()
                            .icon(icon("eraser"))
                            .label("Forget mistakes")
                            .on_click(cx.listener(|this, _, _, cx| this.clear_misses(cx))),
                    )
                    .when(drills_opening, |actions| {
                        actions.child(
                            Button::new("save-opening")
                                .ghost()
                                .icon(icon("save"))
                                .label("Save as study")
                                .loading(self.saving)
                                .disabled(self.drill.is_none())
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.save_opening(window, cx)
                                    }),
                                ),
                        )
                    }),
            )
            // The study's note on this position: below the controls, however long it is.
            .children(comment.filter(|text| !text.is_empty()).map(|text| {
                div()
                    .px(px(10.))
                    .py(px(6.))
                    .border_l_2()
                    .border_color(crate::theme::tint(palette.primary, 50.))
                    .rounded_r(px(6.))
                    .bg(palette.elevated)
                    .text_size(px(13.))
                    .text_color(palette.toned)
                    .child(text)
            }));
        div()
            .size_full()
            .child(
                self.layout
                    .render("repertoire", board_column, panel, window, cx),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Chapter, TreeNode, merge, pick_weak, same_castle, study_roots, weakness};
    use serde_json::json;

    fn chapter(pgn: &str) -> Chapter {
        Chapter {
            id: pgn.into(),
            name: pgn.into(),
            pgn: pgn.into(),
        }
    }

    #[test]
    fn castling_either_way_is_the_same_move() {
        assert!(same_castle("e1h1", "e1g1"));
        assert!(same_castle("e8c8", "e8a8"));
        assert!(!same_castle("e1g1", "e1c1"));
    }

    #[test]
    fn chapters_from_one_start_are_one_tree() {
        let roots = study_roots(&[
            chapter("1. e4 e5 2. Nf3"),
            chapter("1. e4 c5"),
            chapter("1. d4"),
        ]);
        assert_eq!(roots.len(), 1);
        let root = &roots[0];
        assert_eq!(root.children.len(), 2);
        let e4 = root
            .children
            .iter()
            .find(|child| child.san == "e4")
            .unwrap();
        assert_eq!(e4.children.len(), 2);
    }

    #[test]
    fn missed_lines_come_up_more_often() {
        let mut tree = TreeNode::new(None);
        let e4 = tree.add_move("", "e2e4").unwrap();
        tree.add_move("", "d2d4").unwrap();
        let fen = tree.node_at(&e4).fen.clone();
        let table = json!({ fen: 3 });
        let children: Vec<&TreeNode> = tree.children.iter().collect();
        assert_eq!(weakness(children[0], &table), 3.);
        // Weights 7 and 1: draws below 7/8 pick the weak line.
        assert_eq!(pick_weak(&children, &table, 0.8), Some(0));
        assert_eq!(pick_weak(&children, &table, 0.9), Some(1));
        assert_eq!(pick_weak(&[], &table, 0.5), None);
        let mut other = TreeNode::new(None);
        other.add_move("", "c2c4").unwrap();
        merge(&mut tree, other);
        assert_eq!(tree.children.len(), 3);
    }
}
