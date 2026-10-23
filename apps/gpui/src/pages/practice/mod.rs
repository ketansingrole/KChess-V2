//! Practice (`pages/practice.vue`): the board-vision drills and the trainers, one tab at a time,
//! each made the first time it is shown and kept, as the Electron page keeps its stores.

mod coordinates;
mod knight;
mod repertoire;
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

pub struct PracticePage {
    coordinates: Option<Entity<coordinates::Coordinates>>,
    colors: Option<Entity<square_color::SquareColors>>,
    knight: Option<Entity<knight::KnightPaths>>,
    repertoire: Option<Entity<repertoire::Repertoire>>,
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
            focus: cx.focus_handle(),
        }
    }

    /// The tab shown; a tab saved by an earlier release falls back to the first trainer.
    fn tab(cx: &App) -> &'static str {
        let stored = crate::prefs::get::<String>(cx, TAB, "coordinates".into());
        TABS.iter()
            .find(|(key, _)| *key == stored)
            .map_or("coordinates", |(key, _)| key)
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
            "openings" => self
                .repertoire
                .get_or_insert_with(|| cx.new(|cx| repertoire::Repertoire::new(window, cx)))
                .clone()
                .into(),
            _ => {
                let palette = *cx.global::<Palette>();
                return div()
                    .px(side_padding(window))
                    .pt(px(20.))
                    .child(
                        widgets::card(&palette)
                            .items_center()
                            .py(px(40.))
                            .text_color(palette.text_muted)
                            .child("Use the Electron app for this trainer for now."),
                    )
                    .into_any_element();
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
