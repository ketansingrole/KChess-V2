//! Practice → Repertoire → Check against databases: each move the repertoire plays, looked up in
//! the game databases (Your games aside), and the ones to look at listed: a move no game played,
//! one few games played, and one that scored well below the move most played instead. A listed
//! move opens on the analysis board.

use std::collections::HashMap;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_board::analysis::{self, Open};
use kchess_contracts::calls;
use kchess_contracts::check::{MOST_STEPS, RepertoireSteps};
use kchess_contracts::types::{
    GameDbRepertoireCheck, GameDbRepertoireMove, GameDbVerdict, RepertoireStep, StudyChapter,
};
use kchess_domain::training::repertoire_check::{Counts, ENOUGH};
use kchess_ui::icon;
use shakmaty::Color;

use crate::core::AppCore;
use crate::game::STANDARD_FEN;
use crate::game::tree::TreeNode;
use crate::loading::Loading;
use crate::theme::{Palette, radii};
use crate::widgets::{self, KeyboardFocus as _, Pressable as _};
use kchess_domain::tree::numbered_line;

/// Moves from a tree's start, each its ply, SAN and UCI.
type Line = Vec<(u32, String, String)>;

/// A position where the repertoire's side is to move: the moves that reach it from its tree's
/// start, and the repertoire's moves there.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    /// The tree's start.
    pub start: String,
    pub fen: String,
    /// Its half-moves since the game's start.
    pub ply: u32,
    /// The moves from the start.
    pub line: Line,
    /// The repertoire's moves: UCI, SAN and the position after.
    pub moves: Vec<(String, String, String)>,
}

/// Every position of `roots` where `color` is to move and the repertoire plays on, the first way
/// each is reached (a transposition adds its moves to it), in the trees' order: at most
/// `MOST_STEPS`.
pub fn steps(roots: &[TreeNode], color: Color) -> Vec<Step> {
    collect(roots, |node| super::to_play(node, color))
}

/// Every position of `roots` where the repertoire goes on, either side to move, as `steps`.
pub fn every_step(roots: &[TreeNode]) -> Vec<Step> {
    collect(roots, |node| !node.children.is_empty())
}

fn collect(roots: &[TreeNode], keep: impl Fn(&TreeNode) -> bool) -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for root in roots {
        let mut stack: Vec<(&TreeNode, Line)> = vec![(root, Vec::new())];
        while let Some((node, line)) = stack.pop() {
            if keep(node) {
                let moves = node
                    .children
                    .iter()
                    .map(|child| (child.uci.clone(), child.san.clone(), child.fen.clone()));
                match at.get(&node.fen) {
                    Some(&index) => {
                        let step = &mut steps[index];
                        for played in moves {
                            if !step.moves.iter().any(|(uci, ..)| *uci == played.0) {
                                step.moves.push(played);
                            }
                        }
                    }
                    None if steps.len() < MOST_STEPS => {
                        at.insert(node.fen.clone(), steps.len());
                        steps.push(Step {
                            start: root.fen.clone(),
                            fen: node.fen.clone(),
                            ply: node.ply,
                            line: line.clone(),
                            moves: moves.collect(),
                        });
                    }
                    None => {}
                }
            }
            for child in node.children.iter().rev() {
                let mut next = line.clone();
                next.push((node.ply, child.san.clone(), child.uci.clone()));
                stack.push((child, next));
            }
        }
    }
    steps
}

fn counts(played: &GameDbRepertoireMove) -> Counts {
    Counts {
        games: played.games,
        white: played.white,
        draws: played.draws,
        black: played.black,
    }
}

fn percent(part: u64, whole: u64) -> u64 {
    if whole == 0 {
        0
    } else {
        ((part as f64) * 100. / whole as f64).round() as u64
    }
}

