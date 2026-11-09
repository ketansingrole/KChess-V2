//! Tournaments: Lichess's arena schedule as a
//! timeline, one tournament's details (join, standings, the top game, games in progress and
//! totals), and creating an arena.

use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use crate::assets::icon;
use crate::core::AppCore;
use crate::play_layout::{Measured, PAGE_BOTTOM, PAGE_TOP, side_padding};
use crate::theme::Palette;
use crate::widgets::{self, Pill};

mod create;
mod event;
mod timeline;

/// Standings and games move quickly while a tournament runs.
const REFRESH_EVERY: Duration = Duration::from_secs(15);
/// An event that has not started only needs to notice that it has.
const WAITING_REFRESH_EVERY: Duration = Duration::from_secs(60);
/// How often the top game's running clock is redrawn between refreshes.
const CLOCK_TICK: Duration = Duration::from_millis(250);

/// Whether an event of `status`, last read `since` ago, is due a refresh. `since` counts from
/// the answer, which comes a little after the tick that asked: the slack keeps a running event on
/// every tick rather than every other one.
fn refresh_due(status: &Value, since: Duration) -> bool {
    const SLACK: Duration = Duration::from_secs(2);
    match status.as_str() {
        Some("started") => since + SLACK >= REFRESH_EVERY,
        Some("created") => since + SLACK >= WAITING_REFRESH_EVERY,
        _ => false,
    }
}

/// `control`: "3+2", "½+0".
pub fn control(clock: &Value) -> String {
    let minutes = clock["limit"].as_f64().unwrap_or_default() / 60.;
    let base = match minutes {
        0.25 => "¼".to_string(),
        0.5 => "½".to_string(),
        0.75 => "¾".to_string(),
        m if m.fract() == 0. => format!("{m:.0}"),
        m => m.to_string(),
    };
    format!("{base}+{}", clock["increment"].as_f64().unwrap_or_default())
}

/// The tournament open, and what is known of it.
struct Open {
    system: String,
    id: String,
    detail: Option<Value>,
    /// When `detail` arrived: the top game's clocks are as they were then.
    received: Instant,
    error: Option<SharedString>,
    page: usize,
    epoch: u64,
}

/// The page's actions, as its buttons offer them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Action {
    Refresh,
    Create,
    Back,
    Join,
    Game,
    Pause,
    Withdraw,
}

