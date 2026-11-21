//! The Practice page: the board-vision drills and the trainers, one tab at a time,
//! each made the first time it is shown and kept.

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests (`check`).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

// The app's services, the design system and the board, under the names this page's modules use.
// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

use kchess_app::{core, loading, log, network, prefs, reviews, settings, studies, timer};
use kchess_board::{
    board, game, live_game, notation, player_search, position_engine, puzzle_board, sound, voice,
    widgets,
};
use kchess_ui::theme;

mod classics;
pub mod coordinates;
pub mod endgame_drills;
pub mod endgame_lesson;
mod endgames;
mod engine_move;
pub mod guess;
mod guess_find;
mod knight;
pub mod mistakes;
pub mod repertoire;
pub mod run;
mod square_color;

use gpui_kit::component::v_flex;
use gpui_kit::*;

use crate::game::surface::TakesKeys as _;
use crate::theme::Palette;
use gpui_kit::prelude::FluentBuilder as _;
use kchess_nav::page::{
    ActionInfo, NavGroup, Page, PageAction, PageCommand, PageInfo, PageTab, PageView, TabInfo,
};
use kchess_nav::play_layout::{PAGE_TOP, side_padding};

/// Practice's tabs, in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Coordinates,
    Colors,
    Knight,
    Endgames,
    Openings,
    Mistakes,
    Guess,
}

impl PageTab for Tab {
    const ALL: &'static [Tab] = &[
        Tab::Coordinates,
        Tab::Colors,
        Tab::Knight,
        Tab::Endgames,
        Tab::Openings,
        Tab::Mistakes,
        Tab::Guess,
    ];
    const KEY: &'static str = "kchess:practice-tab";

    fn info(self) -> TabInfo {
        let (key, label, title, icon, keywords) = match self {
            Tab::Coordinates => (
                "coordinates",
                "Coordinates",
                "Board coordinates",
                "grid-3x3",
                "squares names",
            ),
            Tab::Colors => (
                "colors",
                "Square colours",
                "Square colours",
                "contrast",
                "light dark",
            ),
            Tab::Knight => (
                "knight",
                "Knight paths",
                "Knight paths",
                "crown",
                "knight tour",
            ),
            Tab::Endgames => (
                "endgames",
                "Endgames",
                "Endgame drills",
                "swords",
                "mate technique",
            ),
            Tab::Openings => (
                "openings",
                "Repertoire",
                "Repertoire trainer",
                "book-marked",
                "opening drill study",
            ),
            Tab::Mistakes => (
                "mistakes",
                "Your mistakes",
                "Your mistakes",
                "book-open",
                "blunders review",
            ),
            Tab::Guess => (
                "guess",
                "Guess the move",
                "Guess the move",
                "target",
                "master games predict",
            ),
        };
        TabInfo {
            key,
            label,
            title,
            icon,
            keywords,
        }
    }
}

/// The actions of the trainer on screen, as its buttons offer them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Drill(endgame_drills::DrillAction),
    Lesson(endgame_lesson::LessonAction),
    Repertoire(repertoire::RepertoireAction),
    Mistakes(mistakes::MistakeAction),
}

