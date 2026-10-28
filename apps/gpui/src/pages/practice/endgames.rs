//! Practice → Endgames: the checkmate and endgame courses, and the drills
//! against Stockfish. The courses and drills are the shared data the Rust rules check
//! (`data/lessons.json`, `data/training.json`); finished chapters and drills are runs the core keeps.

use std::sync::OnceLock;

use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_core::Level;
use serde::Deserialize;
use serde_json::{Value, json};

use super::endgame_drills::EndgameDrills;
use super::endgame_lesson::EndgameLesson;
use crate::assets::icon;
use crate::core::AppCore;
use crate::play_layout::{Measured, PAGE_BOTTOM, PAGE_TOP, side_padding};
use crate::theme::{Palette, tint};
use crate::widgets;
use crate::widgets::KeyboardFocus as _;

const VIEW: &str = "kchess:endgame-view";
const COURSES: &str = "courses";
const DRILLS: &str = "drills";
const COURSE_GROUPS: [&str; 3] = ["Checkmates", "Pawn endgames", "Rook endgames"];
/// The cards' columns (`.course-grid`).
const CARD_MIN: Pixels = px(300.);
const CARD_GAP: Pixels = px(12.);

/// An endgame to convert or hold against Stockfish, from a position checked to be won or drawn.
#[derive(Clone, Deserialize)]
pub struct Drill {
    pub id: String,
    pub title: String,
    pub group: String,
    pub fen: String,
    /// The side you play.
    pub player: String,
    /// `win` or `draw`.
    pub goal: String,
    pub level: String,
    /// What to aim for, in a sentence.
    pub idea: String,
}

#[derive(Clone, Deserialize)]
pub struct Shape {
    pub orig: String,
    pub dest: Option<String>,
    pub brush: String,
}

#[derive(Clone, Deserialize)]
pub struct LessonStep {
    pub fen: String,
    pub text: String,
    #[serde(default)]
    pub shapes: Vec<Shape>,
    /// What the step asks (`LessonTask`), as the rules read it.
    pub task: Value,
}

#[derive(Clone, Deserialize)]
pub struct LessonChapter {
    /// Unique across all courses: the saved run's variant.
    pub id: String,
    pub title: String,
    pub steps: Vec<LessonStep>,
}

#[derive(Clone, Deserialize)]
pub struct Course {
    pub id: String,
    pub group: String,
    pub title: String,
    pub subtitle: String,
    pub chapters: Vec<LessonChapter>,
}

/// `ENDGAME_DRILLS`.
pub fn drills() -> &'static [Drill] {
    static DRILLS: OnceLock<Vec<Drill>> = OnceLock::new();
    DRILLS.get_or_init(|| {
        #[derive(Deserialize)]
        struct Training {
            #[serde(rename = "endgameDrills")]
            endgame_drills: Vec<Drill>,
        }
        serde_json::from_str::<Training>(include_str!(
            "../../../../../crates/kchess-domain/data/training.json"
        ))
        .map(|training| training.endgame_drills)
        .unwrap_or_default()
    })
}

/// `LESSON_COURSES`.
pub fn courses() -> &'static [Course] {
    static COURSES: OnceLock<Vec<Course>> = OnceLock::new();
    COURSES.get_or_init(|| {
        #[derive(Deserialize)]
        struct Lessons {
            courses: Vec<Course>,
        }
        serde_json::from_str::<Lessons>(include_str!(
            "../../../../../crates/kchess-domain/data/lessons.json"
        ))
        .map(|lessons| lessons.courses)
        .unwrap_or_default()
    })
}

/// A drill or chapter finished at least once.
pub fn finished(summary: &Value, id: &str) -> bool {
    summary["best"][id]["score"].as_f64().unwrap_or(0.) > 0.
}

/// What a course or the drills tell the hub.
pub enum TrainerEvent {
    /// Back to the courses.
    Back,
    /// A chapter was finished: the lessons' summary as the core now keeps it.
    Saved(Value),
}

