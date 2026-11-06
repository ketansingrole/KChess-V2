//! Practice: the board-vision drills and the trainers, one tab at a time,
//! each made the first time it is shown and kept, as the Electron page keeps its stores.

mod coordinates;
pub(crate) mod endgame_drills;
pub(crate) mod endgame_lesson;
mod endgames;
mod engine_move;
mod knight;
pub(crate) mod mistakes;
pub(crate) mod repertoire;
mod run;
mod square_color;

use gpui_kit::component::v_flex;
use gpui_kit::*;

use crate::assets::icon;
use crate::play_layout::{PAGE_TOP, side_padding};
use crate::theme::Palette;
use crate::widgets::{self, Pill};

const TAB: &str = "kchess:practice-tab";

/// The tabs, in the Electron app's order: their stored names and labels.
const TABS: [(&str, &str); 6] = [
    ("coordinates", "Coordinates"),
    ("colors", "Square colours"),
    ("knight", "Knight paths"),
    ("endgames", "Endgames"),
    ("openings", "Repertoire"),
    ("mistakes", "Your mistakes"),
];

/// The actions of the trainer on screen, as its buttons offer them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Action {
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
    /// The tab on screen at the last frame: Your mistakes starts at its overview when opened.
    shown_tab: Option<&'static str>,
    focus: FocusHandle,
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
            shown_tab: None,
            focus: cx.focus_handle(),
        }
    }

    /// The endgame drills, while they are on screen.
    pub(crate) fn drills(&self, cx: &App) -> Option<Entity<endgame_drills::EndgameDrills>> {
        if Self::tab(cx) != "endgames" {
            return None;
        }
        self.endgames.as_ref()?.read(cx).shown_drills(cx)
    }

    #[cfg(test)]
    pub(crate) fn mistakes(&self) -> Option<Entity<mistakes::Mistakes>> {
        self.mistakes.clone()
    }

    /// The endgame lesson, while it is on screen.
    fn lesson(&self, cx: &App) -> Option<Entity<endgame_lesson::EndgameLesson>> {
        if Self::tab(cx) != "endgames" {
            return None;
        }
        self.endgames.as_ref()?.read(cx).shown_lesson(cx)
    }

    /// What the trainer on screen can do now.
    pub(crate) fn actions(&self, cx: &App) -> Vec<Action> {
        match Self::tab(cx) {
            "endgames" => {
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
            "openings" => self
                .repertoire
                .as_ref()
                .map(|repertoire| repertoire.read(cx).actions(cx))
                .unwrap_or_default()
                .into_iter()
                .map(Action::Repertoire)
                .collect(),
            "mistakes" => self
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
                if Self::tab(cx) != "mistakes" {
                    return;
                }
                if let Some(mistakes) = self.mistakes.clone() {
                    mistakes.update(cx, |mistakes, cx| mistakes.act(action, cx));
                }
            }
            Action::Repertoire(action) => {
                if Self::tab(cx) != "openings" {
                    return;
                }
                if let Some(repertoire) = self.repertoire.clone() {
                    repertoire.update(cx, |repertoire, cx| repertoire.act(action, window, cx));
                }
            }
        }
    }

    /// The tab shown; a tab saved by an earlier release falls back to the first trainer.
    fn tab(cx: &App) -> &'static str {
        let stored = crate::prefs::get::<String>(cx, TAB, "coordinates".into());
        TABS.iter()
            .find(|(key, _)| *key == stored)
            .map_or("coordinates", |(key, _)| key)
    }

    /// The page was opened from the sidebar: its tab starts afresh where Electron remounts it.
    pub fn shown(&mut self) {
        self.shown_tab = None;
    }

    /// The tab's drill, made the first time it is shown.
    fn body(&mut self, tab: &str, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let view: AnyView = match tab {
            "coordinates" => self
                .coordinates
                .get_or_insert_with(|| cx.new(coordinates::Coordinates::new))
                .clone()
                .into(),
            "colors" => self
                .colors
                .get_or_insert_with(|| cx.new(square_color::SquareColors::new))
                .clone()
                .into(),
            "knight" => self
                .knight
                .get_or_insert_with(|| cx.new(|cx| knight::KnightPaths::new(window, cx)))
                .clone()
                .into(),
            "endgames" => self
                .endgames
                .get_or_insert_with(|| cx.new(endgames::Endgames::new))
                .clone()
                .into(),
            "openings" => self
                .repertoire
                .get_or_insert_with(|| cx.new(|cx| repertoire::Repertoire::new(window, cx)))
                .clone()
                .into(),
            _ => {
                let opened = self.shown_tab != Some("mistakes");
                let mistakes = match &self.mistakes {
                    Some(mistakes) => {
                        if opened {
                            mistakes.update(cx, |mistakes, cx| mistakes.shown(cx));
                        }
                        mistakes.clone()
                    }
                    None => {
                        let mistakes = cx.new(|cx| mistakes::Mistakes::new(window, cx));
                        self.mistakes = Some(mistakes.clone());
                        mistakes
                    }
                };
                mistakes.into()
            }
        };
        view.into_any_element()
    }

    /// The page keeps the keys and hands them to the drill on screen (`keydown` in each drill).
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match Self::tab(cx) {
            "coordinates" => {
                if let Some(drill) = &self.coordinates {
                    drill.update(cx, |drill, cx| drill.key(event, window, cx));
                }
            }
            "colors" => {
                if let Some(drill) = &self.colors {
                    drill.update(cx, |drill, cx| drill.key(event, window, cx));
                }
            }
            // Return or Space goes on to a lesson's next step once this one is read or passed.
            "endgames"
                if self.focus.is_focused(window)
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
            // F turns an endgame drill's or lesson's board, as its button does.
            "endgames"
                if self.focus.is_focused(window)
                    && !event.keystroke.modifiers.modified()
                    && event.keystroke.key == "f" =>
            {
                let flip = self.actions(cx).into_iter().find(|action| {
                    matches!(
                        action,
                        Action::Drill(endgame_drills::DrillAction::Flip)
                            | Action::Lesson(endgame_lesson::LessonAction::Flip)
                    )
                });
                if let Some(flip) = flip {
                    self.act(flip, window, cx);
                    cx.stop_propagation();
                }
            }
            // A mistake's board: F turns it, and a finished one's moves step back and forth.
            "mistakes"
                if self.focus.is_focused(window) && !event.keystroke.modifiers.modified() =>
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
        let tab = Self::tab(cx);
        let tabs = widgets::pills(
            "practice-tabs",
            TABS.iter()
                .map(|(key, label)| Pill {
                    icon: Some(match *key {
                        "coordinates" => icon("grid-3x3"),
                        "colors" => icon("contrast"),
                        "knight" => icon("crown"),
                        "endgames" => icon("swords"),
                        "openings" => icon("book-marked"),
                        _ => icon("book-open"),
                    }),
                    label: (*label).into(),
                    badge: None,
                    disabled: false,
                })
                .collect(),
            TABS.iter().position(|(key, _)| *key == tab).unwrap_or(0),
            |index, window, cx| {
                crate::prefs::set(cx, TAB, TABS[index].0);
                window.refresh();
            },
            false,
            &palette,
        );
        let body = self.body(tab, window, cx);
        self.shown_tab = Some(tab);
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            // The drills that answer keys keep them; a trainer's move box keeps its own focus.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if matches!(Self::tab(cx), "coordinates" | "colors") {
                        window.focus(&this.focus, cx)
                    }
                }),
            )
            // On an endgame's or a mistake's board a click gives the keys back to the page,
            // before a text field takes them.
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, window, cx| {
                if event.button == MouseButton::Left
                    && matches!(Self::tab(cx), "endgames" | "mistakes")
                {
                    window.focus(&this.focus, cx);
                }
            }))
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
