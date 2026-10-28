//! Create an arena: a Lichess arena run by the active account.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use super::TournamentsPage;
use crate::assets::icon;
use crate::core::AppCore;
use crate::game::rules;
use crate::theme::Palette;
use crate::widgets::{self, Choice};

/// `ARENA_CLOCK_MINUTES` (without 0), `ARENA_INCREMENTS`, `ARENA_DURATIONS`, `ARENA_WAIT_MINUTES`.
const CLOCKS: [f64; 20] = [
    0.25, 0.5, 0.75, 1., 1.5, 2., 3., 4., 5., 6., 7., 8., 10., 15., 20., 25., 30., 40., 50., 60.,
];
const INCREMENTS: [f64; 16] = [
    0., 1., 2., 3., 4., 5., 6., 7., 10., 15., 20., 25., 30., 40., 50., 60.,
];
const DURATIONS: [f64; 28] = [
    20., 25., 30., 35., 40., 45., 50., 55., 60., 70., 80., 90., 100., 110., 120., 150., 180., 210.,
    240., 270., 300., 330., 360., 420., 480., 540., 600., 720.,
];
const WAITS: [f64; 10] = [1., 2., 3., 5., 10., 15., 20., 30., 45., 60.];
const VARIANTS: [(&str, &str); 9] = [
    ("standard", "Standard"),
    ("chess960", "Chess960"),
    ("kingOfTheHill", "King of the Hill"),
    ("threeCheck", "Three-check"),
    ("antichess", "Antichess"),
    ("atomic", "Atomic"),
    ("horde", "Horde"),
    ("racingKings", "Racing Kings"),
    ("crazyhouse", "Crazyhouse"),
];
const LATER: &str = "later";

fn minutes_label(minutes: f64) -> String {
    match minutes {
        0.25 => "¼ min".into(),
        0.5 => "½ min".into(),
        0.75 => "¾ min".into(),
        m => format!("{m} min"),
    }
}

fn duration_label(minutes: f64) -> String {
    if minutes < 60. || minutes % 60. != 0. {
        format!("{minutes} min")
    } else {
        format!("{} h", minutes / 60.)
    }
}

/// A number's choice value ("1.5").
fn key(value: f64) -> String {
    value.to_string()
}

/// "2026-10-25 14:00" in local time, as milliseconds.
fn parse_start(text: &str) -> Option<f64> {
    use chrono::TimeZone as _;
    let naive = chrono::NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%d %H:%M").ok()?;
    chrono::Local
        .from_local_datetime(&naive)
        .single()
        .map(|time| time.timestamp_millis() as f64)
}

type Choices = Entity<SelectState<Vec<Choice>>>;

pub struct CreateArena {
    account: String,
    page: WeakEntity<TournamentsPage>,
    name: Entity<InputState>,
    clock: Choices,
    increment: Choices,
    duration: Choices,
    variant: Choices,
    start: Choices,
    start_at: Entity<InputState>,
    password: Entity<InputState>,
    description: Entity<TextareaState>,
    rated: bool,
    busy: bool,
    error: Option<SharedString>,
    reconnect: bool,
    _subscriptions: Vec<Subscription>,
}

impl CreateArena {
    pub fn open(
        account: String,
        page: WeakEntity<TournamentsPage>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let view = cx.new(|cx| CreateArena::new(account, page, window, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Create an arena")
                .w(px(560.))
                .child(view.clone())
        });
    }