/// Each course's card icon.
fn course_icon(id: &str) -> Icon {
    match id {
        "piece-checkmates-1" => icon("chess-queen"),
        "checkmate-patterns-1" => icon("target"),
        "checkmate-patterns-2" => icon("crosshair"),
        "checkmate-patterns-3" => icon("focus"),
        "checkmate-patterns-4" => icon("puzzle"),
        "piece-checkmates-2" => icon("chess-king"),
        "knight-bishop-mate" => icon("chess-knight"),
        "key-squares" => icon("key-round"),
        "opposition" => icon("arrow-left-right"),
        "rook-pawn-vs-queen" => icon("chess-pawn"),
        "rook-pawn-vs-rook" => icon("chess-rook"),
        "basic-rook-endgames" => icon("castle"),
        "intermediate-rook-endings" => icon("tower-control"),
        "practical-rook-endings" => icon("brick-wall"),
        _ => icon("graduation-cap"),
    }
}

pub struct Endgames {
    lessons: Value,
    drill_summary: Value,
    drills: Option<Entity<EndgameDrills>>,
    lesson: Option<Entity<EndgameLesson>>,
    area: Measured,
    child_subscription: Option<Subscription>,
}

impl Endgames {
    pub fn new(cx: &mut Context<Self>) -> Endgames {
        let mut hub = Endgames {
            lessons: Value::Null,
            drill_summary: Value::Null,
            drills: None,
            lesson: None,
            area: Measured::default(),
            child_subscription: None,
        };
        hub.load(cx);
        hub
    }

    /// The finished chapters and drills.
    fn load(&mut self, cx: &mut Context<Self>) {
        let core = cx.global::<AppCore>().clone();
        let (lessons, drills) = (
            core.call("runSummary", vec![json!("lesson")]),
            core.call("runSummary", vec![json!("endgame")]),
        );
        cx.spawn(async move |this, cx| match (lessons.await, drills.await) {
            (Ok(lessons), Ok(drills)) => {
                let _ = this.update(cx, |this, cx| {
                    this.lessons = lessons;
                    this.drill_summary = drills;
                    cx.notify();
                });
            }
            (Err(cause), _) | (_, Err(cause)) => crate::log::write(
                Level::Warn,
                "endgame-training",
                &format!("Could not load progress: {}", cause.message),
            ),
        })
        .detach();
    }

    fn view(cx: &App) -> String {
        crate::prefs::get::<String>(cx, VIEW, COURSES.into())
    }

    fn open(view: &str, cx: &mut App) {
        crate::prefs::set(cx, VIEW, view);
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        Self::open(COURSES, cx);
        self.drills = None;
        self.lesson = None;
        self.child_subscription = None;
        self.load(cx);
        cx.notify();
    }

    fn on_event<V>(&mut self, _: Entity<V>, event: &TrainerEvent, cx: &mut Context<Self>) {
        match event {
            TrainerEvent::Back => self.back(cx),
            TrainerEvent::Saved(summary) => {
                self.lessons = summary.clone();
                cx.notify();
            }
        }
    }

    /// `.course-card`: a course, or the drills, with how much of it is done; it opens `view`.
    fn card(
        view: &str,
        glyph: Icon,
        title: String,
        subtitle: String,
        (done, total): (usize, usize),
        width: Pixels,
        palette: &Palette,
    ) -> impl IntoElement {
        let complete = done == total;
        let view = view.to_string();
        h_flex()
            .id(SharedString::from(format!("card-{view}")))
            .test_support()
            .role(Role::Button)
            .keyboard_focus(palette.primary)
            .aria_label(title.clone())
            .w(width)
            .gap(px(14.))
            .px(px(16.))
            .py(px(14.))
            .border_1()
            .border_color(palette.border)
            .rounded(px(12.))
            .bg(palette.elevated)
            .cursor_pointer()
            .hover(|card| card.border_color(palette.primary))
            .on_click(move |_, window, cx| {
                Self::open(&view, cx);
                window.refresh();
            })
            .child(
                div()
                    .flex_none()
                    .size(px(40.))
                    .p(px(8.))
                    .rounded(px(10.))
                    .bg(tint(palette.primary, 16.))
                    .child(glyph.size(px(24.)).text_color(palette.primary)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(palette.text_muted)
                            .child(subtitle),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(8.))
                    .py(px(2.))
                    .rounded_full()
                    .text_size(px(12.))
                    .bg(if complete {
                        tint(palette.success, 18.)
                    } else {
                        palette.accented
                    })
                    .when(complete, |badge| {
                        badge
                            .text_color(palette.success)
                            .font_weight(FontWeight::SEMIBOLD)
                    })
                    .child(if complete {
                        "Done".to_string()
                    } else {
                        format!("{done} / {total}")
                    }),
            )
    }