/// Why `played`, one of the repertoire's moves in `check`, is listed; `None` when it is not.
pub fn reason(check: &GameDbRepertoireCheck, played: &GameDbRepertoireMove) -> Option<String> {
    let white = kchess_domain::rules::fen_turn(&check.fen) == Color::White;
    let games = |count: u64| widgets::format_counted(count as f64, "game", "games");
    let main = check.moves.iter().find(|other| other.verdict.is_none());
    let most_played = || {
        main.map(|main| {
            format!(
                " · most played {} ({}%)",
                main.san,
                percent(main.games, check.games)
            )
        })
        .unwrap_or_default()
    };
    let score = |played: &GameDbRepertoireMove| {
        counts(played)
            .score(white)
            .map(|score| (score * 100.).round() as u64)
            .unwrap_or_default()
    };
    match played.verdict? {
        GameDbVerdict::Played => None,
        GameDbVerdict::Unplayed => Some(format!(
            "No game played it of {}{}",
            games(check.games),
            most_played()
        )),
        GameDbVerdict::Rare => Some(format!(
            "{}% of {}{}",
            percent(played.games, check.games),
            games(check.games),
            most_played()
        )),
        GameDbVerdict::Weaker => {
            let main = main?;
            Some(format!(
                "Scores {}% in {} · {} scores {}% in {}",
                score(played),
                games(played.games),
                main.san,
                score(main),
                games(main.games)
            ))
        }
    }
}

/// Checks `steps` against the game databases in a dialog.
pub fn open(steps: Vec<Step>, target: Target, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| RepertoireCheck::new(steps, target, cx));
    widgets::open_dialog(window, cx, move |dialog, _, _| {
        dialog
            .title(SharedString::new_static("Check against databases"))
            .w(px(600.))
            .child(view.clone())
    });
}

pub struct RepertoireCheck {
    steps: Vec<Step>,
    target: Target,
    checks: Option<Vec<GameDbRepertoireCheck>>,
    loading: Loading,
}

/// A move to look at: its step, the move, why, and the move most played instead (UCI and SAN).
struct Listed {
    step: usize,
    uci: String,
    san: String,
    reason: String,
    instead: Option<(String, String)>,
}

impl RepertoireCheck {
    fn new(steps: Vec<Step>, target: Target, cx: &mut Context<Self>) -> Self {
        let sent = steps
            .iter()
            .map(|step| RepertoireStep {
                fen: step.fen.clone(),
                moves: step.moves.iter().map(|(uci, ..)| uci.clone()).collect(),
            })
            .collect();
        let mut this = RepertoireCheck {
            steps,
            target,
            checks: None,
            loading: Loading::default(),
        };
        let request = cx.global::<AppCore>().request(calls::GameDbRepertoire {
            steps: RepertoireSteps(sent),
        });
        this.loading.load(
            cx,
            "database",
            |this| &mut this.loading,
            request,
            |this, answer, _| this.checks = answer.ok(),
        );
        this
    }

    fn listed(&self, checks: &[GameDbRepertoireCheck]) -> Vec<Listed> {
        checks
            .iter()
            .enumerate()
            .flat_map(|(step, check)| {
                let instead = check
                    .moves
                    .iter()
                    .find(|other| other.verdict.is_none())
                    .map(|other| (other.uci.clone(), other.san.clone()));
                check.moves.iter().filter_map(move |played| {
                    Some(Listed {
                        step,
                        uci: played.uci.clone(),
                        san: played.san.clone(),
                        reason: reason(check, played)?,
                        instead: instead.clone(),
                    })
                })
            })
            .collect()
    }