    fn new(
        account: String,
        page: WeakEntity<TournamentsPage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> CreateArena {
        let mut select = |items: Vec<Choice>, chosen: String| {
            let state = cx.new(|cx| SelectState::new(items, None, window, cx));
            state.update(cx, |state, cx| {
                state.set_selected_value(&SharedString::from(chosen), window, cx)
            });
            state
        };
        let clock = select(
            CLOCKS
                .iter()
                .map(|m| Choice::new(key(*m), minutes_label(*m)))
                .collect(),
            key(3.),
        );
        let increment = select(
            INCREMENTS
                .iter()
                .map(|s| Choice::new(key(*s), format!("+{s} s")))
                .collect(),
            key(2.),
        );
        let duration = select(
            DURATIONS
                .iter()
                .map(|m| Choice::new(key(*m), duration_label(*m)))
                .collect(),
            key(60.),
        );
        let variant = select(
            VARIANTS
                .iter()
                .map(|(value, label)| Choice::new(*value, *label))
                .collect(),
            "standard".into(),
        );
        let start = select(
            WAITS
                .iter()
                .map(|m| Choice::new(key(*m), format!("In {m} min")))
                .chain(std::iter::once(Choice::new(LATER, "At a set time")))
                .collect(),
            key(5.),
        );
        // A set start defaults to the next round hour.
        let next_hour = {
            use chrono::Timelike as _;
            let next = chrono::Local::now() + chrono::Duration::hours(1);
            next.with_minute(0)
                .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default()
        };
        let start_at = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("YYYY-MM-DD HH:MM")
                .default_value(next_hour)
        });
        let mut subscriptions: Vec<Subscription> = [&clock, &increment, &variant, &start]
            .into_iter()
            .map(|select| cx.observe(select, |_, _, cx| cx.notify()))
            .collect();
        subscriptions.push(cx.observe(&start_at, |_, _, cx| cx.notify()));
        CreateArena {
            account,
            page,
            name: cx
                .new(|cx| InputState::new(window, cx).placeholder("Leave empty for a random name")),
            clock,
            increment,
            duration,
            variant,
            start,
            start_at,
            password: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("Players need it to join")
            }),
            description: cx.new(|cx| TextareaState::new(window, cx).rows(2)),
            rated: true,
            busy: false,
            error: None,
            reconnect: false,
            _subscriptions: subscriptions,
        }
    }

    fn number(select: &Choices, cx: &App) -> f64 {
        select
            .read(cx)
            .selected_value()
            .and_then(|value| value.parse().ok())
            .unwrap_or_default()
    }

    fn later(&self, cx: &App) -> bool {
        self.start
            .read(cx)
            .selected_value()
            .is_some_and(|value| value.as_ref() == LATER)
    }

    fn start_problem(&self, cx: &App) -> Option<&'static str> {
        if !self.later(cx) {
            return None;
        }
        let soon = chrono::Utc::now().timestamp_millis() as f64 + 60_000.;
        match parse_start(&self.start_at.read(cx).value()) {
            Some(at) if at >= soon => None,
            _ => Some("Pick a start time in the future."),
        }
    }

    /// Lichess lets third-party apps play only Rapid and Classical arena games.
    fn playable_here(&self, cx: &App) -> bool {
        let variant = self.variant.read(cx).selected_value();
        variant.is_none_or(|variant| variant.as_ref() != "crazyhouse")
            && rules(
                "canBoardSeek",
                &[
                    json!(Self::number(&self.clock, cx)),
                    json!(Self::number(&self.increment, cx)),
                ],
            )
            .ok()
                == Some(json!(true))
    }

    fn arena(&self, cx: &App) -> Value {
        let mut arena = json!({
            "clockTime": Self::number(&self.clock, cx),
            "clockIncrement": Self::number(&self.increment, cx),
            "minutes": Self::number(&self.duration, cx),
            "variant": self.variant.read(cx).selected_value().map_or("standard".into(), |v| v.to_string()),
            "rated": self.rated,
        });
        // Unset fields are left out: the validators reject null.
        let name = self.name.read(cx).value().trim().to_string();
        if !name.is_empty() {
            arena["name"] = json!(name);
        }
        if self.later(cx) {
            if let Some(at) = parse_start(&self.start_at.read(cx).value()) {
                arena["startDate"] = json!(at);
            }
        } else {
            arena["waitMinutes"] = json!(Self::number(&self.start, cx));
        }
        let password = self.password.read(cx).value().to_string();
        if !password.is_empty() {
            arena["password"] = json!(password);
        }
        let description = self.description.read(cx).value().trim().to_string();
        if !description.is_empty() {
            arena["description"] = json!(description);
        }
        arena
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.start_problem(cx).is_some() {
            return;
        }
        self.busy = true;
        self.error = None;
        self.reconnect = false;
        cx.notify();
        let call = cx.global::<AppCore>().call(
            "createTournament",
            vec![json!(self.account), self.arena(cx)],
        );
        let page = self.page.clone();
        cx.spawn_in(window, async move |this, cx| {
            let answer = call.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match answer {
                    Ok(result) if result.get("needsReconnect").is_some() => this.reconnect = true,
                    Ok(created) => {
                        window.close_dialog(cx);
                        let _ = page.update(cx, |page, cx| page.created(&created, window, cx));
                    }
                    Err(cause) => this.error = Some(cause.message.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for CreateArena {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let later = self.later(cx);
        let problem = self.start_problem(cx);
        let playable = self.playable_here(cx);
        let pair = |a: Div, b: Div| h_flex().gap(px(16.)).child(a.flex_1()).child(b.flex_1());
        v_flex()
            .gap(px(14.))
            .child(widgets::field("Name", Input::new(&self.name), None, &palette))
            .child(pair(
                widgets::field("Clock", Select::new(&self.clock), None, &palette),
                widgets::field("Increment", Select::new(&self.increment), None, &palette),
            ))
            .child(pair(
                widgets::field("Duration", Select::new(&self.duration), None, &palette),
                widgets::field("Variant", Select::new(&self.variant), None, &palette),
            ))
            .child(if later {
                pair(
                    widgets::field("Starts", Select::new(&self.start), None, &palette),
                    widgets::field(
                        "Start time",
                        Input::new(&self.start_at),
                        problem.map(SharedString::from),
                        &palette,
                    ),
                )
            } else {
                widgets::field("Starts", Select::new(&self.start), None, &palette)
            })
            .child(widgets::field(
                "Entry code",
                Input::new(&self.password).mask_toggle(),
                None,
                &palette,
            ))
            .child(widgets::field(
                "Description",
                Textarea::new(&self.description),
                None,
                &palette,
            ))
            .child(
                h_flex()
                    .justify_between()
                    .text_size(px(14.))
                    .child("Rated")
                    .child(
                        Switch::new("arena-rated")
                            .checked(self.rated)
                            .accessibility_label("Rated")
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.rated = *checked;
                                cx.notify();
                            })),
                    ),
            )
            .when(!playable, |form| {
                form.child(
                    h_flex()
                        .gap(px(8.))
                        .p(px(10.))
                        .rounded(px(8.))
                        .bg(palette.accented)
                        .text_size(px(13.))
                        .child(icon("eye").size(px(16.)).flex_none())
                        .child(div().flex_1().min_w_0().child(
                            "KChess can play only Rapid and Classical arena games; players will join faster ones on Lichess.",
                        )),
                )
            })
            .when(self.reconnect, |form| {
                form.child(widgets::StatusBanner::new(widgets::Tone::Alert, icon("key-round"), "Lichess needs a new permission".into()).detail(format!(
                            "Connect @{} again to allow creating tournaments.",
                            self.account
                        )
                        .into(),).extra(h_flex()
                            .mt(px(6.))
                            .child(
                                Button::new("arena-reconnect")
                                    .small()
                                    .outline()
                                    .icon(icon("link"))
                                    .label("Reconnect")
                                    .on_click(|_, window, cx| {
                                        window.close_dialog(cx);
                                        crate::accounts::handle(cx).update(cx, |accounts, cx| {
                                            accounts.connect(window, cx)
                                        })
                                    }),
                            )
                            .into_any_element(),))
            })
            .children(self.error.clone().map(|error| {
                div()
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(error)
            }))
            .child(
                h_flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("arena-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("arena-create")
                            .primary()
                            .icon(icon("plus"))
                            .label("Create arena")
                            .loading(self.busy)
                            .disabled(problem.is_some() || self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{duration_label, minutes_label, parse_start};

    #[test]
    fn choices_read_like_lichess() {
        assert_eq!(minutes_label(0.5), "½ min");
        assert_eq!(minutes_label(3.), "3 min");
        assert_eq!(duration_label(90.), "90 min");
        assert_eq!(duration_label(120.), "2 h");
    }

    #[test]
    fn a_set_start_reads_as_local_time() {
        assert!(parse_start("2026-10-25 14:00").is_some());
        assert!(parse_start("tomorrow").is_none());
    }
}
