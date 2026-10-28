//! Watch: Lichess TV by channel, the games followed
//! players are playing, and tournament broadcasts. One feed streams at a time, and only while the
//! page is shown: leaving it stops the stream.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use crate::assets::icon;
use crate::core::{AppCore, CoreEvent};
use crate::online::OnlineHandle;
use crate::play_layout::{PAGE_BOTTOM, PAGE_TOP, PlayLayout, side_padding};
use crate::theme::Palette;
use crate::widgets::{self, KeyboardFocus as _, Look, Pill};

mod broadcast_engine;
mod broadcasts;
mod live;

use broadcasts::Broadcasts;
use live::Live;

const TAB_KEY: &str = "kchess:watch-tab";
const CHANNEL_KEY: &str = "kchess:watch-channel";
/// Clocks are redrawn this often while something is being watched.
const TICK: Duration = Duration::from_millis(500);
/// The tabs' row and the gap under it, above the board.
const HEADER: Pixels = px(48.);
/// Updates that arrive before the core says which session they belong to (kept, newest last).
const EARLY_KEPT: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Tv,
    Friends,
    Broadcasts,
}

impl Tab {
    fn key(self) -> &'static str {
        match self {
            Tab::Tv => "tv",
            Tab::Friends => "friends",
            Tab::Broadcasts => "broadcasts",
        }
    }

    fn of(key: &str) -> Tab {
        match key {
            "friends" => Tab::Friends,
            "broadcasts" => Tab::Broadcasts,
            _ => Tab::Tv,
        }
    }
}

/// What is being watched: a TV channel or one game.
#[derive(Clone, PartialEq)]
enum Target {
    Channel(String),
    Game(String),
}

impl Target {
    fn json(&self) -> Value {
        match self {
            Target::Channel(channel) => json!({ "channel": channel }),
            Target::Game(id) => json!({ "gameId": id }),
        }
    }
}

/// Which kind of answer is awaited, so its early updates are kept for it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    Watch,
    Round,
}

/// `TvChannel`'s icon (`CHANNEL_ICONS`).
pub fn channel_icon(key: &str) -> gpui_kit::component::Icon {
    match key {
        "best" => icon("crown"),
        "bullet" => icon("zap"),
        "blitz" => icon("flame"),
        "rapid" => icon("rabbit"),
        "classical" => icon("turtle"),
        "ultraBullet" => icon("rocket"),
        "chess960" => icon("dices"),
        "kingOfTheHill" => icon("mountain"),
        "threeCheck" => icon("layers"),
        "antichess" => icon("shield-off"),
        "atomic" => icon("atom"),
        "horde" => icon("grid-3x3"),
        "racingKings" => icon("flag"),
        "crazyhouse" => icon("house"),
        "bot" => icon("bot"),
        "computer" => icon("cpu"),
        _ => icon("tv"),
    }
}

/// "GM Name (2800)": a player as Lichess labels them.
pub fn label(player: &Value) -> String {
    let mut text = String::new();
    if let Some(title) = player["title"].as_str() {
        text.push_str(title);
        text.push(' ');
    }
    text.push_str(player["name"].as_str().unwrap_or("?"));
    if let Some(rating) = player["rating"].as_f64() {
        text.push_str(&format!(" ({rating})"));
    }
    text
}

pub struct WatchPage {
    tab: Tab,
    channels: Vec<Value>,
    target: Option<Target>,
    /// The connection to the feed (`WatchState`), once known.
    phase: Option<String>,
    error: Option<SharedString>,
    session: Option<f64>,
    round_session: Option<f64>,
    pending: Option<Pending>,
    early: Vec<CoreEvent>,
    /// Each request; answers to an older one are dropped.
    epoch: u64,
    live: Live,
    broadcasts: Broadcasts,
    /// Followed players' presence by lowercase name.
    presence: std::collections::HashMap<String, Value>,
    presence_error: Option<SharedString>,
    layout: PlayLayout,
    shown: bool,
    focus: FocusHandle,
    _tick: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for WatchPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl WatchPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> WatchPage {
        let events = cx.global::<AppCore>().events.clone();
        let online = cx.global::<OnlineHandle>().0.clone();
        let accounts = crate::accounts::handle(cx);
        let covers = crate::remote_image::handle(cx);
        let subscriptions = vec![
            cx.subscribe_in(&events, window, |this, _, event: &CoreEvent, window, cx| {
                if event.name.starts_with("watch:") {
                    this.receive(event.clone(), cx);
                }
                if event.name == "engine:analysis" {
                    this.engine_update(&event.payload, cx);
                }
                if event.name == crate::core::RECONNECTED {
                    this.reconnected(window, cx);
                }
            }),
            // The player's own game pauses the engine and leaves an explored line.
            cx.observe(&online, |this, online, cx| {
                if !online.read(cx).assistance_allowed() {
                    this.broadcasts.return_to_game();
                }
                this.engine_changed(cx);
            }),
            cx.observe(&accounts, |_, _, cx| cx.notify()),
            // A broadcast's cover arrived.
            cx.observe(&covers, |_, _, cx| cx.notify()),
        ];
        let tab = Tab::of(&crate::prefs::get::<String>(cx, TAB_KEY, "tv".into()));
        WatchPage {
            tab,
            channels: Vec::new(),
            target: None,
            phase: None,
            error: None,
            session: None,
            round_session: None,
            pending: None,
            early: Vec::new(),
            epoch: 0,
            live: Live::new(cx),
            broadcasts: Broadcasts::new(window, cx),
            presence: std::collections::HashMap::new(),
            presence_error: None,
            layout: PlayLayout::default(),
            shown: false,
            focus: cx.focus_handle(),
            _tick: None,
            _subscriptions: subscriptions,
        }
    }