    fn row(
        &self,
        index: usize,
        listed: &Listed,
        cx: &Context<Self>,
        palette: &Palette,
    ) -> impl IntoElement + use<> {
        let step = self.steps[listed.step].clone();
        let uci = listed.uci.clone();
        let row = line_row(
            ("repertoire-check-move", index),
            &step,
            &listed.san,
            &listed.reason,
            palette,
        )
        .on_click({
            let step = step.clone();
            cx.listener(move |_, _, window, cx| open_line(&step, &uci, window, cx))
        })
        .test_support();
        // The move most played instead can join the repertoire.
        let add = listed.instead.clone().map(|(uci, san)| {
            let target = self.target.clone();
            add_button(("repertoire-check-add", index), &san)
                .on_click(move |_, window, cx| add_to_repertoire(&target, &step, &uci, window, cx))
        });
        h_flex()
            .gap(px(4.))
            .child(row.flex_1().min_w_0())
            .children(add)
    }
}

/// Where the moves a check finds are added: the repertoire's study and the chapters drilled.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub study: String,
    pub chapters: Vec<StudyChapter>,
}

impl Target {
    /// Of the chapters that start at `start`, the one that follows `moves` the furthest (the
    /// first of those that follow it as far).
    pub fn chapter_for(&self, start: &str, moves: &[String]) -> Option<String> {
        let mut best: Option<(usize, &str)> = None;
        for chapter in &self.chapters {
            let Some(tree) = TreeNode::from_pgn(&chapter.pgn) else {
                continue;
            };
            if tree.fen != start {
                continue;
            }
            let (mut node, mut depth) = (&tree, 0);
            for uci in moves {
                match node.children.iter().find(|child| child.uci == *uci) {
                    Some(child) => (node, depth) = (child, depth + 1),
                    None => break,
                }
            }
            if best.is_none_or(|(furthest, _)| depth > furthest) {
                best = Some((depth, &chapter.id));
            }
        }
        best.map(|(_, id)| id.to_string())
    }
}

/// `uci` after `step`'s line added to `target`'s study, on the analysis board in the chapter that
/// has the most of that line, where the next move can be added to it too.
pub fn add_to_repertoire(
    target: &Target,
    step: &Step,
    uci: &str,
    window: &mut Window,
    cx: &mut App,
) {
    let mut moves: Vec<String> = step.line.iter().map(|(.., uci)| uci.clone()).collect();
    moves.push(uci.to_string());
    let Some(chapter) = target.chapter_for(&step.start, &moves) else {
        return;
    };
    window.close_dialog(cx);
    analysis::open(
        Open::StudyLine {
            study: target.study.clone(),
            chapter,
            moves,
        },
        window,
        cx,
    );
}