pub struct TournamentsPage {
    /// Only what KChess can play (`playable`), or everything.
    playable_only: bool,
    list: Option<Value>,
    loading: bool,
    error: Option<SharedString>,
    /// Each list request; an answer to an earlier one (another account's) is dropped.
    list_request: u64,
    /// `JoinedTournament`s the core keeps.
    joined: Vec<Value>,
    open: Option<Open>,
    password: Entity<InputState>,
    busy: bool,
    needs_reconnect: bool,
    featured_board: Option<Entity<crate::board::ChessBoard>>,
    schedule: ScrollHandle,
    schedule_area: Measured,
    scrolled_to_now: bool,
    focus: FocusHandle,
    /// Whether the page is on screen: the open event is refreshed only then.
    visible: bool,
    _refresh: Task<()>,
    _clock: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for TournamentsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TournamentsPage {
    /// The name of the tournament shown, once its details came.
    #[cfg(test)]
    pub(crate) fn shown_name(&self) -> Option<String> {
        let detail = self.open.as_ref()?.detail.as_ref()?;
        detail["name"].as_str().map(str::to_string)
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> TournamentsPage {
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Entry code (if any)")
        });
        let accounts = crate::accounts::handle(cx);
        let subscriptions = vec![
            cx.observe(&accounts, |_, _, cx| cx.notify()),
            // Return in the entry code joins, as the Join button beside it does.
            cx.subscribe_in(
                &password,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event
                        && this.may_join(cx)
                    {
                        this.join(window, cx);
                    }
                },
            ),
        ];
        let refresh = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(REFRESH_EVERY).await;
                let alive = this
                    .update(cx, |this, cx| {
                        if this.refresh_due() {
                            this.load_event(cx);
                        }
                    })
                    .is_ok();
                if !alive {
                    return;
                }
            }
        });
        // Between refreshes the top game's side to move keeps losing time.
        let clock = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CLOCK_TICK).await;
                let alive = this
                    .update(cx, |this, cx| {
                        let running = this.open.as_ref().is_some_and(|open| {
                            open.detail.as_ref().is_some_and(|detail| {
                                detail["status"] != json!("finished")
                                    && event::featured_running(&detail["featured"]).is_some()
                            })
                        });
                        if running {
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !alive {
                    return;
                }
            }
        });
        let mut page = TournamentsPage {
            playable_only: false,
            list: None,
            loading: false,
            error: None,
            list_request: 0,
            joined: Vec::new(),
            open: None,
            password,
            busy: false,
            needs_reconnect: false,
            featured_board: None,
            schedule: ScrollHandle::new(),
            schedule_area: Measured::default(),
            scrolled_to_now: false,
            focus: cx.focus_handle(),
            visible: true,
            _refresh: refresh,
            _clock: clock,
            _subscriptions: subscriptions,
        };
        page.refresh(cx);
        page.reload_joined(cx);
        page
    }

    fn account(cx: &App) -> Option<String> {
        crate::accounts::handle(cx).read(cx).active(cx)
    }

    /// The account's argument: its name, or empty for anonymous lookups.
    fn account_arg(cx: &App) -> Value {
        json!(Self::account(cx).unwrap_or_default())
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.list_request += 1;
        let request = self.list_request;
        self.loading = true;
        self.error = None;
        cx.notify();
        let call = cx
            .global::<AppCore>()
            .call("tournaments", vec![Self::account_arg(cx)]);
        cx.spawn(async move |this, cx| {
            let answer = call.await;
            let _ = this.update(cx, |this, cx| {
                // A later refresh (another account's) owns the list and its spinner now.
                if request != this.list_request {
                    return;
                }
                this.loading = false;
                match answer {
                    Ok(list) => this.list = Some(list),
                    Err(cause) => this.error = Some(cause.message.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn reload_joined(&mut self, cx: &mut Context<Self>) {
        let call = cx.global::<AppCore>().call("joinedTournaments", Vec::new());
        cx.spawn(async move |this, cx| {
            if let Ok(joined) = call.await {
                let _ = this.update(cx, |this, cx| {
                    this.joined = joined.as_array().cloned().unwrap_or_default();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Joined from KChess by `account` and not over yet.
    fn is_joined(&self, system: &str, id: &str, account: Option<&str>) -> bool {
        let now = chrono::Utc::now().timestamp_millis() as f64;
        let Some(account) = account else {
            return false;
        };
        self.joined.iter().any(|entry| {
            entry["system"] == json!(system)
                && entry["id"] == json!(id)
                && entry["account"]
                    .as_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case(account))
                && entry["until"].as_f64().is_some_and(|until| until > now)
        })
    }

    /// Show one tournament (`?system=arena&id=…`).
    pub fn show_event(&mut self, system: &str, id: &str, cx: &mut Context<Self>) {
        let same = self
            .open
            .as_ref()
            .is_some_and(|open| open.system == system && open.id == id);
        if !same {
            self.open = Some(Open {
                system: system.to_string(),
                id: id.to_string(),
                detail: None,
                received: Instant::now(),
                error: None,
                page: 1,
                epoch: 0,
            });
        }
        self.load_event(cx);
        cx.notify();
    }

    /// A tournament known only by its id (a game's): Swiss if KChess knows it as one, else arena.
    pub fn show_known_event(&mut self, id: &str, cx: &mut Context<Self>) {
        let swiss = self
            .joined
            .iter()
            .any(|entry| entry["id"] == json!(id) && entry["system"] == json!("swiss"))
            || self.list.as_ref().is_some_and(|list| {
                list["swiss"]
                    .as_array()
                    .is_some_and(|swiss| swiss.iter().any(|entry| entry["id"] == json!(id)))
            });
        self.show_event(if swiss { "swiss" } else { "arena" }, id, cx);
    }

    fn close_event(&mut self, cx: &mut Context<Self>) {
        self.open = None;
        self.scrolled_to_now = false;
        cx.notify();
    }

    /// Whether the open event is due a refresh: only while the page is shown, often while it
    /// runs, now and then while it waits to start, never once it has finished.
    fn refresh_due(&self) -> bool {
        self.visible
            && self.open.as_ref().is_some_and(|open| {
                open.detail
                    .as_ref()
                    .is_some_and(|detail| refresh_due(&detail["status"], open.received.elapsed()))
            })
    }

    /// The page left the screen: the open event stops refreshing.
    pub fn hidden(&mut self) {
        self.visible = false;
    }

    /// The page is back: an open event that is due is refreshed at once.
    pub fn shown(&mut self, cx: &mut Context<Self>) {
        self.visible = true;
        if self.refresh_due() {
            self.load_event(cx);
        }
    }

    fn load_event(&mut self, cx: &mut Context<Self>) {
        let account = Self::account_arg(cx);
        let Some(open) = &mut self.open else {
            return;
        };
        open.epoch += 1;
        let epoch = open.epoch;
        let (system, id, page) = (open.system.clone(), open.id.clone(), open.page);
        let call = cx.global::<AppCore>().call(
            "tournament",
            vec![json!(system), json!(id), account, json!(page)],
        );
        cx.spawn(async move |this, cx| {
            let answer = call.await;
            let _ =
                this.update(cx, |this, cx| {
                    // Every tournament opened counts from 1: a reply is this one's only if it also
                    // names the tournament still open.
                    let Some(open) = this.open.as_mut().filter(|open| {
                        open.epoch == epoch && open.system == system && open.id == id
                    }) else {
                        return;
                    };
                    match answer {
                        Ok(detail) => {
                            open.detail = Some(detail);
                            open.received = Instant::now();
                            open.error = None;
                        }
                        Err(cause) => open.error = Some(cause.message.into()),
                    }
                    // The core forgets an arena that is over or that you left.
                    this.reload_joined(cx);
                    cx.notify();
                });
        })
        .detach();
    }

    fn go_to_page(&mut self, page: usize, cx: &mut Context<Self>) {
        if let Some(open) = &mut self.open {
            open.page = page.max(1);
        }
        self.load_event(cx);
    }

    /// Whether the open event can be joined now: not over, not joined yet, playable here and open
    /// to the account, with no action under way.
    pub(super) fn may_join(&self, cx: &App) -> bool {
        let Some(detail) = self.open.as_ref().and_then(|open| open.detail.as_ref()) else {
            return false;
        };
        !self.busy
            && detail["status"] != json!("finished")
            && !self.joined_now(detail, cx)
            && detail["playable"] == json!(true)
            && detail["verdicts"]["accepted"] != json!(false)
    }

    /// `join`: with the entry code, if any; a login without the permission asks to reconnect.
    fn join(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = self.open.as_ref().and_then(|open| open.detail.clone()) else {
            return;
        };
        let Some(account) = Self::account(cx) else {
            Self::ask_connect(window, cx);
            return;
        };
        let password = self.password.read(cx).value().trim().to_string();
        let mut args = vec![
            detail["system"].clone(),
            detail["id"].clone(),
            json!(account),
        ];
        // An unset code is left out: the validators reject null.
        if !password.is_empty() {
            args.push(json!(password));
        }
        self.act(
            "joinTournament",
            args,
            Some(if detail["system"] == json!("arena") {
                format!(
                    "Joined {}. Games open here automatically when you are paired.",
                    detail["name"].as_str().unwrap_or_default()
                )
            } else {
                format!(
                    "Joined {}. Each round's game opens here automatically.",
                    detail["name"].as_str().unwrap_or_default()
                )
            }),
            window,
            cx,
        );
    }

    fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = self.open.as_ref().and_then(|open| open.detail.clone()) else {
            return;
        };
        let Some(account) = Self::account(cx) else {
            return;
        };
        self.act(
            "leaveTournament",
            vec![
                detail["system"].clone(),
                detail["id"].clone(),
                json!(account),
            ],
            None,
            window,
            cx,
        );
    }

    /// Join or leave, then read the tournament again.
    fn act(
        &mut self,
        method: &'static str,
        args: Vec<Value>,
        done: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.needs_reconnect = false;
        cx.notify();
        let call = cx.global::<AppCore>().call(method, args);
        cx.spawn_in(window, async move |this, cx| {
            let answer = call.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match answer {
                    Ok(result) if result.get("needsReconnect").is_some() => {
                        this.needs_reconnect = true
                    }
                    Ok(_) => {
                        this.password
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        if let Some(done) = done {
                            window.push_notification(done, cx);
                        }
                        this.reload_joined(cx);
                        this.load_event(cx);
                    }
                    Err(cause) => window.push_notification(cause.message, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn ask_connect(window: &mut Window, cx: &mut App) {
        crate::widgets::open_alert_dialog(
            window,
            cx,
            |dialog: gpui_kit::component::dialog::AlertDialog, _, _| {
                dialog
                    .title("Connect Lichess to join")
                    .description("Connect your Lichess account to join or create tournaments")
                    .ok_text("Connect Lichess")
                    .cancel_text("Keep browsing")
                    .show_cancel(true)
                    .on_ok(|_, window, cx| {
                        crate::accounts::handle(cx)
                            .update(cx, |accounts, cx| accounts.connect(window, cx));
                        true
                    })
            },
        );
    }

    /// What the page's buttons can do now: the schedule's, or the open event's.
    pub(crate) fn actions(&self, cx: &App) -> Vec<Action> {
        let Some(open) = &self.open else {
            let mut actions = Vec::new();
            if !self.loading {
                actions.push(Action::Refresh);
            }
            actions.push(Action::Create);
            return actions;
        };
        // Create arena stays in the header while an event is open.
        let mut actions = vec![Action::Back];
        actions.extend(self.event_actions(open.detail.as_ref(), cx));
        actions.push(Action::Create);
        actions
    }

    /// The open event's own actions: join it, or go to your game and leave it.
    fn event_actions(&self, detail: Option<&Value>, cx: &App) -> Vec<Action> {
        let Some(detail) = detail else {
            return Vec::new();
        };
        if detail["status"] == json!("finished") || self.busy {
            return Vec::new();
        }
        if !self.joined_now(detail, cx) {
            let open =
                detail["playable"] == json!(true) && detail["verdicts"]["accepted"] != json!(false);
            return if open { vec![Action::Join] } else { Vec::new() };
        }
        let mut actions = Vec::new();
        if detail["me"]["gameId"].is_string() {
            actions.push(Action::Game);
        }
        actions.push(if detail["system"] == json!("arena") {
            Action::Pause
        } else {
            Action::Withdraw
        });
        actions
    }

    /// Run an action as its button does, if it is still allowed.
    pub(crate) fn run(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.actions(cx).contains(&action) {
            return;
        }
        match action {
            Action::Refresh => self.refresh(cx),
            Action::Create => self.start_create(window, cx),
            Action::Back => self.close_event(cx),
            Action::Join => self.join(window, cx),
            Action::Game => {
                let game = self
                    .open
                    .as_ref()
                    .and_then(|open| open.detail.as_ref())
                    .and_then(|detail| detail["me"]["gameId"].as_str().map(str::to_string));
                if let Some(id) = game {
                    crate::shell::open_online_game(&id, window, cx);
                }
            }
            Action::Pause | Action::Withdraw => self.leave(window, cx),
        }
    }

    fn start_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(account) = Self::account(cx) else {
            Self::ask_connect(window, cx);
            return;
        };
        let page = cx.entity().downgrade();
        create::CreateArena::open(account, page, window, cx);
    }

    /// An arena was made: show it.
    fn created(&mut self, created: &Value, window: &mut Window, cx: &mut Context<Self>) {
        window.push_notification(
            format!(
                "Created {}",
                created["name"].as_str().unwrap_or("the arena")
            ),
            cx,
        );
        self.refresh(cx);
        if let Some(id) = created["id"].as_str() {
            self.show_event("arena", id, cx);
        }
    }

    fn reconnect_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.needs_reconnect {
            return None;
        }
        let account = Self::account(cx).unwrap_or_default();
        Some(
            widgets::StatusBanner::new(
                widgets::Tone::Alert,
                icon("key-round"),
                "Lichess needs a new permission".into(),
            )
            .detail(
                format!("Connect @{account} again to allow joining and creating tournaments")
                    .into(),
            )
            .extra(
                h_flex()
                    .child(
                        Button::new("tournaments-reconnect")
                            .small()
                            .outline()
                            .icon(icon("link"))
                            .label("Reconnect")
                            .on_click(|_, window, cx| {
                                crate::accounts::handle(cx)
                                    .update(cx, |accounts, cx| accounts.connect(window, cx))
                            }),
                    )
                    .into_any_element(),
            )
            .into_any_element(),
        )
    }
}

impl Render for TournamentsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let showing_event = self.open.is_some();
        let failed = self.list.is_none() && self.error.is_some();
        let page = cx.entity().downgrade();
        let choose = move |index: usize, _: &mut Window, cx: &mut App| {
            let _ = page.update(cx, |page, cx| {
                page.playable_only = index == 0;
                cx.notify();
            });
        };
        let header = h_flex()
            .gap(px(8.))
            .flex_wrap()
            // The key keeps room for two of its entries a line; when the controls leave it less,
            // they wrap below it rather than squeezing it into a column.
            // The key names the colours of a listed schedule, so it waits for one.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when(!showing_event && self.list.is_some(), |lead| {
                        lead.min_w(px(320.)).child(timeline::legend(&palette))
                    }),
            )
            // The controls wrap as one group.
            .child(
                h_flex()
                    .gap(px(8.))
                    .items_center()
                    .when(!showing_event, |row| {
                        row.child(widgets::pills(
                            "tournaments-filter",
                            vec![
                                Pill {
                                    icon: None,
                                    label: "Playable here".into(),
                                    badge: None,
                                    disabled: false,
                                },
                                Pill {
                                    icon: None,
                                    label: "All".into(),
                                    badge: None,
                                    disabled: false,
                                },
                            ],
                            usize::from(!self.playable_only),
                            choose,
                            true,
                            &palette,
                        ))
                        // A list that failed to load offers Try again in its place instead.
                        .when(!failed, |row| {
                            row.child(
                                Button::new("tournaments-refresh")
                                    .small()
                                    .outline()
                                    .icon(icon("refresh-cw"))
                                    .label("Refresh")
                                    .loading(self.loading)
                                    .disabled(self.loading)
                                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                            )
                        })
                    })
                    .child(
                        Button::new("tournaments-create")
                            .small()
                            .primary()
                            .icon(icon("plus"))
                            .label("Create arena")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.start_create(window, cx)),
                            ),
                    ),
            );
        let reconnect = self.reconnect_notice(cx);
        let body = if showing_event {
            self.event_view(cx, &palette)
        } else {
            self.schedule_view(window, cx, &palette)
        };
        div()
            .id("tournaments-page")
            .track_focus(&self.focus)
            // Escape goes back to the list from an event, while no text field has the keys.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.open.is_some()
                    && crate::board::page_has_keys(&this.focus, window, cx)
                    && widgets::is_escape(event)
                {
                    this.close_event(cx);
                    cx.stop_propagation();
                }
            }))
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, window, cx| {
                if event.button == MouseButton::Left {
                    window.focus(&this.focus, cx);
                }
            }))
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .gap(px(16.))
                    .px(side_padding(window))
                    .pt(PAGE_TOP)
                    .pb(PAGE_BOTTOM)
                    .child(header)
                    .children(reconnect)
                    .child(body),
            )
    }
}