    fn playing_own_game(cx: &App) -> bool {
        cx.global::<OnlineHandle>().0.read(cx).busy_with_game()
    }

    /// The page was shown: channels, presence and broadcasts afresh, and the feed again.
    pub fn shown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.shown = true;
        self.load_channels(cx);
        self.refresh_presence(cx);
        self.load_broadcasts(cx);
        self.resume(window, cx);
        self.start_ticking(cx);
        self.read_engine_status(cx);
    }

    /// The page was left: nothing downloads for a board no one sees.
    pub fn hidden(&mut self, cx: &mut Context<Self>) {
        self.shown = false;
        self._tick = None;
        self.stop(cx);
        self.engine_changed(cx);
    }

    fn start_ticking(&mut self, cx: &mut Context<Self>) {
        self._tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let live = this
                    .update(cx, |this, cx| {
                        if this.live.frame.is_some() || this.broadcasts.ticking() {
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !live {
                    return;
                }
            }
        }));
    }

    /// The stream on a lost connection ended with it: what is watched streams again.
    fn reconnected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The engine's search ended with the connection too.
        let request = self.broadcasts.engine.engine.wanted().cloned();
        self.broadcasts.engine.engine.restart(
            request,
            |page: &mut WatchPage| &mut page.broadcasts.engine.engine,
            cx,
        );
        if let (Some(target), Some(_)) = (self.target.clone(), self.session) {
            self.watch(target, cx);
        } else if self.round_session.is_some()
            && let Some(round) = self.broadcasts.resumable()
        {
            let game = self.broadcasts.selected.clone();
            self.open_round(round, game, window, cx);
        }
    }

    /// What the tab shows when it opens: TV's last channel, an open tournament's round.
    fn resume(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if Self::playing_own_game(cx) {
            return;
        }
        match self.tab {
            Tab::Tv if self.target.is_none() => {
                let channel = crate::prefs::get::<String>(cx, CHANNEL_KEY, "best".into());
                let known = self.channels.is_empty()
                    || self.channels.iter().any(|c| c["key"] == json!(channel));
                self.watch_channel(if known { channel } else { "best".into() }, cx);
            }
            Tab::Broadcasts => {
                if let Some(round) = self.broadcasts.resumable() {
                    let game = self.broadcasts.selected.clone();
                    self.open_round(round, game, window, cx);
                }
            }
            _ => {}
        }
    }

    fn choose_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        if tab == self.tab {
            return;
        }
        self.tab = tab;
        crate::prefs::set(cx, TAB_KEY, tab.key());
        self.stop(cx);
        self.resume(window, cx);
        self.engine_changed(cx);
        cx.notify();
    }

    /// Watch a live game (`watchGame`): the Following tab, streaming it.
    pub fn watch_game(&mut self, id: &str, cx: &mut Context<Self>) {
        self.tab = Tab::Friends;
        crate::prefs::set(cx, TAB_KEY, "friends");
        self.watch(Target::Game(id.to_string()), cx);
    }

    fn load_channels(&mut self, cx: &mut Context<Self>) {
        let channels = cx.global::<AppCore>().call("tvChannels", Vec::new());
        cx.spawn(async move |this, cx| {
            let answer = channels.await;
            let _ = this.update(cx, |this, cx| {
                match answer {
                    Ok(channels) => {
                        this.channels = channels.as_array().cloned().unwrap_or_default()
                    }
                    Err(cause) => this.error = Some(cause.message.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn watch_channel(&mut self, channel: String, cx: &mut Context<Self>) {
        crate::prefs::set(cx, CHANNEL_KEY, channel.clone());
        self.watch(Target::Channel(channel), cx);
    }

    /// `watch`: stream a channel or a game; its updates carry the session the core answers.
    fn watch(&mut self, target: Target, cx: &mut Context<Self>) {
        self.epoch += 1;
        let epoch = self.epoch;
        self.early.clear();
        self.session = None;
        self.round_session = None;
        self.phase = None;
        self.error = None;
        self.live.clear();
        self.target = Some(target.clone());
        self.pending = Some(Pending::Watch);
        cx.notify();
        let call = cx.global::<AppCore>().call("watch", vec![target.json()]);
        cx.spawn(async move |this, cx| {
            let answer = call.await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch {
                    return;
                }
                match answer {
                    Ok(session) => {
                        this.session = session.as_f64();
                        this.replay_early(cx);
                    }
                    Err(cause) => {
                        this.pending = None;
                        this.early.clear();
                        this.error = Some(cause.message.into());
                        this.target = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `stop`: no feed, no frame.
    fn stop(&mut self, cx: &mut Context<Self>) {
        self.epoch += 1;
        self.early.clear();
        self.pending = None;
        self.session = None;
        self.round_session = None;
        self.phase = None;
        self.target = None;
        self.live.clear();
        self.broadcasts.pause_clocks();
        let stopping = cx.global::<AppCore>().call("stopWatching", Vec::new());
        cx.background_spawn(async move {
            if let Err(cause) = stopping.await {
                crate::log::write(
                    kchess_core::Level::Warn,
                    "watch",
                    &format!("stopping watch failed: {}", cause.message),
                );
            }
        })
        .detach();
        cx.notify();
    }

    fn replay_early(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        for event in std::mem::take(&mut self.early) {
            self.receive(event, cx);
        }
    }

    /// A feed's update: a frame, the connection's state, or a broadcast round's games.
    fn receive(&mut self, event: CoreEvent, cx: &mut Context<Self>) {
        let round = event.name == "watch:broadcast";
        if self.pending
            == Some(if round {
                Pending::Round
            } else {
                Pending::Watch
            })
        {
            self.early.push(event);
            if self.early.len() > EARLY_KEPT {
                self.early.remove(0);
            }
            return;
        }
        let session = event.payload["session"].as_f64();
        match event.name.as_str() {
            "watch:frame" if session.is_some() && session == self.session => {
                self.phase = Some("connected".into());
                self.live.set_frame(event.payload, cx);
            }
            "watch:state" if session.is_some() && session == self.session => {
                let phase = event.payload["phase"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                if phase != "connected" && self.phase.as_deref() == Some("connected") {
                    self.live.freeze_clock();
                }
                if let Some(message) = event.payload["message"].as_str() {
                    self.error = Some(message.to_string().into());
                }
                self.phase = Some(phase);
            }
            "watch:broadcast" if session.is_some() && session == self.round_session => {
                if let Some(error) = self.broadcasts.update(&event.payload) {
                    self.error = Some(error.into());
                }
                self.engine_games_changed(cx);
            }
            _ => return,
        }
        cx.notify();
    }

    /// Who the friends tab lists: followed players' presence, fifty names a request.
    fn refresh_presence(&mut self, cx: &mut Context<Self>) {
        let names: Vec<String> = crate::accounts::handle(cx)
            .read(cx)
            .tracked()
            .iter()
            .filter_map(|friend| friend["username"].as_str().map(str::to_string))
            .collect();
        let core = cx.global::<AppCore>().clone();
        self.presence_error = None;
        for chunk in names.chunks(50) {
            let presence = core.call("presence", vec![json!(chunk)]);
            cx.spawn(async move |this, cx| {
                let answer = presence.await;
                let _ = this.update(cx, |this, cx| {
                    match answer {
                        Ok(report) => {
                            for (name, entry) in report["users"].as_object().into_iter().flatten() {
                                this.presence.insert(name.to_lowercase(), entry.clone());
                            }
                        }
                        Err(cause) => this.presence_error = Some(cause.message.into()),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn channel_list(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let disabled = Self::playing_own_game(cx);
        let current = match &self.target {
            Some(Target::Channel(channel)) => Some(channel.clone()),
            _ => None,
        };
        let header = h_flex()
            .justify_between()
            .px(px(8.))
            .child(widgets::heading("Channels", palette))
            .child(
                Button::new("watch-channels-refresh")
                    .ghost()
                    .xsmall()
                    .icon(icon("refresh-cw"))
                    .accessibility_label("Refresh channels")
                    .on_click(cx.listener(|this, _, _, cx| this.load_channels(cx))),
            );
        list_card(palette).child(header).child(
            v_flex()
                .id("watch-channel-list")
                .gap(px(2.))
                .overflow_y_scroll()
                .children(self.channels.iter().enumerate().map(|(index, channel)| {
                    let key = channel["key"].as_str().unwrap_or_default().to_string();
                    let chosen = current.as_deref() == Some(key.as_str());
                    let sub = if channel["player"].is_object() {
                        label(&channel["player"])
                    } else {
                        "No game".into()
                    };
                    h_flex()
                        .id(("watch-channel", index))
                        .role(Role::Button)
                        .keyboard_focus(palette.primary)
                        .aria_label(channel["label"].as_str().unwrap_or_default().to_string())
                        .aria_selected(chosen)
                        .gap(px(12.))
                        .p(px(8.))
                        .rounded(px(8.))
                        .when(chosen, |row| {
                            row.bg(crate::theme::tint(palette.primary, 14.))
                        })
                        .when(disabled, |row| row.opacity(0.6))
                        .when(!disabled, |row| {
                            let key = key.clone();
                            row.cursor_pointer()
                                .hover(|row| row.bg(palette.accented))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.watch_channel(key.clone(), cx)
                                }))
                        })
                        .child(
                            channel_icon(&key)
                                .size(px(20.))
                                .flex_none()
                                .text_color(if chosen {
                                    palette.primary
                                } else {
                                    palette.text_muted
                                }),
                        )
                        .child(
                            v_flex()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_size(px(14.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .truncate()
                                        .child(
                                            channel["label"]
                                                .as_str()
                                                .unwrap_or_default()
                                                .to_string(),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(palette.text_muted)
                                        .truncate()
                                        .child(sub),
                                ),
                        )
                })),
        )
    }

    fn friends_list(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let disabled = Self::playing_own_game(cx);
        let friends = crate::accounts::handle(cx).read(cx).tracked();
        let online = friends
            .iter()
            .filter(|friend| {
                friend["username"].as_str().is_some_and(|name| {
                    self.presence
                        .get(&name.to_lowercase())
                        .is_some_and(|p| p["online"] == json!(true))
                })
            })
            .count();
        let playing: Vec<(String, Option<String>)> = friends
            .iter()
            .filter_map(|friend| {
                let name = friend["username"].as_str()?;
                let presence = self.presence.get(&name.to_lowercase())?;
                (presence["playing"] == json!(true)).then(|| {
                    (
                        name.to_string(),
                        presence["playingId"].as_str().map(str::to_string),
                    )
                })
            })
            .collect();
        let header = h_flex()
            .justify_between()
            .items_start()
            .px(px(8.))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(widgets::heading("Followed players playing now", palette))
                    .child(widgets::hint(
                        if friends.is_empty() {
                            "Follow players on the Following page.".to_string()
                        } else {
                            format!("{online} of {} online", friends.len())
                        },
                        palette,
                    )),
            )
            .child(
                Button::new("watch-friends-refresh")
                    .ghost()
                    .xsmall()
                    .icon(icon("refresh-cw"))
                    .accessibility_label("Refresh followed players")
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_presence(cx))),
            );
        list_card(palette)
            .child(header)
            .children(self.presence_error.clone().map(|error| {
                div()
                    .px(px(8.))
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(format!("Could not refresh followed players: {error}"))
            }))
            .when(playing.is_empty(), |card| {
                card.child(div().px(px(8.)).child(widgets::hint(
                    "No followed player is playing right now.",
                    palette,
                )))
            })
            .children(
                playing
                    .into_iter()
                    .enumerate()
                    .map(|(index, (name, game))| {
                        h_flex()
                            .gap(px(10.))
                            .p(px(8.))
                            .child(icon("swords").size(px(16.)).text_color(palette.text_muted))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        crate::pages::players::player_link(
                                            ("watch-friend", index),
                                            name,
                                            palette.primary,
                                        )
                                        .text_size(px(14.))
                                        .font_weight(FontWeight::SEMIBOLD),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette.text_muted)
                                            .child("Playing now"),
                                    ),
                            )
                            .child(
                                Button::new(("watch-friend-game", index))
                                    .xsmall()
                                    .primary()
                                    .label("Watch")
                                    .disabled(game.is_none() || disabled)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(id) = game.clone() {
                                            this.watch(Target::Game(id), cx);
                                        }
                                    })),
                            )
                    }),
            )
    }

    fn notices(&self, cx: &mut Context<Self>, palette: &Palette) -> Vec<AnyElement> {
        let mut notices = Vec::new();
        if Self::playing_own_game(cx) {
            notices.push(
                widgets::StatusBanner::new(
                    widgets::Tone::Alert,
                    icon("swords"),
                    "You are in a game".into(),
                )
                .detail("Watching is paused while your game uses the connection.".into())
                .into_any_element(),
            );
        }
        let failed = matches!(self.phase.as_deref(), Some("error" | "ended"));
        if let (true, Some(target)) = (failed, self.target.clone()) {
            notices.push(
                h_flex()
                    .child(
                        Button::new("watch-retry")
                            .xsmall()
                            .outline()
                            .label("Retry feed")
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.watch(target.clone(), cx)),
                            ),
                    )
                    .into_any_element(),
            );
        }
        if let Some(error) = &self.error {
            notices.push(
                div()
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(error.clone())
                    .into_any_element(),
            );
        }
        notices
    }
}

/// The list beside the watched board (channels, followed players).
fn list_card(palette: &Palette) -> Div {
    widgets::card(palette)
        .px(px(12.))
        .py(px(16.))
        .gap(px(8.))
        .w(px(240.))
        .flex_none()
}

impl Render for WatchPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let tabs = [
            (Tab::Tv, "TV", icon("tv")),
            (Tab::Friends, "Following", icon("users")),
            (Tab::Broadcasts, "Broadcasts", icon("radio")),
        ];
        let selected = tabs
            .iter()
            .position(|(tab, _, _)| *tab == self.tab)
            .unwrap_or(0);
        let page = cx.entity().downgrade();
        let choose = move |index: usize, window: &mut Window, cx: &mut App| {
            let tab = [Tab::Tv, Tab::Friends, Tab::Broadcasts][index];
            let _ = page.update(cx, |page, cx| page.choose_tab(tab, window, cx));
        };
        let header = h_flex()
            .gap(px(12.))
            .child(
                div().flex_1().min_w_0().children(
                    (self.tab == Tab::Broadcasts)
                        .then(|| self.broadcasts_toolbar(cx, &palette))
                        .flatten(),
                ),
            )
            .child(widgets::header_pills(
                "watch-tabs",
                tabs.into_iter()
                    .map(|(_, label, glyph)| Pill {
                        icon: Some(glyph),
                        label: label.into(),
                        badge: None,
                        disabled: false,
                    })
                    .collect(),
                selected,
                choose,
                &palette,
            ));
        let notices = self.notices(cx, &palette);
        let body = match self.tab {
            Tab::Tv => {
                let list = self.channel_list(cx, &palette);
                self.live_layout(list, window, cx, &palette)
            }
            Tab::Friends => {
                let list = self.friends_list(cx, &palette);
                self.live_layout(list, window, cx, &palette)
            }
            Tab::Broadcasts => self.broadcasts_body(window, cx, &palette),
        };
        div()
            .id("watch-page")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .size_full()
            .relative()
            .overflow_y_scroll()
            .child(self.layout_probe(cx))
            .child(
                v_flex()
                    .gap(px(16.))
                    .px(side_padding(window))
                    .pt(PAGE_TOP)
                    .pb(PAGE_BOTTOM)
                    .child(header)
                    .children(notices)
                    .child(body)
                    .when(self.tab == Tab::Tv, |page| {
                        page.children(self.live.past_games(cx, &palette))
                    }),
            )
    }
}

impl WatchPage {
    fn layout_probe(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        self.layout.probe(cx)
    }

    /// ←/→ and ↑/↓ step through the shown game; F turns the board.
    fn key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        let key = event.keystroke.key.as_str();
        let handled = match self.tab {
            Tab::Broadcasts if key == "l" => self.toggle_broadcast_engine(cx),
            Tab::Broadcasts => {
                let handled = self.broadcasts.key(key, cx);
                if handled {
                    self.engine_changed(cx);
                }
                handled
            }
            _ => self.live.key(key, cx),
        };
        if handled {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn load_broadcasts(&mut self, cx: &mut Context<Self>) {
        self.broadcasts.load(cx);
    }
}

/// The page's badge for a live game.
fn live_badge(palette: &Palette) -> Div {
    widgets::badge("Live", Look::Error, palette)
        .gap(px(4.))
        .child(icon("radio").size(px(12.)))
}