/// The button beside a listed move that adds `san` to the repertoire.
pub fn add_button(id: (&'static str, usize), san: &str) -> Button {
    let label = format!("Add {san} to the repertoire");
    Button::new(id)
        .ghost()
        .small()
        .icon(icon("book-plus"))
        .tooltip(label.clone())
        .accessibility_label(label)
}

/// `step`'s line and then `uci` on the analysis board: from the start, or from the position after
/// it when the tree starts elsewhere.
pub fn open_line(step: &Step, uci: &str, window: &mut Window, cx: &mut App) {
    let open = if step.start == STANDARD_FEN {
        let mut moves: Vec<String> = step.line.iter().map(|(.., uci)| uci.clone()).collect();
        moves.push(uci.to_string());
        Open::Line(moves)
    } else {
        Open::Position(after(&step.fen, uci).or_else(|| Some(step.fen.clone())))
    };
    window.close_dialog(cx);
    analysis::open(open, window, cx);
}

/// The position after `uci` from `fen`.
fn after(fen: &str, uci: &str) -> Option<String> {
    use kchess_domain::rules;
    let mut pos = rules::standard_from_fen(fen)?;
    let m = rules::legal_move(&pos, rules::parse_uci(uci)?)?;
    shakmaty::Position::play_unchecked(&mut pos, m);
    Some(rules::make_fen(&pos))
}

/// A move of `step` to look at, `san`: its line with the move standing out, and why, `reason`.
pub fn line_row(
    id: (&'static str, usize),
    step: &Step,
    san: &str,
    reason: &str,
    palette: &Palette,
) -> Stateful<Div> {
    let before: Vec<(u32, &str)> = step
        .line
        .iter()
        .map(|(ply, san, _)| (*ply, san.as_str()))
        .collect();
    let whole: Vec<(u32, &str)> = before.iter().copied().chain([(step.ply, san)]).collect();
    let line = numbered_line(whole.iter().copied());
    // The move stands out from the moves that lead to it.
    let lead = numbered_line(before.iter().copied());
    let played = line
        .strip_prefix(lead.as_str())
        .unwrap_or(line.as_str())
        .trim_start()
        .to_string();
    v_flex()
        .id(id)
        .role(Role::Button)
        .keyboard_focus(palette.primary)
        .aria_label(format!("{line}: {reason}"))
        .gap(px(2.))
        .px(px(8.))
        .py(px(6.))
        .rounded(radii::BOX)
        .cursor_pointer()
        .pressable(palette)
        .child(
            h_flex()
                .gap(px(4.))
                .text_size(px(13.))
                .when(!lead.is_empty(), |row| row.child(lead))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(played)),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(reason.to_string()),
        )
}

impl Render for RepertoireCheck {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let Some(checks) = &self.checks else {
            let body = match self.loading.error() {
                Some(error) => div()
                    .text_color(palette.error)
                    .child(error.clone())
                    .into_any_element(),
                None => widgets::loading("Checking your moves…", &palette).into_any_element(),
            };
            return v_flex().gap(px(12.)).child(body);
        };
        let moves: usize = self.steps.iter().map(|step| step.moves.len()).sum();
        let unjudged: usize = checks
            .iter()
            .zip(&self.steps)
            .filter(|(check, _)| check.games < ENOUGH)
            .map(|(_, step)| step.moves.len())
            .sum();
        let listed = self.listed(checks);
        let summary = [
            Some(widgets::format_counted(
                moves as f64,
                "move checked",
                "moves checked",
            )),
            Some(widgets::format_counted(
                listed.len() as f64,
                "to look at",
                "to look at",
            )),
            (unjudged > 0).then(|| {
                format!(
                    "{} with too few games to judge",
                    widgets::format_counted(unjudged as f64, "move", "moves")
                )
            }),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<String>>()
        .join(" · ");
        let rows: Vec<AnyElement> = listed
            .iter()
            .enumerate()
            .map(|(index, listed)| self.row(index, listed, cx, &palette).into_any_element())
            .collect();
        v_flex()
            .gap(px(12.))
            .child(
                h_flex().child(
                    div()
                        .id("repertoire-check-summary")
                        .test_support()
                        .aria_label(summary.clone())
                        .text_size(px(13.))
                        .text_color(palette.text_muted)
                        .child(summary),
                ),
            )
            .child(
                v_flex()
                    .id("repertoire-check-moves")
                    .max_h(px(440.))
                    .overflow_y_scroll()
                    .gap(px(2.))
                    .children(rows),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{Target, TreeNode, every_step, numbered_line, steps};
    use crate::repertoire::study_roots;
    use kchess_contracts::types::StudyChapter;
    use shakmaty::Color;

    fn root(pgn: &str) -> TreeNode {
        let chapter = StudyChapter {
            id: pgn.into(),
            name: pgn.into(),
            pgn: pgn.into(),
        };
        study_roots(&[chapter]).remove(0)
    }

    #[test]
    fn a_line_reads_numbered() {
        assert_eq!(
            numbered_line([(0, "e4"), (1, "c5"), (2, "Nf3")]),
            "1. e4 c5 2. Nf3"
        );
        assert_eq!(numbered_line([(1, "c5"), (2, "Nf3")]), "1… c5 2. Nf3");
    }

    #[test]
    fn steps_are_the_positions_the_repertoire_answers_once_each() {
        // As White: 1. e4 or 1. d4; after 1...e5 both 2. Nf3 and 2. Nc3; after 1...c5 2. Nf3;
        // after 1...d5 2. c4.
        let root = root("1. e4 (1. d4 d5 2. c4) e5 (1... c5 2. Nf3) 2. Nf3 (2. Nc3) *");
        let steps = steps(std::slice::from_ref(&root), Color::White);
        let read: Vec<(String, Vec<String>)> = steps
            .iter()
            .map(|step| {
                (
                    numbered_line(step.line.iter().map(|(ply, san, _)| (*ply, san.as_str()))),
                    step.moves.iter().map(|(_, san, _)| san.clone()).collect(),
                )
            })
            .collect();
        assert_eq!(
            read,
            [
                (String::new(), vec!["e4".to_string(), "d4".to_string()]),
                (
                    "1. e4 e5".to_string(),
                    vec!["Nf3".to_string(), "Nc3".to_string()]
                ),
                ("1. e4 c5".to_string(), vec!["Nf3".to_string()]),
                ("1. d4 d5".to_string(), vec!["c4".to_string()]),
            ]
        );
        // As Black: the positions after White's moves.
        let black = steps_for_black(&root);
        assert_eq!(black, ["1. e4", "1. d4"]);
    }

    #[test]
    fn every_step_is_each_position_the_repertoire_goes_on_from_either_side() {
        let root = root("1. e4 e5 (1... c5 2. Nf3) 2. Nf3 *");
        let read: Vec<(String, Vec<String>)> = every_step(std::slice::from_ref(&root))
            .iter()
            .map(|step| {
                (
                    numbered_line(step.line.iter().map(|(ply, san, _)| (*ply, san.as_str()))),
                    step.moves.iter().map(|(_, san, _)| san.clone()).collect(),
                )
            })
            .collect();
        assert_eq!(
            read,
            [
                (String::new(), vec!["e4".to_string()]),
                (
                    "1. e4".to_string(),
                    vec!["e5".to_string(), "c5".to_string()]
                ),
                ("1. e4 e5".to_string(), vec!["Nf3".to_string()]),
                ("1. e4 c5".to_string(), vec!["Nf3".to_string()]),
            ]
        );
    }

    #[test]
    fn a_move_joins_the_chapter_that_has_the_most_of_its_line() {
        let chapter = |id: &str, pgn: &str| StudyChapter {
            id: id.into(),
            name: id.into(),
            pgn: pgn.into(),
        };
        let target = Target {
            study: "study".into(),
            chapters: vec![
                chapter("open", "1. e4 e5 2. Nf3 *"),
                chapter("sicilian", "1. e4 c5 2. Nf3 d6 *"),
                chapter(
                    "french",
                    "[FEN \"rnbqkbnr/pppp1ppp/4p3/8/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2\"]\n\n2. d4 *",
                ),
            ],
        };
        let start = crate::game::STANDARD_FEN.to_string();
        let line = |moves: &str| -> Vec<String> { moves.split(' ').map(str::to_string).collect() };
        assert_eq!(
            target
                .chapter_for(&start, &line("e2e4 c7c5 b1c3"))
                .as_deref(),
            Some("sicilian")
        );
        // Both have 1. e4: the first.
        assert_eq!(
            target.chapter_for(&start, &line("e2e4 c7c6")).as_deref(),
            Some("open")
        );
        assert_eq!(
            target
                .chapter_for(
                    "rnbqkbnr/pppp1ppp/4p3/8/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2",
                    &line("d2d4 d7d5")
                )
                .as_deref(),
            Some("french")
        );
        assert_eq!(target.chapter_for("8/8/8/8/8/8/8/8 w - - 0 1", &[]), None);
    }

    fn steps_for_black(root: &TreeNode) -> Vec<String> {
        steps(std::slice::from_ref(root), Color::Black)
            .iter()
            .map(|step| numbered_line(step.line.iter().map(|(ply, san, _)| (*ply, san.as_str()))))
            .collect()
    }
}
