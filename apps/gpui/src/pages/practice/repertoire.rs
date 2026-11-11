//! Repertoire: drill a repertoire. Any saved study is a
//! tree of moves; KChess plays the other side's moves from it (lines due for review first, then
//! favouring the lines you get wrong) and you answer with one of yours. Each line, root to leaf,
//! is scheduled by spaced repetition: a line finished without a mistake comes back later and
//! later, a missed one at once. One chapter can be drilled, or all of them: chapters from the
//! same starting position are one tree. An opening chosen on the Openings page is drilled without a
//! study: its named line, then the explorer's popular moves for a few more.

use crate::game::tree::TreePgn as _;
use kchess_contracts::calls;
use kchess_contracts::check::Fen;
use kchess_contracts::types::{LookupOptions, PositionLookupKind};
use std::collections::VecDeque;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::Level;
use kchess_domain::training::openings::{self, ExplorerMove};
use kchess_domain::training::repetition;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use shakmaty::Color;

use super::run;
use crate::assets::icon;
use crate::board::{Arrow, BoardEvent, BoardState, ChessBoard};
use crate::core::AppCore;
use crate::game::tree::{TreeNode, child_path, comment_text};
use crate::game::{STANDARD_FEN, random_unit, view};
use crate::pages::Page;
use crate::pages::openings::OpeningChoice;
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout};
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::studies::StudyHandle;
use crate::theme::{Palette, radii};
use crate::widgets::{self, Choice, Pill, Tone};

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
fn merge(into: &mut TreeNode, mut from: TreeNode) {
    for comment in from.comments.take().unwrap_or_default() {
        let comments = into.comments.get_or_insert_with(Vec::new);
        if !comments.contains(&comment) {
            comments.push(comment);
        }
    }
    for child in std::mem::take(&mut from.children) {
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

/// The paths of a tree's leaves: each one is a line, from the root to its end.
fn leaves(root: &TreeNode) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![(root, String::new())];
    while let Some((node, path)) = stack.pop() {
        if node.children.is_empty() {
            if !path.is_empty() {
                found.push(path);
            }
            continue;
        }
        for child in node.children.iter().rev() {
            stack.push((child, child_path(&path, &child.uci)));
        }
    }
    found
}

/// A line's name in the schedule.
fn line_id(root: &TreeNode, path: &str) -> String {
    repetition::line_id(&root.fen, root.fen == STANDARD_FEN, path)
}

/// Whether `path` is `line` or on the way to it.
fn leads_to(path: &str, line: &str) -> bool {
    line == path
        || line
            .strip_prefix(path)
            .is_some_and(|rest| rest.starts_with(' '))
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0., |elapsed| elapsed.as_millis() as f64)
}

/// When a line is next due, from now: "in 5 min", "in 3 h", "in 2 days".
fn due_in(due: f64, now: f64) -> String {
    let minutes = ((due - now) / 60_000.).ceil().max(1.);
    if minutes < 60. {
        format!("in {minutes} min")
    } else if minutes < 24. * 60. {
        format!("in {} h", (minutes / 60.).round())
    } else {
        let days = (minutes / (24. * 60.)).round();
        format!("in {days} day{}", if days == 1. { "" } else { "s" })
    }
}

/// The panel's line on the schedule: how many lines are due, or when the next one is.
fn schedule_text(summary: &repetition::Summary, now: f64) -> Option<String> {
    match (summary.due, summary.next_due) {
        (0, None) => None,
        (0, Some(due)) => Some(format!("Next line due {}", due_in(due, now))),
        (1, _) => Some("1 line due".into()),
        (due, _) => Some(format!("{due} lines due")),
    }
}

/// Whether `color` has a move to play at `node`: a line's last move leaves nothing to play,
/// whoever is on move, and the next line follows.
fn to_play(node: &TreeNode, color: Color) -> bool {
    !node.children.is_empty() && turn_of(&node.fen) == color
}

fn turn_of(fen: &str) -> Color {
    if fen.split(' ').nth(1) == Some("b") {
        Color::Black
    } else {
        Color::White
    }
}

