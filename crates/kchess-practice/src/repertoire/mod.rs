//! Repertoire: drill a repertoire. Any saved study is a
//! tree of moves; KChess plays the other side's moves from it (lines due for review first, then
//! favouring the lines you get wrong) and you answer with one of yours. Each line, root to leaf,
//! is scheduled by spaced repetition: a line finished without a mistake comes back later and
//! later, a missed one at once. One chapter can be drilled, or all of them: chapters from the
//! same starting position are one tree. An opening chosen on the Openings page is drilled without a
//! study: its named line, then the explorer's popular moves for a few more.

use crate::game::tree::TreePgn as _;
use kchess_contracts::calls;
use kchess_contracts::check::{Fen, MissCounts, Position, RepertoireKey, RepertoireLine};
use kchess_contracts::types::{
    Keyed, Library, LookupOptions, PositionLookupKind, RepertoireMisses, RepertoireSchedule, Study,
    StudyChapter as Chapter, StudyCommand,
};
use std::collections::VecDeque;
use std::time::Duration;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::Level;
use kchess_domain::training::openings::{self, ExplorerMove};
use kchess_domain::training::repetition;
use serde_json::{Map, json};
use shakmaty::Color;

use super::run;
use crate::board::{Arrow, BoardEvent, BoardState, ChessBoard};
use crate::core::AppCore;
use crate::game::STANDARD_FEN;
use crate::game::tree::{TreeNode, child_path, comment_text};
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::studies::StudyHandle;
use crate::theme::{Palette, radii};
use crate::voice::moves::{
    Access, BoardBar, Offer, Said, Turn, VoiceMoves, board_bar, offered_grammar,
};
use crate::widgets::{self, Choice, Pill, Tone};
use kchess_board::game::OpeningChoice;
use kchess_domain::voice::PuzzleCommand;
use kchess_games::random_unit;
use kchess_nav::page::Page;
use kchess_nav::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout, tab_bar_height};
use kchess_ui::icon;

pub mod check;
mod lines;
pub(crate) use lines::due_in_study;
pub mod your_games;
use lines::*;

const STUDY: &str = "kchess:repertoire-study";
/// Whether there are game databases but Your games, whether there is Your games, and each
/// player's from Player's games: its id and the name a menu gives them.
type Sources = (bool, bool, Vec<(String, String)>);
/// Your games' database id, as the core names it.
const OWN_DATABASE: &str = "own";
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

/// The repertoire trainer's actions, as its buttons offer them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RepertoireAction {
    ShowMove,
    Restart,
    Review,
    Forget,
    Save,
}

/// What the trees are built from: the study as last edited, its chapter, the side and the
/// opening.
#[derive(Clone, PartialEq)]
struct Choices {
    edited: Option<serde_json::Number>,
    study: String,
    chapter: String,
    color: Color,
    opening: Option<OpeningChoice>,
}

pub struct Repertoire {
    studies: Vec<Study>,
    misses: RepertoireMisses,
    /// Every repertoire's spaced-repetition schedule: key → line → card.
    schedule: RepertoireSchedule,
    /// The latest read of the misses and schedule: an earlier one answering after it, or after a
    /// change of this window's own, would put back what they replaced.
    misses_read: crate::loading::Epoch,
    /// The line being drilled, when one was chosen (a due line): the opponent's moves follow it.
    target: Option<String>,
    /// Whether this line had a wrong move or a shown one: it is then answered wrong.
    line_missed: bool,
    /// Only due lines come up, until none is left.
    reviewing: bool,
    /// An opening's drill tree, and the opening and side it was built for.
    drill: Option<TreeNode>,
    drill_for: Option<(Option<OpeningChoice>, Color)>,
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
    /// A repertoire is being made from games (`make_from_games`).
    making: bool,
    /// The game databases its lines are checked against and made from (`databases`).
    sources: Sources,
    reply: Option<Task<()>>,
    board: Entity<ChessBoard>,
    layout: PlayLayout,
    move_box: crate::game::surface::MoveBox,
    /// Spoken moves, and Show move and Restart line said aloud.
    voice: VoiceMoves,
    study_select: Entity<SelectState<Vec<Choice>>>,
    chapter_select: Entity<SelectState<Vec<Choice>>>,
    /// The selects' items are rebuilt on the next frame, where there is a window.
    stale_selects: bool,
    /// The choices the trees were built for: another page (Openings' Train) may change them.
    built_for: Option<Choices>,
    _subscriptions: Vec<Subscription>,
}