    fn render_courses(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let width = self.area.size().width;
        let width = if width > px(0.) {
            width
        } else {
            window.viewport_size().width - px(400.)
        };
        let (count, card_width) = widgets::columns(width, CARD_MIN, CARD_GAP);
        let grid = |cards: Vec<AnyElement>| {
            let mut rows = Vec::new();
            let mut cards = cards.into_iter().peekable();
            while cards.peek().is_some() {
                rows.push(
                    h_flex()
                        .items_stretch()
                        .gap(CARD_GAP)
                        .children(cards.by_ref().take(count)),
                );
            }
            v_flex().gap(CARD_GAP).children(rows)
        };
        let group = |title: &'static str, cards: Vec<AnyElement>| {
            v_flex()
                .gap(px(10.))
                .child(widgets::heading(title, palette))
                .child(grid(cards))
        };
        let mut sections: Vec<AnyElement> = COURSE_GROUPS
            .iter()
            .map(|name| {
                let cards = courses()
                    .iter()
                    .filter(|course| course.group == *name)
                    .map(|course| {
                        let done = course
                            .chapters
                            .iter()
                            .filter(|chapter| finished(&self.lessons, &chapter.id))
                            .count();
                        Self::card(
                            &course.id,
                            course_icon(&course.id),
                            course.title.clone(),
                            course.subtitle.clone(),
                            (done, course.chapters.len()),
                            card_width,
                            palette,
                        )
                        .into_any_element()
                    })
                    .collect();
                group(name, cards).into_any_element()
            })
            .collect();
        let done = drills()
            .iter()
            .filter(|drill| finished(&self.drill_summary, &drill.id))
            .count();
        sections.push(
            group(
                "Drills",
                vec![
                    Self::card(
                        DRILLS,
                        icon("swords"),
                        "Endgame drills".into(),
                        "Win or hold against Stockfish".into(),
                        (done, drills().len()),
                        card_width,
                        palette,
                    )
                    .into_any_element(),
                ],
            )
            .into_any_element(),
        );
        div()
            .id("endgame-courses")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .px(side_padding(window))
                    .pt(PAGE_TOP)
                    .pb(PAGE_BOTTOM)
                    .child(
                        div()
                            .relative()
                            .child(self.area.probe(cx))
                            .child(v_flex().gap(px(24.)).children(sections)),
                    ),
            )
            .into_any_element()
    }
}

impl Render for Endgames {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let view = Self::view(cx);
        if let Some(course) = courses().iter().find(|course| course.id == view) {
            let current = self
                .lesson
                .as_ref()
                .filter(|lesson| lesson.read(cx).course_id() == course.id)
                .cloned();
            let lesson = current.unwrap_or_else(|| {
                let summary = self.lessons.clone();
                let lesson = cx.new(|cx| EndgameLesson::new(course.clone(), summary, window, cx));
                self.child_subscription = Some(cx.subscribe(&lesson, Self::on_event));
                self.lesson = Some(lesson.clone());
                lesson
            });
            return div().size_full().child(lesson).into_any_element();
        }
        if view == DRILLS {
            let drills = self.drills.clone().unwrap_or_else(|| {
                let drills = cx.new(|cx| EndgameDrills::new(window, cx));
                self.child_subscription = Some(cx.subscribe(&drills, Self::on_event));
                self.drills = Some(drills.clone());
                drills
            });
            return div().size_full().child(drills).into_any_element();
        }
        self.render_courses(window, cx, &palette)
    }
}

#[cfg(test)]
mod tests {
    use super::{courses, drills};

    #[test]
    fn the_shared_courses_and_drills_load() {
        assert_eq!(drills().len(), 9);
        assert_eq!(courses().len(), 14);
        assert!(courses().iter().all(|course| !course.chapters.is_empty()));
        assert!(
            courses()
                .iter()
                .flat_map(|course| &course.chapters)
                .all(|chapter| !chapter.steps.is_empty())
        );
    }
}