pub struct PracticePage {
    coordinates: Option<Entity<coordinates::Coordinates>>,
    colors: Option<Entity<square_color::SquareColors>>,
    knight: Option<Entity<knight::KnightPaths>>,
    repertoire: Option<Entity<repertoire::Repertoire>>,
    endgames: Option<Entity<endgames::Endgames>>,
    mistakes: Option<Entity<mistakes::Mistakes>>,
    guess: Option<Entity<guess::GuessMoves>>,
    /// The tab last opened (`show_tab`): Your mistakes starts at its overview when opened.
    shown_tab: Option<Tab>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for PracticePage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl PracticePage {
    pub fn new(cx: &mut Context<Self>) -> PracticePage {
        PracticePage {
            coordinates: None,
            colors: None,
            knight: None,
            repertoire: None,
            endgames: None,
            mistakes: None,
            guess: None,
            shown_tab: None,
            focus: cx.focus_handle(),
            // The tab is a view preference: chosen here, from Quick Pick or from another window.
            _subscriptions: vec![
                cx.observe_global::<crate::prefs::ViewPreferences>(|this, cx| this.follow_tab(cx)),
            ],
        }
    }

    /// The tab chosen, once it changes: Your mistakes, opened again, starts at its overview.
    fn follow_tab(&mut self, cx: &mut Context<Self>) {
        let tab = Tab::stored(cx);
        if self.shown_tab == Some(tab) {
            return;
        }
        self.shown_tab = Some(tab);
        if tab == Tab::Mistakes
            && let Some(mistakes) = &self.mistakes
        {
            mistakes.update(cx, |mistakes, cx| mistakes.shown(cx));
        }
    }

    /// The endgame drills, while they are on screen.
    pub(crate) fn drills(&self, cx: &App) -> Option<Entity<endgame_drills::EndgameDrills>> {
        if Tab::stored(cx) != Tab::Endgames {
            return None;
        }
        self.endgames.as_ref()?.read(cx).shown_drills(cx)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn coordinates(&self) -> Option<Entity<coordinates::Coordinates>> {
        self.coordinates.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn colors(&self) -> Option<Entity<square_color::SquareColors>> {
        self.colors.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn mistakes(&self) -> Option<Entity<mistakes::Mistakes>> {
        self.mistakes.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn repertoire(&self) -> Option<Entity<repertoire::Repertoire>> {
        self.repertoire.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn guess(&self) -> Option<Entity<guess::GuessMoves>> {
        self.guess.clone()
    }

    /// Guess through a game's PGN as `side`, on the Guess the move tab.
    pub(crate) fn guess_moves(
        &mut self,
        pgn: &str,
        side: Option<shakmaty::Color>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        Tab::Guess.store(cx);
        let guess = self
            .guess
            .get_or_insert_with(|| cx.new(|cx| guess::GuessMoves::new(window, cx)))
            .clone();
        guess.update(cx, |guess, cx| guess.load(pgn, side, cx));
        cx.notify();
    }

    /// The endgame lesson, while it is on screen.
    fn lesson(&self, cx: &App) -> Option<Entity<endgame_lesson::EndgameLesson>> {
        if Tab::stored(cx) != Tab::Endgames {
            return None;
        }
        self.endgames.as_ref()?.read(cx).shown_lesson(cx)
    }

    /// Run an action on the trainer on screen.
    pub(crate) fn act(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            Action::Drill(action) => {
                if let Some(drills) = self.drills(cx) {
                    drills.update(cx, |drills, cx| drills.act(action, cx));
                }
            }
            Action::Lesson(action) => {
                if let Some(lesson) = self.lesson(cx) {
                    lesson.update(cx, |lesson, cx| lesson.act(action, cx));
                }
            }
            Action::Mistakes(action) => {
                if Tab::stored(cx) != Tab::Mistakes {
                    return;
                }
                if let Some(mistakes) = self.mistakes.clone() {
                    mistakes.update(cx, |mistakes, cx| mistakes.act(action, cx));
                }
            }
            Action::Repertoire(action) => {
                if Tab::stored(cx) != Tab::Openings {
                    return;
                }
                if let Some(repertoire) = self.repertoire.clone() {
                    repertoire.update(cx, |repertoire, cx| repertoire.act(action, window, cx));
                }
            }
        }
    }

    /// Show `tab`, as its pill does.
    pub fn show_tab(&mut self, tab: Tab, _: &mut Window, cx: &mut Context<Self>) {
        tab.store(cx);
        cx.notify();
    }

    /// The tab's drill, made the first time it is shown.
    fn body(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let view: AnyView = match tab {
            Tab::Coordinates => self
                .coordinates
                .get_or_insert_with(|| cx.new(coordinates::Coordinates::new))
                .clone()
                .into(),
            Tab::Colors => self
                .colors
                .get_or_insert_with(|| cx.new(square_color::SquareColors::new))
                .clone()
                .into(),
            Tab::Knight => self
                .knight
                .get_or_insert_with(|| cx.new(|cx| knight::KnightPaths::new(window, cx)))
                .clone()
                .into(),
            Tab::Endgames => self
                .endgames
                .get_or_insert_with(|| cx.new(endgames::Endgames::new))
                .clone()
                .into(),
            Tab::Openings => self
                .repertoire
                .get_or_insert_with(|| cx.new(|cx| repertoire::Repertoire::new(window, cx)))
                .clone()
                .into(),
            Tab::Guess => self
                .guess
                .get_or_insert_with(|| cx.new(|cx| guess::GuessMoves::new(window, cx)))
                .clone()
                .into(),
            Tab::Mistakes => self
                .mistakes
                .get_or_insert_with(|| cx.new(|cx| mistakes::Mistakes::new(window, cx)))
                .clone()
                .into(),
        };
        view.into_any_element()
    }

    /// The page keeps the keys and hands them to the drill on screen.
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match Tab::stored(cx) {
            Tab::Coordinates => {
                if let Some(drill) = &self.coordinates {
                    drill.update(cx, |drill, cx| drill.key(event, window, cx));
                }
            }
            Tab::Colors => {
                if let Some(drill) = &self.colors {
                    drill.update(cx, |drill, cx| drill.key(event, window, cx));
                }
            }
            // Return or Space goes on to a lesson's next step once this one is read or passed.
            Tab::Endgames
                if crate::board::page_has_keys(&self.focus, window, cx)
                    && !event.keystroke.modifiers.modified()
                    && matches!(event.keystroke.key.as_str(), "enter" | "space") =>
            {
                let next = self
                    .lesson(cx)
                    .and_then(|lesson| lesson.read(cx).next_on_return());
                if let Some(next) = next {
                    self.act(Action::Lesson(next), window, cx);
                    cx.stop_propagation();
                }
            }
            // A mistake's board: F turns it, and a finished one's moves step back and forth.
            Tab::Mistakes
                if crate::board::page_has_keys(&self.focus, window, cx)
                    && !event.keystroke.modifiers.modified() =>
            {
                if let Some(mistakes) = &self.mistakes
                    && mistakes.update(cx, |mistakes, cx| mistakes.key(&event.keystroke.key, cx))
                {
                    cx.stop_propagation();
                }
            }
            _ => {}
        }
    }
}

impl Render for PracticePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let tab = Tab::stored(cx);
        let tabs = widgets::pills(
            "practice-tabs",
            Tab::pills(),
            tab.index(),
            |index, window, cx| {
                Tab::ALL[index].store(cx);
                window.refresh();
            },
            false,
            &palette,
        );
        let body = self.body(tab, window, cx);
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            // The drills that answer keys keep them; a trainer's move box keeps its own focus.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if matches!(Tab::stored(cx), Tab::Coordinates | Tab::Colors) {
                        window.focus(&this.focus, cx)
                    }
                }),
            )
            // On an endgame's or a mistake's board a click gives the keys back to the page.
            .when(
                matches!(Tab::stored(cx), Tab::Endgames | Tab::Mistakes),
                |page| page.takes_keys(&self.focus),
            )
            .child(
                div()
                    .flex_none()
                    .px(side_padding(window))
                    .pt(PAGE_TOP)
                    .child(tabs),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}