/// The repertoire trainer's actions, as its buttons offer them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RepertoireAction {
    ShowMove,
    Restart,
    Review,
    Forget,
    Save,
}

pub struct Repertoire {
    studies: Vec<Study>,
    misses: Value,
    /// Every repertoire's spaced-repetition schedule: key → line → card.
    schedule: Value,
    /// The line being drilled, when one was chosen (a due line): the opponent's moves follow it.
    target: Option<String>,
    /// Whether this line had a wrong move or a shown one: it is then answered wrong.
    line_missed: bool,
    /// Only due lines come up, until none is left.
    reviewing: bool,
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
                &kchess_domain::rules::setup_start(
                    kchess_domain::variant::Variant::Standard,
                    STANDARD_FEN,
                )
                .unwrap_or_default(),
                Color::White,
                None,
                None,
            ))
        });
        let move_input = cx.new(|cx| InputState::new(window, cx).placeholder("Type a move"));
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
            // Studies saved, renamed or removed anywhere show in the select.
            cx.observe(&cx.global::<StudyHandle>().0.clone(), |this, _, cx| {
                this.studies = Self::studies_of(cx);
                this.stale_selects = true;
                cx.notify();
            }),
            // The board follows blindfold mode.
            cx.subscribe(&settings, |this, _, _: &SettingsChanged, cx| {
                this.redraw(cx)
            }),
        ];
        let mut drill = Repertoire {
            studies: Self::studies_of(cx),
            misses: Value::Null,
            schedule: Value::Null,
            target: None,
            line_missed: false,
            reviewing: false,
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
        drill.load_misses(cx);
        drill.rebuild(cx);
        drill
    }

    /// The missed moves, as the core keeps them.
    fn load_misses(&mut self, cx: &mut Context<Self>) {
        let library = cx.global::<AppCore>().request(calls::Library {});
        cx.spawn(async move |this, cx| match library.await {
            Ok(library) => {
                let _ = this.update(cx, |this, cx| {
                    this.misses = library["repertoireMisses"].clone();
                    this.schedule = library["repertoireSchedule"].clone();
                    // A due line comes first, also when the schedule arrived after the first line.
                    if this.target.is_none()
                        && this.correct + this.wrong == 0
                        && this.due_line(cx).is_some()
                    {
                        this.start_line(cx);
                    }
                    cx.notify();
                });
            }
            Err(cause) => crate::log::write(
                Level::Warn,
                "repertoire",
                &format!("Reading the missed moves failed: {}", cause.message),
            ),
        })
        .detach();
    }

    /// The saved studies, as the library has them.
    fn studies_of(cx: &App) -> Vec<Study> {
        serde_json::from_value(json!(cx.global::<StudyHandle>().0.read(cx).items()))
            .unwrap_or_default()
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
        let color = kchess_domain::color::name(Self::color(cx));
        match Self::opening(cx).filter(|_| Self::drills_opening(cx)) {
            Some(opening) => format!(
                "opening:{}:{color}",
                opening.name.chars().take(180).collect::<String>()
            ),
            None => format!("{}:{color}", Self::study_id(cx)),
        }
    }

    fn choices(cx: &App) -> Value {
        let edited = cx
            .global::<StudyHandle>()
            .0
            .read(cx)
            .study(&Self::study_id(cx))
            .map(|study| study["updatedAt"].clone());
        json!([
            edited,
            Self::study_id(cx),
            crate::prefs::get::<String>(cx, CHAPTER, ALL_CHAPTERS.into()),
            kchess_domain::color::name(Self::color(cx)),
            Self::opening(cx).map(|opening| json!(opening)),
        ])
    }

    /// The trees for what is chosen, the opening's built first when it changed; then a new line.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.built_for = Self::choices(cx);
        if Self::drills_opening(cx) {
            let wanted = json!([
                Self::opening(cx).map(|o| json!(o)),
                kchess_domain::color::name(Self::color(cx))
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
                    .request(calls::PositionLookup {
                        kind: PositionLookupKind::Opening,
                        fen: Fen(fen.clone()),
                        options: Some(LookupOptions::default()),
                    })
                    .await;
                let moves = match found {
                    Ok(found) => found.saved.moves,
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
                let count = |n: Option<i64>| n.unwrap_or_default() as f64;
                let counts: Vec<ExplorerMove> = moves
                    .into_iter()
                    .map(|found| ExplorerMove {
                        white: count(found.white),
                        draws: count(found.draws),
                        black: count(found.black),
                        uci: found.uci,
                    })
                    .collect();
                let yours = turn_of(&fen) == color;
                let replies = openings::opening_replies(&counts, yours);
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
            .is_some_and(|node| to_play(node, Self::color(cx)))
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

    /// Every line of the trees: its tree, its path and its name in the schedule.
    fn lines(&self) -> Vec<(usize, String, String)> {
        self.roots
            .iter()
            .enumerate()
            .flat_map(|(index, root)| {
                leaves(root).into_iter().map(move |path| {
                    let id = line_id(root, &path);
                    (index, path, id)
                })
            })
            .collect()
    }

    /// How this repertoire's lines stand in the schedule now.
    fn summary(&self, cx: &App) -> repetition::Summary {
        let lines = self.lines();
        let table = &self.schedule[self.miss_key(cx)];
        repetition::summary(lines.iter().map(|(_, _, id)| id.as_str()), table, now_ms())
    }

    /// The most overdue line, if any is due.
    fn due_line(&self, cx: &App) -> Option<(usize, String)> {
        let lines = self.lines();
        let ids: Vec<&str> = lines.iter().map(|(_, _, id)| id.as_str()).collect();
        let table = &self.schedule[self.miss_key(cx)];
        let index = repetition::most_overdue(&ids, table, now_ms())?;
        let (root, path, _) = lines.into_iter().nth(index)?;
        Some((root, path))
    }

    /// Review the due lines, most overdue first, until none is left.
    fn review_due(&mut self, cx: &mut Context<Self>) {
        self.reviewing = true;
        self.start_line(cx);
    }

    fn start_line(&mut self, cx: &mut Context<Self>) {
        self.reply = None;
        self.hint.clear();
        self.line_missed = false;
        self.target = None;
        let due = self.due_line(cx);
        if due.is_none() && self.reviewing {
            self.reviewing = false;
            self.feedback = Some(Feedback {
                kind: Kind::Done,
                text: "Due lines reviewed".into(),
                detail: None,
            });
            self.redraw(cx);
            return;
        }
        self.at = match due {
            Some((root, path)) => {
                self.target = Some(path);
                Some((root, String::new()))
            }
            None => {
                let table = self.misses[self.miss_key(cx)].clone();
                let roots: Vec<&TreeNode> = self.roots.iter().collect();
                crate::game::drawn(random_unit())
                    .and_then(|draw| pick_weak(&roots, &table, draw))
                    .map(|index| (index, String::new()))
            }
        };
        if self.at.is_some() {
            self.feedback = Some(Feedback {
                kind: Kind::Info,
                text: if self.target.is_some() {
                    "Line due for review".into()
                } else {
                    "New line".into()
                },
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
        // The line chosen for review, while the moves are on it; otherwise the weak lines.
        let on_target = self.target.as_ref().and_then(|target| {
            children
                .iter()
                .position(|child| leads_to(&child_path(&path, &child.uci), target))
        });
        let Some(next) = on_target
            .or_else(|| {
                crate::game::drawn(random_unit())
                    .and_then(|draw| pick_weak(&children, &table, draw))
            })
            .map(|index| children[index])
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
        self.review_line(cx);
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
            self.line_missed = true;
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
                    "Expected {}",
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
            self.board.update(cx, |board, cx| board.undo_move(cx));
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
        let typed =
            kchess_domain::rules::setup_start(kchess_domain::variant::Variant::Standard, &node.fen)
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

    /// What the trainer's buttons can do now; nothing before there is a study or opening.
    pub(crate) fn actions(&self, cx: &App) -> Vec<RepertoireAction> {
        if self.studies.is_empty() && Self::opening(cx).is_none() {
            return Vec::new();
        }
        let mut actions = Vec::new();
        if self.your_turn(cx) {
            actions.push(RepertoireAction::ShowMove);
        }
        actions.push(RepertoireAction::Restart);
        if self.summary(cx).due > 0 {
            actions.push(RepertoireAction::Review);
        }
        if self.has_misses(cx) {
            actions.push(RepertoireAction::Forget);
        }
        if Self::drills_opening(cx) && self.drill.is_some() && !self.saving {
            actions.push(RepertoireAction::Save);
        }
        actions
    }

    /// Run an action as its button does, if it is still allowed.
    pub(crate) fn act(
        &mut self,
        action: RepertoireAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.actions(cx).contains(&action) {
            return;
        }
        match action {
            RepertoireAction::ShowMove => self.show_answer(cx),
            RepertoireAction::Restart => {
                self.reviewing = false;
                self.start_line(cx)
            }
            RepertoireAction::Review => self.review_due(cx),
            RepertoireAction::Forget => self.clear_misses(window, cx),
            RepertoireAction::Save => self.save_opening(window, cx),
        }
    }

    fn show_answer(&mut self, cx: &mut Context<Self>) {
        if !self.your_turn(cx) {
            return;
        }
        if let Some(node) = self.node() {
            self.hint = Self::arrows(node, Arrow::hint);
            // A shown move was not remembered.
            self.line_missed = true;
        }
        self.redraw(cx);
    }

    /// The finished line answered in the schedule: right when it had no wrong or shown move.
    fn review_line(&mut self, cx: &mut Context<Self>) {
        let Some((root, path)) = self.at.clone() else {
            return;
        };
        let Some(tree) = self.roots.get(root) else {
            return;
        };
        if path.is_empty() {
            return;
        }
        let (key, line, correct) = (self.miss_key(cx), line_id(tree, &path), !self.line_missed);
        // At once here, so the next line is chosen by it; the core's answer replaces it.
        let mut schedule = self.schedule.as_object().cloned().unwrap_or_default();
        if let Err(cause) = repetition::record(&mut schedule, &key, &line, correct, now_ms()) {
            crate::log::write(
                Level::Warn,
                "repertoire",
                &format!("Scheduling a line failed: {cause}"),
            );
            return;
        }
        self.schedule = Value::Object(schedule);
        let saved = cx.global::<AppCore>().request(calls::ReviewRepertoireLine {
            key: json!(key),
            line: json!(line),
            correct: json!(correct),
        });
        cx.spawn(async move |this, cx| match saved.await {
            Ok(schedule) => {
                let _ = this.update(cx, |this, cx| {
                    this.schedule = schedule;
                    cx.notify();
                });
            }
            Err(cause) => crate::log::write(
                Level::Warn,
                "repertoire",
                &format!("Saving a line's review failed: {}", cause.message),
            ),
        })
        .detach();
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
        let saved = cx.global::<AppCore>().request(calls::RecordRepertoireMiss {
            key: json!(key),
            fen: json!(fen),
        });
        self.keep_misses(saved, "saving a repertoire miss", cx);
    }

    /// Whether this repertoire and side has misses to forget.
    fn has_misses(&self, cx: &App) -> bool {
        self.misses[&self.miss_key(cx)]
            .as_object()
            .is_some_and(|table| !table.is_empty())
    }

    /// Forget this repertoire's misses at once; Undo puts them back.
    fn clear_misses(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.miss_key(cx);
        let forgotten = self
            .misses
            .as_object_mut()
            .and_then(|misses| misses.remove(&key))
            .filter(|table| table.as_object().is_some_and(|table| !table.is_empty()));
        let saved = cx
            .global::<AppCore>()
            .request(calls::ClearRepertoireMisses { key: json!(key) });
        self.keep_misses(saved, "clearing repertoire misses", cx);
        let Some(forgotten) = forgotten else {
            return;
        };
        let page = cx.entity().downgrade();
        window.push_notification(
            widgets::action_notification(
                Notification::new().title("Mistakes forgotten"),
                "undo-forget-mistakes",
                "Undo",
                move |_, cx| {
                    let (key, forgotten) = (key.clone(), forgotten.clone());
                    let _ = page.update(cx, |page, cx| {
                        let saved =
                            cx.global::<AppCore>()
                                .request(calls::RestoreRepertoireMisses {
                                    key: json!(key),
                                    misses: forgotten,
                                });
                        page.keep_misses(saved, "restoring repertoire misses", cx);
                    });
                },
            ),
            cx,
        );
    }

    fn keep_misses(
        &mut self,
        saved: impl std::future::Future<Output = Result<Value, kchess_contracts::CoreError>> + 'static,
        what: &'static str,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| match saved.await {
            Ok(misses) => {
                let _ = this.update(cx, |this, cx| {
                    this.misses = misses;
                    cx.notify();
                });
            }
            Err(cause) => crate::log::write(
                Level::Warn,
                "studies",
                &format!("{what} failed: {}", cause.message),
            ),
        })
        .detach();
    }

    /// Keep the drilled opening, with the replies found for it, as a study to edit or drill later.
    fn save_opening(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(mut tree), Some(opening)) = (self.drill.clone(), Self::opening(cx)) else {
            return;
        };
        let mut headers = Map::new();
        headers.insert("Event".into(), json!(opening.name));
        tree.headers = Some(headers);
        let library = cx.global::<StudyHandle>().0.clone();
        let name = library.read(cx).fresh_name(&opening.name);
        self.saving = true;
        cx.notify();
        let saved = library
            .read(cx)
            .command(json!({ "op": "save", "name": name, "pgn": tree.pgn_text() }));
        cx.spawn_in(window, async move |this, cx| {
            let saved = saved.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match saved {
                    Ok(result) => {
                        library.update(cx, |library, cx| {
                            library.apply(&result);
                            cx.notify();
                        });
                        window.push_notification("Saved to Studies", cx);
                    }
                    Err(cause) => {
                        crate::log::write(
                            Level::Warn,
                            "opening-trainer",
                            &format!("Saving the opening failed: {cause}"),
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
            let position = kchess_domain::rules::setup_start(
                kchess_domain::variant::Variant::Standard,
                &node.fen,
            )
            .unwrap_or_default();
            let last = node
                .uci
                .get(0..2)
                .zip(node.uci.get(2..4))
                .and_then(|(from, to)| Some((from.parse().ok()?, to.parse().ok()?)));
            let active = to_play(node, color);
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
                    Some("Drill an opening, or lines you saved as a study"),
                    &palette,
                )))
                .into_any_element();
        }
        let drills_opening = Self::drills_opening(cx);
        let color = Self::color(cx);
        let your_turn = self.your_turn(cx);
        let board_size = self.layout.board_size(window);
        let board_column = v_flex().w(board_size).flex_none().gap(widgets::STACK_GAP);
        let board_column = if self.at.is_some() {
            board_column
                .child(crate::board::cached(&self.board))
                .child(widgets::lone_entry(widgets::move_entry(
                    &self.move_input,
                    your_turn,
                    &palette,
                )))
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
            "After the named line you play the most played move on Lichess, and KChess answers with popular replies. Lines due for review come first; lines you got wrong come up more often."
        } else {
            "KChess answers with moves from your study. Lines due for review come first; lines you got wrong come up more often."
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
        let summary = self.summary(cx);
        let schedule = schedule_text(&summary, now_ms());
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
                            .text_size(px(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette.highlighted)
                            .child("Repertoire"),
                    )
                    .child(
                        h_flex()
                            .gap(px(4.))
                            .child(help)
                            .child(widgets::view_toggles(cx)),
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
                panel.child(widgets::loading("Loading popular replies…", &palette))
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
                        widgets::StatusBanner::new(
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
                        )
                        .when_some(
                            feedback.detail.clone().map(SharedString::from),
                            |banner, value| banner.detail(value),
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
            .children(schedule.map(|text| {
                div()
                    .id("repertoire-due")
                    .test_support()
                    .aria_label(SharedString::from(text.clone()))
                    .text_size(px(14.))
                    .text_color(palette.text_muted)
                    .child(text)
            }))
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
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.reviewing = false;
                                this.start_line(cx)
                            })),
                    )
                    .child(
                        Button::new("review-due")
                            .outline()
                            .icon(icon("calendar-days"))
                            .label("Review due lines")
                            .disabled(summary.due == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.review_due(cx))),
                    )
                    .child(
                        Button::new("forget-mistakes")
                            .ghost()
                            .icon(icon("eraser"))
                            .label("Forget mistakes")
                            .disabled(!self.has_misses(cx))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.clear_misses(window, cx)),
                            ),
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
                    .px(px(12.))
                    .py(px(6.))
                    .border_l_2()
                    .border_color(crate::theme::tint(palette.primary, 50.))
                    .rounded_r(radii::SM)
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

/// Practice → Repertoire on a study.
pub(crate) fn train_study(id: String, window: &mut Window, cx: &mut App) {
    crate::prefs::set(cx, STUDY, id);
    super::open_tab(super::Tab::Openings, window, cx);
}

/// Practice → Repertoire on an opening.
pub(crate) fn train_opening(
    choice: &crate::pages::openings::OpeningChoice,
    window: &mut Window,
    cx: &mut App,
) {
    crate::prefs::set(cx, OPENING_CHOICE, choice);
    crate::prefs::set(cx, STUDY, "opening");
    super::open_tab(super::Tab::Openings, window, cx);
}

#[cfg(test)]
mod tests {
    use super::{
        Chapter, TreeNode, due_in, leads_to, leaves, merge, pick_weak, same_castle, schedule_text,
        study_roots, to_play, weakness,
    };
    use kchess_domain::training::repetition::Summary;
    use serde_json::json;
    use shakmaty::Color;

    fn chapter(pgn: &str) -> Chapter {
        Chapter {
            id: pgn.into(),
            name: pgn.into(),
            pgn: pgn.into(),
        }
    }

    /// The opponent's reply that ended a line left the board on the player's turn: a move in
    /// the pause before the next line counted as wrong ("Expected nothing (line ended)") and
    /// marked the position as missed.
    #[test]
    fn a_finished_line_has_nothing_to_play() {
        let roots = study_roots(&[chapter("1. e4 e5")]);
        let e4 = &roots[0].children[0];
        let e5 = &e4.children[0];
        assert!(to_play(&roots[0], Color::White));
        assert!(to_play(e4, Color::Black));
        assert!(!to_play(e5, Color::White));
        assert!(!to_play(e5, Color::Black));
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

    #[test]
    fn each_leaf_is_a_line() {
        let roots = study_roots(&[
            chapter("1. e4 e5 2. Nf3"),
            chapter("1. e4 c5"),
            chapter("1. d4"),
        ]);
        assert_eq!(leaves(&roots[0]), ["e2e4 e7e5 g1f3", "e2e4 c7c5", "d2d4"]);
        assert!(leads_to("e2e4", "e2e4 e7e5"));
        assert!(leads_to("e2e4 e7e5", "e2e4 e7e5"));
        assert!(!leads_to("e2e4 e7e6", "e2e4 e7e5 g1f3"));
        assert!(leaves(&TreeNode::new(None)).is_empty());
    }

    #[test]
    fn the_schedule_reads_as_due_lines_or_the_next_one() {
        let now = 1_000_000_000.;
        let summary = |due, next_due| Summary {
            due,
            new: 0,
            next_due,
        };
        assert_eq!(schedule_text(&summary(0, None), now), None);
        assert_eq!(
            schedule_text(&summary(1, Some(now)), now).as_deref(),
            Some("1 line due")
        );
        assert_eq!(
            schedule_text(&summary(3, None), now).as_deref(),
            Some("3 lines due")
        );
        assert_eq!(
            schedule_text(&summary(0, Some(now + 86_400_000.)), now).as_deref(),
            Some("Next line due in 1 day")
        );
        assert_eq!(due_in(now + 30_000., now), "in 1 min");
        assert_eq!(due_in(now + 3. * 3_600_000., now), "in 3 h");
        assert_eq!(due_in(now + 6. * 86_400_000., now), "in 6 days");
    }
}