impl Repertoire {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Repertoire {
        let board = cx.new(|_| ChessBoard::new(BoardState::start()));
        let (move_box, typed) = crate::game::surface::MoveBox::new(Self::typed_move, window, cx);
        let study_select = cx.new(|cx| SelectState::new(Vec::<Choice>::new(), None, window, cx));
        let chapter_select = cx.new(|cx| SelectState::new(Vec::<Choice>::new(), None, window, cx));
        let settings = cx.global::<SettingsHandle>().0.clone();
        let trainer = cx.entity().downgrade();
        let voice = crate::voice::handle(cx);
        let subscriptions = vec![
            cx.subscribe(&voice, |this, _, event: &crate::voice::VoiceEvent, cx| {
                this.voice_heard(event, cx)
            }),
            cx.observe(&voice, |_, _, cx| cx.notify()),
            // Another window practised a repertoire: its misses and schedule are the profile's.
            crate::core::profile_changes(
                cx,
                &[kchess_contracts::types::ProfileArea::Repertoire],
                move |cx| {
                    trainer
                        .update(cx, |trainer, cx| trainer.load_misses(cx))
                        .ok();
                },
            ),
            cx.subscribe(&board, |this, _, event: &BoardEvent, cx| {
                if let BoardEvent::Move(uci) = event {
                    this.play(uci, cx);
                }
            }),
            typed,
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
                this.follow_choices(cx);
                cx.notify();
            }),
            cx.observe_global::<crate::prefs::ViewPreferences>(|this, cx| this.follow_choices(cx)),
            // A database added or removed changes what Check and From games offer.
            cx.observe(
                &cx.global::<kchess_board::game_db::GameDbHandle>().0.clone(),
                |this, _, cx| {
                    let sources = Self::databases(cx);
                    if sources != this.sources {
                        this.sources = sources;
                        cx.notify();
                    }
                },
            ),
            // The board follows blindfold mode.
            cx.subscribe(&settings, |this, _, _: &SettingsChanged, cx| {
                this.redraw(cx)
            }),
        ];
        let mut drill = Repertoire {
            studies: Self::studies_of(cx),
            misses: Keyed::default(),
            schedule: Keyed::default(),
            misses_read: crate::loading::Epoch::default(),
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
            making: false,
            sources: Self::databases(cx),
            reply: None,
            board,
            layout: PlayLayout::under(tab_bar_height::<crate::Tab>),
            move_box,
            voice: VoiceMoves::new(crate::voice::VoiceSource::Repertoire, cx),
            study_select,
            chapter_select,
            stale_selects: true,
            built_for: None,
            _subscriptions: subscriptions,
        };
        drill.load_misses(cx);
        drill.rebuild(cx);
        drill
    }

    /// The opening whose drill tree is built, when one is drilled.
    #[cfg(any(test, feature = "test-support"))]
    pub fn drilled_opening(&self) -> Option<&str> {
        let (opening, _) = self.drill_for.as_ref()?;
        opening.as_ref().map(|opening| opening.name.as_str())
    }

    /// How many repertoires have missed moves.
    #[cfg(any(test, feature = "test-support"))]
    pub fn missed_repertoires(&self) -> usize {
        self.misses.len()
    }

    /// The missed moves, as the core keeps them.
    fn load_misses(&mut self, cx: &mut Context<Self>) {
        let turn = self.misses_read.start();
        let library = cx.global::<AppCore>().request(calls::PracticeLibrary {});
        cx.spawn(async move |this, cx| match library.await {
            Ok(library) => {
                this.update(cx, |this, cx| this.adopt_misses(turn, library, cx))
                    .ok();
            }
            Err(cause) => crate::log::write(
                Level::Warn,
                "repertoire",
                &format!("Reading the missed moves failed: {}", cause.message),
            ),
        })
        .detach();
    }

    /// The misses and schedule `turn` read, if it is the latest read.
    fn adopt_misses(
        &mut self,
        turn: crate::loading::Turn,
        library: Library,
        cx: &mut Context<Self>,
    ) {
        if !self.misses_read.is_current(turn) {
            return;
        }
        self.misses = library.repertoire_misses;
        self.schedule = library.repertoire_schedule;
        // A due line comes first, also when the schedule arrived after the first line.
        if self.target.is_none() && self.correct + self.wrong == 0 && self.due_line(cx).is_some() {
            self.start_line(cx);
        }
        cx.notify();
    }

    /// Starts a read of the misses whose answer the test gives (`answer_misses_read`).
    #[cfg(any(test, feature = "test-support"))]
    pub fn start_misses_read(&mut self) -> crate::loading::Turn {
        self.misses_read.start()
    }

    /// Answers a read of the misses `start_misses_read` started.
    #[cfg(any(test, feature = "test-support"))]
    pub fn answer_misses_read(
        &mut self,
        turn: crate::loading::Turn,
        library: Library,
        cx: &mut Context<Self>,
    ) {
        self.adopt_misses(turn, library, cx);
    }

    /// Records a miss at `fen` in the repertoire drilled, as a wrong move does.
    #[cfg(any(test, feature = "test-support"))]
    pub fn miss_at(&mut self, fen: String, cx: &mut Context<Self>) {
        self.record_miss(fen, cx);
    }

    /// The saved studies, as the library has them.
    fn studies_of(cx: &App) -> Vec<Study> {
        cx.global::<StudyHandle>().0.read(cx).items().to_vec()
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

    /// The repertoire's moves checked against the game databases, in a dialog.
    pub fn check_against_databases(&self, window: &mut Window, cx: &mut App) {
        let steps = check::steps(&self.roots, Self::color(cx));
        check::open(steps, self.target(cx), window, cx);
    }

    /// The repertoire followed through your games with its side, in a dialog.
    pub fn check_against_your_games(&self, window: &mut Window, cx: &mut App) {
        let steps = check::every_step(&self.roots);
        let whose = your_games::Whose::Yours;
        your_games::open(steps, self.target(cx), Self::color(cx), whose, window, cx);
    }

    /// The repertoire followed through the games of `name` (their database from Player's games,
    /// `database`) with the other side, in a dialog: what they play where it waits for them.
    pub fn check_against_player(
        &self,
        database: String,
        name: String,
        window: &mut Window,
        cx: &mut App,
    ) {
        let steps = check::every_step(&self.roots);
        let whose = your_games::Whose::Player { database, name };
        your_games::open(steps, self.target(cx), Self::color(cx), whose, window, cx);
    }

    /// The study drilled and the chapters its lines come from, which a check's moves join.
    fn target(&self, cx: &App) -> check::Target {
        let chapters = match self.chapter(cx) {
            Some(chapter) => vec![chapter.clone()],
            None => self
                .study(cx)
                .map(|study| study.chapters.clone())
                .unwrap_or_default(),
        };
        check::Target {
            study: Self::study_id(cx),
            chapters,
        }
    }

    /// The game databases to check the lines against: whether there are any but Your games,
    /// whether there is Your games, and each player's from Player's games (its id and the name its
    /// menu item gives the player: their username, or the database's name when two share one).
    fn databases(cx: &App) -> Sources {
        use kchess_contracts::types::DatabaseSource;
        let handle = cx.global::<kchess_board::game_db::GameDbHandle>().0.clone();
        let databases = handle.read(cx).databases();
        let players: Vec<(&str, &str, &str)> = databases
            .iter()
            .filter(|database| database.source == DatabaseSource::Player)
            .filter_map(|database| {
                let player = database.player.as_ref()?;
                Some((
                    database.id.as_str(),
                    player.username.as_str(),
                    database.name.as_str(),
                ))
            })
            .collect();
        let players = players
            .iter()
            .map(|&(id, username, name)| {
                let shared = players
                    .iter()
                    .filter(|(_, other, _)| other.eq_ignore_ascii_case(username))
                    .count()
                    > 1;
                (
                    id.to_string(),
                    if shared { name } else { username }.to_string(),
                )
            })
            .collect();
        (
            databases
                .iter()
                .any(|database| database.source != DatabaseSource::Own),
            databases
                .iter()
                .any(|database| database.source == DatabaseSource::Own),
            players,
        )
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

    fn choices(cx: &App) -> Choices {
        let study = Self::study_id(cx);
        Choices {
            edited: cx
                .global::<StudyHandle>()
                .0
                .read(cx)
                .study(&study)
                .map(|study| study.updated_at.clone()),
            study,
            chapter: crate::prefs::get::<String>(cx, CHAPTER, ALL_CHAPTERS.into()),
            color: Self::color(cx),
            opening: Self::opening(cx),
        }
    }

    /// The trees follow choices made elsewhere (Openings' Train, a study edited), when they
    /// change rather than when the page is next drawn.
    fn follow_choices(&mut self, cx: &mut Context<Self>) {
        if self.built_for.as_ref() != Some(&Self::choices(cx)) {
            self.stale_selects = true;
            self.rebuild(cx);
            cx.notify();
        }
    }

    /// The trees for what is chosen, the opening's built first when it changed; then a new line.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.built_for = Some(Self::choices(cx));
        if Self::drills_opening(cx) {
            let wanted = (Self::opening(cx), Self::color(cx));
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

    /// The named line, then the explorer's most played move for the player and
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
                let yours = kchess_domain::rules::fen_turn(&fen) == color;
                let replies = openings::opening_replies(&counts, yours);
                for uci in replies {
                    if let Some(child) = root.add_move(&path, &uci)
                        && depth + 1 < EXTRA_PLIES
                    {
                        queue.push_back((child, depth + 1));
                    }
                }
            }
            this.update(cx, |this, cx| {
                this.drill = Some(root);
                this.note = note;
                this.preparing = false;
                this.roots = this.drill.clone().into_iter().collect();
                this.start_line(cx);
            })
            .ok();
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
            this.update(cx, step).ok();
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
        let table = self.schedule.get(&self.miss_key(cx));
        repetition::summary(
            lines.iter().map(|(_, _, id)| id.as_str()),
            table,
            kchess_games::now_ms(),
        )
    }

    /// The most overdue line, if any is due.
    fn due_line(&self, cx: &App) -> Option<(usize, String)> {
        let lines = self.lines();
        let ids: Vec<&str> = lines.iter().map(|(_, _, id)| id.as_str()).collect();
        let table = self.schedule.get(&self.miss_key(cx));
        let index = repetition::most_overdue(&ids, table, kchess_games::now_ms())?;
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
                let table = self.misses.get(&self.miss_key(cx));
                let roots: Vec<&TreeNode> = self.roots.iter().collect();
                crate::game::drawn(random_unit())
                    .and_then(|draw| pick_weak(&roots, table, draw))
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
        let table = self.misses.get(&self.miss_key(cx)).cloned();
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
                    .and_then(|draw| pick_weak(&children, table.as_ref(), draw))
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
            // No line to drill yet: the board played the move at once, and it goes back.
            self.board.update(cx, |board, cx| board.undo_move(cx));
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
            .find(|child| child.uci == uci || same_move_at(&node.fen, &child.uci, uci))
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
        let variant = kchess_domain::variant::Variant::Standard;
        let position = kchess_domain::rules::setup_start(variant, &node.fen);
        if let Some(uci) = self.move_box.take(position.as_ref(), window, cx) {
            self.play(&uci, cx);
        }
    }

    /// The trainer's buttons voice can press: Show move (“show move” or “hint”) and Restart
    /// line (“restart line”, “retry” or “try again”). A line ended goes on to the next by itself,
    /// so there is no “next”; Forget mistakes and Save as study stay buttons.
    fn voice_offers(&self, cx: &App) -> Vec<Offer> {
        let actions = self.actions(cx);
        let show = actions.contains(&RepertoireAction::ShowMove);
        let restart = actions.contains(&RepertoireAction::Restart);
        [PuzzleCommand::ShowMove, PuzzleCommand::Hint]
            .map(|command| Offer::new(command, show, "Move shown", "No move to show now"))
            .into_iter()
            .chain(
                [PuzzleCommand::RestartLine, PuzzleCommand::Retry].map(|command| {
                    Offer::new(command, restart, "Line restarted", "Nothing to restart")
                }),
            )
            .collect()
    }

    /// A spoken move belongs to this position of the line; another drops a choice waiting.
    fn sync_voice(&mut self, cx: &mut Context<Self>) {
        let context = self
            .at
            .as_ref()
            .map(|(root, path)| format!("{root}:{path}"))
            .unwrap_or_default();
        let grammar = offered_grammar(&self.voice_offers(cx));
        let listening = self.your_turn(cx);
        self.voice.sync(&context, None, grammar, listening, cx);
    }

    fn voice_heard(&mut self, event: &crate::voice::VoiceEvent, cx: &mut Context<Self>) {
        let fen = self.node().map(|node| node.fen.clone()).unwrap_or_default();
        let setup = kchess_domain::variant::Setup::standard_at(&fen);
        let Some(heard) = self.voice.heard(event, &fen, self.your_turn(cx), cx) else {
            return;
        };
        let turn = Turn {
            taking: self.your_turn(cx),
            idle: None,
            immediate: false,
        };
        let offers = self.voice_offers(cx);
        match self.voice.hear_offered(&heard, &setup, turn, &offers, cx) {
            Said::Play(uci) => self.play(&uci, cx),
            Said::Command(PuzzleCommand::ShowMove | PuzzleCommand::Hint) => self.show_answer(cx),
            Said::Command(PuzzleCommand::RestartLine | PuzzleCommand::Retry) => {
                self.reviewing = false;
                self.start_line(cx)
            }
            Said::Command(PuzzleCommand::Next | PuzzleCommand::Back) | Said::Nothing => {}
        }
        cx.notify();
    }

    /// The moves the board shows as hints or answers.
    #[cfg(any(test, feature = "test-support"))]
    pub fn hint_shown(&self) -> usize {
        self.hint.len()
    }

    /// `text` said clearly, with voice on: as the recognizer's phrase reaches the trainer.
    #[cfg(any(test, feature = "test-support"))]
    pub fn hear(&mut self, text: &str, cx: &mut Context<Self>) {
        let heard = self.voice.control.hear_clearly(text);
        self.voice_heard(&heard, cx);
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
    pub fn actions(&self, cx: &App) -> Vec<RepertoireAction> {
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
        let mut schedule = self.schedule.clone();
        if let Err(cause) =
            repetition::record(&mut schedule, &key, &line, correct, kchess_games::now_ms())
        {
            crate::log::write(
                Level::Warn,
                "repertoire",
                &format!("Scheduling a line failed: {cause}"),
            );
            return;
        }
        self.schedule = schedule;
        self.misses_read.invalidate();
        let saved = cx.global::<AppCore>().request(calls::ReviewRepertoireLine {
            key: RepertoireKey(key),
            line: RepertoireLine(line),
            correct,
        });
        cx.spawn(async move |this, cx| match saved.await {
            Ok(schedule) => {
                this.update(cx, |this, cx| {
                    this.schedule = schedule;
                    cx.notify();
                })
                .ok();
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
        let table = self.misses.entry(&key);
        let count = table
            .get(&fen)
            .and_then(serde_json::Number::as_f64)
            .unwrap_or(0.)
            + 1.;
        table.insert(fen.clone(), serde_json::Number::from(count as u64));
        let saved = cx.global::<AppCore>().request(calls::RecordRepertoireMiss {
            key: RepertoireKey(key),
            fen: Position(fen),
        });
        self.keep_misses(saved, "saving a repertoire miss", cx);
    }

    /// Whether this repertoire and side has misses to forget.
    fn has_misses(&self, cx: &App) -> bool {
        self.misses
            .get(&self.miss_key(cx))
            .is_some_and(|table| !table.is_empty())
    }

    /// Forget this repertoire's misses at once; once forgotten, Undo puts back what the core
    /// removed (with any miss another window had recorded), and only then.
    fn clear_misses(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.miss_key(cx);
        self.misses.0.retain(|(table, _)| *table != key);
        self.misses_read.invalidate();
        cx.notify();
        let cleared = cx
            .global::<AppCore>()
            .request(calls::ClearRepertoireMisses {
                key: RepertoireKey(key.clone()),
            });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let cleared = match cleared.await {
                Ok(cleared) => cleared,
                Err(cause) => {
                    crate::log::write(
                        Level::Warn,
                        "studies",
                        &format!("clearing repertoire misses failed: {}", cause.message),
                    );
                    return;
                }
            };
            let forgotten: Keyed<f64> = Keyed(
                cleared
                    .removed
                    .0
                    .into_iter()
                    .map(|(fen, count)| (fen, count.as_f64().unwrap_or(0.)))
                    .collect(),
            );
            window_handle
                .update(cx, |_, window, cx| {
                    this.update(cx, |this, cx| {
                        this.misses = cleared.misses;
                        if !forgotten.is_empty() {
                            this.offer_undo(key, forgotten, window, cx);
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .ok();
        })
        .detach();
    }

    /// The notification whose Undo puts `forgotten` back as `key`'s misses.
    fn offer_undo(
        &mut self,
        key: String,
        forgotten: Keyed<f64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page = cx.entity().downgrade();
        window.push_notification(
            widgets::action_notification(
                Notification::new().title("Mistakes forgotten"),
                "undo-forget-mistakes",
                "Undo",
                move |_, cx| {
                    let (key, forgotten) = (key.clone(), forgotten.clone());
                    page.update(cx, |page, cx| {
                        let saved =
                            cx.global::<AppCore>()
                                .request(calls::RestoreRepertoireMisses {
                                    key: RepertoireKey(key),
                                    misses: MissCounts(forgotten),
                                });
                        page.keep_misses(saved, "restoring repertoire misses", cx);
                    })
                    .ok();
                },
            ),
            cx,
        );
    }

    fn keep_misses(
        &mut self,
        saved: impl std::future::Future<Output = Result<RepertoireMisses, kchess_contracts::CoreError>>
        + 'static,
        what: &'static str,
        cx: &mut Context<Self>,
    ) {
        // A read sent before this write answers with the misses from before it.
        self.misses_read.invalidate();
        cx.spawn(async move |this, cx| match saved.await {
            Ok(misses) => {
                this.update(cx, |this, cx| {
                    this.misses = misses;
                    cx.notify();
                })
                .ok();
            }
            Err(cause) => crate::log::write(
                Level::Warn,
                "studies",
                &format!("{what} failed: {}", cause.message),
            ),
        })
        .detach();
    }

    /// A repertoire of `color`, made from the games of `database` (Your games, or `name`'s from
    /// Player's games) and saved as a study, which is then drilled with that side.
    pub fn make_from_games(
        &mut self,
        database: String,
        name: Option<String>,
        color: Color,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let side = if color == Color::White {
            "White"
        } else {
            "Black"
        };
        let made = cx
            .global::<AppCore>()
            .request(calls::GameDbRepertoireBuild {
                color: kchess_contracts::check::Side(color),
                database: kchess_contracts::check::DatabaseId(database),
            });
        let library = cx.global::<StudyHandle>().0.clone();
        self.making = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let whose = name
                .as_deref()
                .map_or("your".to_string(), |name| format!("{name}'s"));
            let built = match made.await {
                Ok(built) if built.moves > 0 => Ok(built),
                Ok(built) if built.games == 0 => Err(format!("None of {whose} games as {side}")),
                Ok(_) => Err(format!("Too few of {whose} games as {side}")),
                Err(cause) => {
                    crate::log::write(
                        Level::Warn,
                        "repertoire",
                        &format!("Making a repertoire from games failed: {cause}"),
                    );
                    Err("Could not make the repertoire".to_string())
                }
            };
            let saved = match built {
                Ok(built) => {
                    let title = match &name {
                        Some(name) => format!("{name} as {side}"),
                        None => format!("Your games as {side}"),
                    };
                    let save = this.update(cx, |_, cx| {
                        let library = library.read(cx);
                        library.command(StudyCommand::Save {
                            name: library.fresh_name(&title),
                            pgn: built.pgn,
                            id: None,
                            chapter_id: None,
                        })
                    });
                    match save {
                        Ok(save) => Some(save.await),
                        Err(_) => return,
                    }
                }
                Err(message) => {
                    this.update_in(cx, |this, window, cx| {
                        this.making = false;
                        window.push_notification(message, cx);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            this.update_in(cx, |this, window, cx| {
                this.making = false;
                match saved {
                    Some(Ok(result)) => {
                        library.update(cx, |library, cx| {
                            library.apply(&result);
                            cx.notify();
                        });
                        if let Some(id) = result.id.clone() {
                            let side = if color == Color::White {
                                "white"
                            } else {
                                "black"
                            };
                            crate::prefs::set(cx, COLOR, side);
                            train_study(id, window, cx);
                        }
                        window.push_notification("Saved to Studies", cx);
                    }
                    Some(Err(cause)) => {
                        crate::log::write(
                            Level::Warn,
                            "repertoire",
                            &format!("Saving the repertoire made from games failed: {cause}"),
                        );
                        window.push_notification("Could not save the study", cx);
                    }
                    None => {}
                }
                cx.notify();
            })
            .ok();
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
        let saved = library.read(cx).command(StudyCommand::Save {
            name,
            pgn: tree.pgn_text(),
            id: None,
            chapter_id: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let saved = saved.await;
            this.update_in(cx, |this, window, cx| {
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
            })
            .ok();
        })
        .detach();
    }

    fn redraw(&mut self, cx: &mut Context<Self>) {
        self.sync_voice(cx);
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
            state.blindfold = crate::settings::frontend(cx).blindfold;
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

    /// From games: a menu of Your games and each player's games from Player's games, to make
    /// a repertoire from; none without them. With `sides`, each is offered with either side
    /// (where the side played is not shown); without, with the side played.
    fn games_button(&self, sides: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (_, yours, players) = self.sources.clone();
        let mut sources: Vec<(String, Option<String>)> = Vec::new();
        if yours {
            sources.push((OWN_DATABASE.to_string(), None));
        }
        sources.extend(players.into_iter().map(|(id, name)| (id, Some(name))));
        if sources.is_empty() {
            return None;
        }
        let colors = if sides {
            vec![Color::White, Color::Black]
        } else {
            vec![Self::color(cx)]
        };
        let page = cx.entity().downgrade();
        let button = Button::new("repertoire-from-games");
        // Among the empty page's buttons, outlined as they are.
        let button = if sides {
            button.outline()
        } else {
            button.ghost()
        };
        Some(
            button
                .icon(icon("list-tree"))
                .label("From games")
                .loading(self.making)
                .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                    let items = sources.iter().flat_map(|source| {
                        colors.iter().map(move |color| (source.clone(), *color))
                    });
                    items.fold(menu, |menu, ((database, name), color)| {
                        let whose = name.clone().unwrap_or_else(|| "Your games".to_string());
                        let label = if sides {
                            let side = if color == Color::White {
                                "White"
                            } else {
                                "Black"
                            };
                            format!("{whose} as {side}")
                        } else {
                            whose
                        };
                        let icon_name = if name.is_some() {
                            "user-search"
                        } else {
                            "user"
                        };
                        let page = page.clone();
                        menu.item(PopupMenuItem::new(label).icon(icon(icon_name)).on_click(
                            move |_, window, cx| {
                                let (database, name) = (database.clone(), name.clone());
                                page.update(cx, |this, cx| {
                                    this.make_from_games(database, name, color, window, cx)
                                })
                                .ok();
                            },
                        ))
                    })
                })
                .into_any_element(),
        )
    }

    fn empty_actions(extra: Option<AnyElement>) -> impl IntoElement {
        h_flex()
            .gap(px(8.))
            .flex_wrap()
            .justify_center()
            .child(
                Button::new("browse-openings")
                    .primary()
                    .label("Browse openings")
                    .on_click(|_, window, cx| kchess_nav::shell::open(Page::Openings, window, cx)),
            )
            .child(
                Button::new("open-studies")
                    .outline()
                    .label("Open studies")
                    .on_click(|_, window, cx| kchess_nav::shell::open(Page::Studies, window, cx)),
            )
            .children(extra)
    }

    fn empty(
        title: &'static str,
        description: Option<&'static str>,
        extra: Option<AnyElement>,
        palette: &Palette,
    ) -> Div {
        widgets::empty_state(
            icon("book-marked"),
            title,
            description.map(SharedString::from),
            palette,
        )
        .py(px(32.))
        .child(Self::empty_actions(extra))
    }
}

impl Render for Repertoire {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.stale_selects) {
            self.refresh_selects(window, cx);
        }
        let palette = *cx.global::<Palette>();
        let opening = Self::opening(cx);
        if self.studies.is_empty() && opening.is_none() {
            let from_games = self.games_button(true, cx);
            return div()
                .size_full()
                .px(kchess_nav::play_layout::side_padding(window))
                .pt(px(20.))
                .child(widgets::card(&palette).child(Self::empty(
                    "Nothing to drill yet",
                    Some("Drill an opening, or lines you saved as a study"),
                    from_games,
                    &palette,
                )))
                .into_any_element();
        }
        let drills_opening = Self::drills_opening(cx);
        let (databases, yours, players) = self.sources.clone();
        let color = Self::color(cx);
        let your_turn = self.your_turn(cx);
        // The board's column: the board and the move box's row, once a line is on the board.
        let around = if self.at.is_some() {
            widgets::MOVE_BOX_ROW
        } else {
            px(0.)
        };
        let (board_size, panel_height) = self.layout.board_and_panel(around, window);
        let board_column = v_flex()
            .id("repertoire-column")
            .test_support()
            .w(board_size)
            .flex_none()
            .gap(widgets::STACK_GAP);
        let board_column = if self.at.is_some() {
            let entry = widgets::move_entry(&self.move_box.input, your_turn, &palette);
            let (voice, voice_line) = board_bar(
                &self.voice,
                cx.entity().downgrade(),
                Access {
                    write: |this: &mut Self| &mut this.voice,
                    play: |this: &mut Self, uci, _, cx| this.play(uci, cx),
                },
                BoardBar {
                    id: "repertoire-voice",
                    hint: "Say “E four”, “show move” or “restart line”",
                    active: your_turn,
                    listening: your_turn,
                    grammar: offered_grammar(&self.voice_offers(cx)),
                },
                cx,
            );
            board_column
                .child(crate::board::resizable(&self.board))
                .child(widgets::input_row(entry, voice))
                .children(voice_line)
                .children(self.move_box.message(&palette))
        } else if !(drills_opening && self.preparing) {
            board_column.child(widgets::card(&palette).child(Self::empty(
                "Choose a repertoire",
                None,
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
                    .small()
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
        let schedule = schedule_text(&summary, kchess_games::now_ms());
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
            .id("repertoire-panel")
            .test_support()
            .h(panel_height)
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
                            page.update(cx, |page, cx| page.rebuild(cx)).ok();
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
                    .when(!drills_opening && (databases || yours), |actions| {
                        let page = cx.entity().downgrade();
                        let players = players.clone();
                        actions.child(
                            Button::new("repertoire-check")
                                .ghost()
                                .icon(icon("circle-check"))
                                .label("Check")
                                .disabled(self.roots.is_empty())
                                .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                                    let (mine, theirs) = (page.clone(), page.clone());
                                    let menu = menu
                                        .when(yours, |menu| {
                                            menu.item(
                                                PopupMenuItem::new("Against your games")
                                                    .icon(icon("user"))
                                                    .on_click(move |_, window, cx| {
                                                        mine.update(cx, |this, cx| {
                                                            this.check_against_your_games(
                                                                window, cx,
                                                            )
                                                        })
                                                        .ok();
                                                    }),
                                            )
                                        })
                                        .when(databases, |menu| {
                                            menu.item(
                                                PopupMenuItem::new("Against databases")
                                                    .icon(icon("database"))
                                                    .on_click(move |_, window, cx| {
                                                        theirs
                                                            .update(cx, |this, cx| {
                                                                this.check_against_databases(
                                                                    window, cx,
                                                                )
                                                            })
                                                            .ok();
                                                    }),
                                            )
                                        });
                                    players.iter().fold(menu, |menu, (database, name)| {
                                        let (page, database, name) =
                                            (page.clone(), database.clone(), name.clone());
                                        menu.item(
                                            PopupMenuItem::new(format!("Against {name}"))
                                                .icon(icon("user-search"))
                                                .on_click(move |_, window, cx| {
                                                    let (database, name) =
                                                        (database.clone(), name.clone());
                                                    page.update(cx, |this, cx| {
                                                        this.check_against_player(
                                                            database, name, window, cx,
                                                        )
                                                    })
                                                    .ok();
                                                }),
                                        )
                                    })
                                }),
                        )
                    })
                    .children(self.games_button(false, cx))
                    .when(drills_opening, |actions| {
                        actions.child(
                            Button::new("save-opening")
                                .ghost()
                                .icon(icon("book-plus"))
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
                    .rounded_r(radii::BOX)
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
pub fn train_study(id: String, window: &mut Window, cx: &mut App) {
    crate::prefs::set(cx, STUDY, id);
    super::open_tab(super::Tab::Openings, window, cx);
}

/// Practice → Repertoire on a study's lines for `color`, every chapter, where its due lines
/// come first (Home's Today's practice).
pub fn review_study(id: String, color: Color, window: &mut Window, cx: &mut App) {
    crate::prefs::set(cx, STUDY, id);
    crate::prefs::set(cx, COLOR, kchess_domain::color::name(color));
    crate::prefs::set(cx, CHAPTER, ALL_CHAPTERS);
    super::open_tab(super::Tab::Openings, window, cx);
}

/// Practice → Repertoire on an opening.
pub fn train_opening(
    choice: &kchess_board::game::OpeningChoice,
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
        Chapter, Keyed, TreeNode, due_in, leads_to, leaves, merge, pick_weak, same_move_at,
        schedule_text, study_roots, to_play, weakness,
    };
    use kchess_domain::training::repetition::Summary;
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
        let castles = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
        assert!(same_move_at(castles, "e1h1", "e1g1"));
        assert!(!same_move_at(castles, "e1g1", "e1c1"));
        let black = "r3k2r/8/8/8/8/8/8/R3K2R b KQkq - 0 1";
        assert!(same_move_at(black, "e8c8", "e8a8"));
    }

    /// A rook's move from e1 to h1 was taken for the repertoire's Rg1, as if it castled.
    #[test]
    fn a_rook_move_is_not_castling() {
        assert!(!same_move_at(
            "1k6/8/8/8/8/8/8/K3R3 w - - 0 1",
            "e1h1",
            "e1g1"
        ));
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
        let table = Keyed(vec![(fen, serde_json::Number::from(3))]);
        let children: Vec<&TreeNode> = tree.children.iter().collect();
        assert_eq!(weakness(children[0], Some(&table)), 3.);
        // Weights 7 and 1: draws below 7/8 pick the weak line.
        assert_eq!(pick_weak(&children, Some(&table), 0.8), Some(0));
        assert_eq!(pick_weak(&children, Some(&table), 0.9), Some(1));
        assert_eq!(pick_weak(&[], Some(&table), 0.5), None);
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