#[cfg(test)]
mod tests {
    /// Overlapping list requests (a refresh, then the account changed) let the first answer
    /// clear the spinner while the second was on its way, and land over it if it came later.
    #[gpui_kit::test]
    fn only_the_latest_tournament_list_lands(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let mut app = crate::e2e::Harness::start(cx);
        let (first, second) = app.with_window(|window, cx| {
            let page = cx.new(|cx| super::TournamentsPage::new(window, cx));
            page.update(cx, |page, cx| {
                page.refresh(cx);
                let first = page.list_request;
                page.refresh(cx);
                (first, page.list_request)
            })
        });
        assert_ne!(first, second);
    }

    #[test]
    fn only_a_running_event_refreshes_every_tick_and_a_waiting_one_each_minute() {
        use super::{REFRESH_EVERY, WAITING_REFRESH_EVERY, refresh_due};
        use serde_json::json;
        use std::time::Duration;
        let tick = REFRESH_EVERY - Duration::from_millis(300);
        assert!(refresh_due(&json!("started"), tick));
        assert!(!refresh_due(&json!("started"), Duration::from_secs(1)));
        // An upcoming event used to be refreshed every tick, for hours before its start.
        assert!(!refresh_due(&json!("created"), tick));
        assert!(refresh_due(
            &json!("created"),
            WAITING_REFRESH_EVERY - Duration::from_millis(300)
        ));
        assert!(!refresh_due(&json!("finished"), Duration::from_secs(3600)));
    }

    use super::control;
    use serde_json::json;

    #[test]
    fn time_controls_read_like_lichess() {
        assert_eq!(control(&json!({ "limit": 180, "increment": 2 })), "3+2");
        assert_eq!(control(&json!({ "limit": 30, "increment": 0 })), "½+0");
        assert_eq!(control(&json!({ "limit": 90, "increment": 1 })), "1.5+1");
    }
}