impl PageAction for Action {
    fn info(&self) -> ActionInfo {
        use endgame_drills::DrillAction;
        use endgame_lesson::LessonAction;
        use mistakes::MistakeAction;
        use repertoire::RepertoireAction;
        let (id, label, glyph, keywords) = match *self {
            Action::Drill(DrillAction::Hint) => ("hint", "Hint", "lightbulb", "help best move"),
            Action::Drill(DrillAction::TakeBack) => ("takeback", "Take back", "undo-2", "undo"),
            Action::Drill(DrillAction::Flip) => return ActionInfo::flip("practice:flip"),
            Action::Drill(DrillAction::Restart) => ("restart", "Restart", "rotate-ccw", "again"),
            Action::Lesson(LessonAction::Back) => {
                ("courses", "All courses", "chevron-left", "back lessons")
            }
            Action::Lesson(LessonAction::Hint) => ("lesson-hint", "Hint", "lightbulb", "help"),
            Action::Lesson(LessonAction::TakeBack) => {
                ("lesson-takeback", "Take back", "undo-2", "undo")
            }
            Action::Lesson(LessonAction::Flip) => return ActionInfo::flip("practice:lesson-flip"),
            Action::Lesson(LessonAction::Retry) => ("retry", "Retry", "rotate-ccw", "again"),
            Action::Lesson(LessonAction::Previous) => {
                ("previous", "Previous step", "chevron-left", "back")
            }
            Action::Lesson(LessonAction::Next) => ("next-step", "Next", "chevron-right", "step"),
            Action::Lesson(LessonAction::NextChapter) => {
                ("next-chapter", "Next chapter", "chevron-right", "lesson")
            }
            Action::Repertoire(RepertoireAction::ShowMove) => {
                ("show-move", "Show move", "lightbulb", "hint answer")
            }
            Action::Repertoire(RepertoireAction::Restart) => (
                "restart-line",
                "Restart line",
                "rotate-ccw",
                "new line again",
            ),
            Action::Repertoire(RepertoireAction::Review) => (
                "review-due",
                "Review due lines",
                "calendar-days",
                "spaced repetition schedule",
            ),
            Action::Repertoire(RepertoireAction::Forget) => {
                ("forget", "Forget mistakes", "eraser", "clear misses")
            }
            Action::Repertoire(RepertoireAction::Save) => {
                ("save", "Save as study", "save", "keep opening")
            }
            Action::Mistakes(MistakeAction::Flip) => {
                return ActionInfo::flip("practice:mistake-flip");
            }
            Action::Mistakes(MistakeAction::Hint) => {
                ("mistake-hint", "Hint: piece to move", "lightbulb", "help")
            }
            Action::Mistakes(MistakeAction::Solution) => {
                ("solution", "Show solution", "eye", "answer")
            }
            Action::Mistakes(MistakeAction::Next) => {
                ("next", "Next due position", "arrow-right", "mistake")
            }
            Action::Mistakes(MistakeAction::Due) => {
                ("due", "Practice due positions", "target", "mistakes start")
            }
            Action::Mistakes(MistakeAction::Early) => {
                ("early", "Review early", "fast-forward", "mistakes ahead")
            }
        };
        ActionInfo::new(format!("practice:{id}"), label, glyph, keywords)
    }
}

