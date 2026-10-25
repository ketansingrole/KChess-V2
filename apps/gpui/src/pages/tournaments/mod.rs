//! Tournaments (`pages/tournaments.vue`, `stores/tournaments.ts`): Lichess's arena schedule as a
//! timeline, one tournament's details (join, standings, the top game, games in progress and
//! totals), and creating an arena.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::InputState;
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
    error: Option<SharedString>,
    page: usize,
    epoch: u64,
}

pub struct TournamentsPage {
    /// Only what KChess can play (`playable`), or everything.
    playable_only: bool,
    list: Option<Value>,
    loading: bool,
    error: Option<SharedString>,
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
    _refresh: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for TournamentsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TournamentsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> TournamentsPage {
        let password = cx.new(|cx| InputState::new(window, cx).placeholder("Entry code (if any)"));
        let accounts = crate::accounts::handle(cx);
        let subscriptions = vec![cx.observe(&accounts, |_, _, cx| cx.notify())];
        let refresh = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(REFRESH_EVERY).await;
                let alive = this
                    .update(cx, |this, cx| {
                        let running = this.open.as_ref().is_some_and(|open| {
                            open.detail
                                .as_ref()
                                .is_some_and(|detail| detail["status"] != json!("finished"))
                        });
                        if running {
                            this.load_event(cx);
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
            _refresh: refresh,
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
        self.loading = true;
        self.error = None;
        cx.notify();
        let call = cx
            .global::<AppCore>()
            .call("tournaments", vec![Self::account_arg(cx)]);
        cx.spawn(async move |this, cx| {
            let answer = call.await;
            let _ = this.update(cx, |this, cx| {
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
                error: None,
                page: 1,
                epoch: 0,
            });
        }
        self.load_event(cx);
        cx.notify();
    }

    fn close_event(&mut self, cx: &mut Context<Self>) {
        self.open = None;
        self.scrolled_to_now = false;
        cx.notify();
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
            let _ = this.update(cx, |this, cx| {
                let Some(open) = this.open.as_mut().filter(|open| open.epoch == epoch) else {
                    return;
                };
                match answer {
                    Ok(detail) => {
                        open.detail = Some(detail);
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
        window.open_alert_dialog(
            cx,
            |dialog: gpui_kit::component::dialog::AlertDialog, _, _| {
                dialog
                    .title("Connect Lichess to join")
                    .description("Connect your Lichess account to join or create tournaments.")
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
                "Created {}.",
                created["name"].as_str().unwrap_or("the arena")
            ),
            cx,
        );
        self.refresh(cx);
        if let Some(id) = created["id"].as_str() {
            self.show_event("arena", id, cx);
        }
    }

    fn reconnect_notice(&self, cx: &mut Context<Self>, palette: &Palette) -> Option<AnyElement> {
        if !self.needs_reconnect {
            return None;
        }
        let account = Self::account(cx).unwrap_or_default();
        Some(
            widgets::status_banner(
                widgets::Tone::Alert,
                icon("key-round"),
                "Lichess needs a new permission".into(),
                Some(
                    format!("Connect @{account} again to allow joining and creating tournaments.")
                        .into(),
                ),
                Some(
                    h_flex()
                        .mt(px(6.))
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
                ),
                palette,
            )
            .into_any_element(),
        )
    }
}

impl Render for TournamentsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let showing_event = self.open.is_some();
        let page = cx.entity().downgrade();
        let choose = move |index: usize, _: &mut Window, cx: &mut App| {
            let _ = page.update(cx, |page, cx| {
                page.playable_only = index == 0;
                cx.notify();
            });
        };
        let header = h_flex()
            .gap(px(10.))
            .flex_wrap()
            .child(div().flex_1().min_w_0().when(!showing_event, |lead| {
                lead.child(timeline::legend(&palette))
            }))
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
                .child(
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
            .child(
                Button::new("tournaments-create")
                    .small()
                    .primary()
                    .icon(icon("plus"))
                    .label("Create arena")
                    .on_click(cx.listener(|this, _, window, cx| this.start_create(window, cx))),
            );
        let reconnect = self.reconnect_notice(cx, &palette);
        let body = if showing_event {
            self.event_view(cx, &palette)
        } else {
            self.schedule_view(window, cx, &palette)
        };
        div()
            .id("tournaments-page")
            .track_focus(&self.focus)
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
    use super::control;
    use serde_json::json;

    #[test]
    fn time_controls_read_like_lichess() {
        assert_eq!(control(&json!({ "limit": 180, "increment": 2 })), "3+2");
        assert_eq!(control(&json!({ "limit": 30, "increment": 0 })), "½+0");
        assert_eq!(control(&json!({ "limit": 90, "increment": 1 })), "1.5+1");
    }
}
