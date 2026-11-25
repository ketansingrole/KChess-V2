//! Practice → Endgames: the checkmate and endgame courses, the drills
//! against Stockfish, and the endgames your own games let slip, found with the endgame
//! tablebases (`gameDbYourEndgames`) and drilled the same way. The courses and drills are the shared data the Rust rules check
//! (`data/lessons.json`, `data/training.json`); finished chapters and drills are runs the core keeps.

use kchess_contracts::calls;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::Level;
use kchess_contracts::types::{RunKind, RunSummary, YourEndgame, YourEndgames};

use super::endgame_drills::EndgameDrills;
use super::endgame_lesson::EndgameLesson;
use crate::core::AppCore;
use crate::theme::{Palette, radii, tint};
use crate::widgets;
use crate::widgets::KeyboardFocus as _;
use crate::widgets::Pressable as _;
use kchess_nav::play_layout::{Measured, PAGE_BOTTOM, PAGE_TOP, column_width, side_padding};
use kchess_ui::icon;

const VIEW: &str = "kchess:endgame-view";
const COURSES: &str = "courses";
const DRILLS: &str = "drills";
/// The endgames of your games.
const YOURS: &str = "yours";
/// Their groups: a win given away, and a draw.
const WINS: &str = "Wins let go";
const DRAWS: &str = "Draws let go";
const COURSE_GROUPS: [&str; 3] = ["Checkmates", "Pawn endgames", "Rook endgames"];
/// The cards' columns.
const CARD_MIN: Pixels = px(300.);
const CARD_GAP: Pixels = px(12.);

pub use kchess_domain::training::endgames::EndgameDrill as Drill;
pub use kchess_domain::training::lessons::{Chapter, Course, Step};

/// The endgame drills against Stockfish.
pub fn drills() -> &'static [Drill] {
    &kchess_domain::training::data::data().endgame_drills
}

/// The checkmate and endgame courses.
pub fn courses() -> &'static [Course] {
    &kchess_domain::training::lessons::data().courses
}

/// A drill or chapter finished at least once.
pub fn finished(summary: Option<&RunSummary>, id: &str) -> bool {
    super::run::best(summary, id).unwrap_or(0.) > 0.
}

/// What a course or the drills tell the hub.
pub enum TrainerEvent {
    Back,
    /// A chapter was finished: the lessons' summary as the core now keeps it.
    Saved(RunSummary),
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

/// An endgame of your games as a drill: the position before your move, its goal, and what you
/// played there.
pub fn your_drill(endgame: &YourEndgame) -> Drill {
    let dots = if endgame.side == "white" { "." } else { "..." };
    let outcome = match (endgame.goal.as_str(), endgame.became.as_str()) {
        ("win", "draw") => "only draws",
        _ => "loses",
    };
    Drill {
        id: format!("own:{}", endgame.game),
        title: endgame.material.clone(),
        group: if endgame.goal == "win" { WINS } else { DRAWS }.to_string(),
        fen: endgame.fen.clone(),
        player: endgame.side.clone(),
        goal: endgame.goal.clone(),
        level: endgame.date.clone().unwrap_or_default(),
        idea: format!(
            "Against {} you played {}{dots} {} here, which {outcome}.",
            endgame.opponent, endgame.number, endgame.san
        ),
    }
}

pub struct Endgames {
    lessons: Option<RunSummary>,
    drill_summary: Option<RunSummary>,
    drills: Option<Entity<EndgameDrills>>,
    /// The endgames of your games, once looked through.
    yours: Option<YourEndgames>,
    your_drills: Option<Entity<EndgameDrills>>,
    /// Whether there is Your games, which the card waits for.
    own: bool,
    _subscriptions: Vec<Subscription>,
    lesson: Option<Entity<EndgameLesson>>,
    area: Measured,
    child_subscription: Option<Subscription>,
}

impl Endgames {
    pub fn new(cx: &mut Context<Self>) -> Endgames {
        let databases = cx.global::<kchess_board::game_db::GameDbHandle>().0.clone();
        // Your games added or removed shows or hides its card.
        let subscriptions = vec![cx.observe(&databases, |this, _, cx| {
            let own = Self::has_own(cx);
            if own != this.own {
                this.own = own;
                this.load(cx);
                cx.notify();
            }
        })];
        let mut hub = Endgames {
            lessons: None,
            drill_summary: None,
            drills: None,
            yours: None,
            your_drills: None,
            own: Self::has_own(cx),
            _subscriptions: subscriptions,
            lesson: None,
            area: Measured::default(),
            child_subscription: None,
        };
        hub.load(cx);
        hub
    }