impl PageView for PracticePage {
    const INFO: PageInfo = PageInfo {
        page: Page::Practice,
        label: "Practice",
        short: "Practice",
        icon: "graduation-cap",
        group: Some(NavGroup::Train),
        keywords: "",
    };

    type Action = Action;

    fn create(_: &mut Window, cx: &mut Context<Self>) -> Self {
        PracticePage::new(cx)
    }

    /// Opened again from the sidebar, the tab starts afresh.
    fn shown(this: &Entity<Self>, _again: bool, _window: &mut Window, cx: &mut App) {
        this.update(cx, |practice, cx| {
            practice.shown_tab = None;
            practice.follow_tab(cx);
            cx.notify();
        });
    }

    /// The trainers with a board of their own: the coordinates, knight paths, endgame and
    /// repertoire trainers.
    fn zen(&self, cx: &App) -> bool {
        matches!(
            Tab::stored(cx),
            Tab::Coordinates | Tab::Knight | Tab::Endgames | Tab::Openings
        )
    }

    fn actions(&self, cx: &App) -> Vec<Action> {
        match Tab::stored(cx) {
            Tab::Endgames => {
                if let Some(lesson) = self.lesson(cx) {
                    return lesson
                        .read(cx)
                        .actions()
                        .into_iter()
                        .map(Action::Lesson)
                        .collect();
                }
                self.drills(cx)
                    .map(|drills| drills.read(cx).actions())
                    .unwrap_or_default()
                    .into_iter()
                    .map(Action::Drill)
                    .collect()
            }
            Tab::Openings => self
                .repertoire
                .as_ref()
                .map(|repertoire| repertoire.read(cx).actions(cx))
                .unwrap_or_default()
                .into_iter()
                .map(Action::Repertoire)
                .collect(),
            Tab::Mistakes => self
                .mistakes
                .as_ref()
                .map(|mistakes| mistakes.read(cx).actions())
                .unwrap_or_default()
                .into_iter()
                .map(Action::Mistakes)
                .collect(),
            _ => Vec::new(),
        }
    }

    fn act(this: &Entity<Self>, action: Action, window: &mut Window, cx: &mut App) {
        this.update(cx, |practice, cx| practice.act(action, window, cx));
    }

    fn places(_: &App) -> Vec<PageCommand> {
        kchess_nav::page::tab_places::<Self, Tab>("practice", PracticePage::show_tab)
    }
}

/// Practice at `tab`.
pub fn open_tab(tab: Tab, window: &mut Window, cx: &mut App) {
    kchess_nav::shell::show::<PracticePage>(window, cx, move |practice, window, cx| {
        practice.show_tab(tab, window, cx)
    });
}

/// Guess the move through a game's PGN as `side`, on Practice.
pub fn guess_moves(pgn: &str, side: Option<shakmaty::Color>, window: &mut Window, cx: &mut App) {
    let pgn = pgn.to_string();
    kchess_nav::shell::show::<PracticePage>(window, cx, move |practice, window, cx| {
        practice.guess_moves(&pgn, side, window, cx)
    });
}

#[cfg(test)]
mod tests {
    use super::{PracticePage, Tab};
    use kchess_nav::page::{PageTab as _, PageView as _};

    /// Other pages open Practice at a tab, or at Guess the move with a game; each asks the shell
    /// to open Practice. The command palette offers each tab.
    #[gpui_kit::test]
    fn other_pages_open_practice_at_what_they_asked(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let mut app = kchess_board::testing::Started::new(cx);
        let (page, shell) = app.with_window(|_, cx| {
            let page = cx.new(PracticePage::new);
            let shell = std::rc::Rc::new(kchess_nav::testing::OnlyPage::of(page.clone()));
            cx.set_global(kchess_nav::shell::Navigator(shell.clone()));
            (page, shell)
        });
        let tab = app.with_window(|window, cx| {
            super::open_tab(Tab::Knight, window, cx);
            Tab::stored(cx)
        });
        assert_eq!(tab, Tab::Knight);
        let (tab, guessing) = app.with_window(|window, cx| {
            super::guess_moves("1. e4 e5 *", None, window, cx);
            (Tab::stored(cx), page.read(cx).guess().is_some())
        });
        assert_eq!((tab, guessing), (Tab::Guess, true));
        assert_eq!(*shell.opened.borrow(), ["Practice", "Practice"]);
        let places = app.with_window(|_, cx| PracticePage::places(cx).len());
        assert_eq!(places, Tab::ALL.len());
    }
}