    fn has_own(cx: &App) -> bool {
        use kchess_contracts::types::DatabaseSource;
        cx.global::<kchess_board::game_db::GameDbHandle>()
            .0
            .read(cx)
            .databases()
            .iter()
            .any(|database| database.source == DatabaseSource::Own)
    }

    /// The finished chapters and drills, and the endgames of your games.
    fn load(&mut self, cx: &mut Context<Self>) {
        let core = cx.global::<AppCore>().clone();
        if self.own {
            let yours = core.request(calls::GameDbYourEndgames {});
            cx.spawn(async move |this, cx| match yours.await {
                Ok(yours) => {
                    this.update(cx, |this, cx| {
                        this.yours = Some(yours);
                        cx.notify();
                    })
                    .ok();
                }
                Err(cause) => crate::log::write(
                    Level::Warn,
                    "endgame-training",
                    &format!("Could not look through your endgames: {}", cause.message),
                ),
            })
            .detach();
        }
        let (lessons, drills) = (
            core.request(calls::RunSummary {
                kind: RunKind::Lesson,
            }),
            core.request(calls::RunSummary {
                kind: RunKind::Endgame,
            }),
        );
        cx.spawn(async move |this, cx| match (lessons.await, drills.await) {
            (Ok(lessons), Ok(drills)) => {
                let _ = this.update(cx, |this, cx| {
                    this.lessons = Some(lessons);
                    this.drill_summary = Some(drills);
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

    /// The lesson, while it is on screen.
    pub(crate) fn shown_lesson(&self, cx: &App) -> Option<Entity<EndgameLesson>> {
        let view = Self::view(cx);
        self.lesson
            .clone()
            .filter(|lesson| lesson.read(cx).course_id() == view)
    }

    /// The drills, while they are on screen.
    pub(crate) fn shown_drills(&self, cx: &App) -> Option<Entity<EndgameDrills>> {
        match Self::view(cx).as_str() {
            DRILLS => self.drills.clone(),
            YOURS => self.your_drills.clone(),
            _ => None,
        }
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
        self.your_drills = None;
        self.lesson = None;
        self.child_subscription = None;
        self.load(cx);
        cx.notify();
    }

    fn on_event<V>(&mut self, _: Entity<V>, event: &TrainerEvent, cx: &mut Context<Self>) {
        match event {
            TrainerEvent::Back => self.back(cx),
            TrainerEvent::Saved(summary) => {
                self.lessons = Some(summary.clone());
                cx.notify();
            }
        }
    }

    /// A course, or the drills, with how much of it is done when that counts; it opens `view`.
    fn card(
        view: &str,
        glyph: Icon,
        title: String,
        subtitle: String,
        progress: Option<(usize, usize)>,
        width: Pixels,
        palette: &Palette,
    ) -> impl IntoElement {
        let view = view.to_string();
        h_flex()
            .id(SharedString::from(format!("card-{view}")))
            .test_support()
            .role(Role::Button)
            .keyboard_focus(palette.primary)
            .aria_label(title.clone())
            .w(width)
            .gap(px(12.))
            .p(px(16.))
            .border_1()
            .border_color(palette.border)
            .rounded(radii::BOX)
            .bg(palette.elevated)
            .cursor_pointer()
            .pressable_card(palette)
            .on_click(move |_, window, cx| {
                Self::open(&view, cx);
                window.refresh();
            })
            .child(
                div()
                    .flex_none()
                    .size(px(40.))
                    .p(px(8.))
                    .rounded(radii::BOX)
                    .bg(palette.accented)
                    .child(glyph.size(px(24.)).text_color(palette.toned)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(palette.text_muted)
                            .child(subtitle),
                    ),
            )
            .children(progress.map(|(done, total)| {
                let complete = done == total;
                div()
                    .flex_none()
                    .px(px(8.))
                    .py(px(2.))
                    .rounded_full()
                    .text_size(px(12.))
                    .font_features(widgets::tabular())
                    .when(!complete, |badge| badge.text_color(palette.text_muted))
                    .when(complete, |badge| {
                        badge
                            .bg(tint(palette.success, 18.))
                            .text_color(palette.success)
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .child(if complete {
                        "Done".to_string()
                    } else {
                        format!("{done} / {total}")
                    })
            }))
    }

    /// Your endgames without the tablebases to find them: where to get them.
    fn render_no_tables(&self, window: &Window, palette: &Palette) -> AnyElement {
        div()
            .id("your-endgames")
            .test_support()
            .size_full()
            .px(side_padding(window))
            .pt(PAGE_TOP)
            .child(
                widgets::card(palette).child(
                    widgets::empty_state(
                        icon("history"),
                        "Your endgames",
                        Some(SharedString::new_static(
                            "The endgame tablebases find the endgames your games let slip",
                        )),
                        palette,
                    )
                    .py(px(32.))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                Button::new("your-endgames-back")
                                    .outline()
                                    .label("All courses")
                                    .on_click(|_, window, cx| {
                                        Self::open(COURSES, cx);
                                        window.refresh();
                                    }),
                            )
                            .child(
                                Button::new("your-endgames-tablebases")
                                    .primary()
                                    .label("Get the tablebases")
                                    .on_click(|_, window, cx| {
                                        kchess_nav::shell::open_settings(
                                            kchess_nav::sections::Section::Storage,
                                            window,
                                            cx,
                                        )
                                    }),
                            ),
                    ),
                ),
            )
            .into_any_element()
    }

    fn render_courses(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        // The page's width inside its padding, so the grid opens with its columns for the
        // window it opens in rather than settling into them a frame later.
        let width = column_width(window, Pixels::MAX, self.area.size().width);
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
                .gap(px(8.))
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
                            .filter(|chapter| finished(self.lessons.as_ref(), &chapter.id))
                            .count();
                        Self::card(
                            &course.id,
                            course_icon(&course.id),
                            course.title.clone(),
                            course.subtitle.clone(),
                            Some((done, course.chapters.len())),
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
            .filter(|drill| finished(self.drill_summary.as_ref(), &drill.id))
            .count();
        let mut cards = vec![
            Self::card(
                DRILLS,
                icon("swords"),
                "Endgame drills".into(),
                "Win or hold against Stockfish".into(),
                Some((done, drills().len())),
                card_width,
                palette,
            )
            .into_any_element(),
        ];
        // Your endgames, once Your games is looked through: those it let slip, or that the
        // tablebases are needed to find them.
        if let Some(yours) = self.yours.as_ref().filter(|_| self.own)
            && (!yours.tables || !yours.endgames.is_empty())
        {
            let done = yours
                .endgames
                .iter()
                .filter(|endgame| finished(self.drill_summary.as_ref(), &your_drill(endgame).id))
                .count();
            cards.push(
                Self::card(
                    YOURS,
                    icon("history"),
                    "Your endgames".into(),
                    if yours.tables {
                        "Win or hold what your games let slip".into()
                    } else {
                        "Needs the endgame tablebases".into()
                    },
                    yours.tables.then_some((done, yours.endgames.len())),
                    card_width,
                    palette,
                )
                .into_any_element(),
            );
        }
        sections.push(group("Drills", cards).into_any_element());
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
        if view == YOURS
            && let Some(yours) = &self.yours
        {
            if !yours.tables {
                return self.render_no_tables(window, &palette);
            }
            if !yours.endgames.is_empty() {
                let drills = self.your_drills.clone().unwrap_or_else(|| {
                    let set: Vec<Drill> = yours.endgames.iter().map(your_drill).collect();
                    let drills = cx.new(|cx| EndgameDrills::of(set, vec![WINS, DRAWS], window, cx));
                    self.child_subscription = Some(cx.subscribe(&drills, Self::on_event));
                    self.your_drills = Some(drills.clone());
                    drills
                });
                return div().size_full().child(drills).into_any_element();
            }
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
        assert!(!drills().is_empty() && !courses().is_empty());
        assert!(courses().iter().all(|course| !course.chapters.is_empty()));
        assert!(
            courses()
                .iter()
                .flat_map(|course| &course.chapters)
                .all(|chapter| !chapter.steps.is_empty())
        );
    }
}
