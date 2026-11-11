//! Play on Lichess: find a game (a seek, a challenge, correspondence), the
//! challenges and games in progress beside it, and the live game on the board with its offers,
//! actions and chat. The game itself is `crate::online`'s, shared by the whole app.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use crate::assets::icon;
use crate::board::{BoardEvent, ChessBoard};
use crate::export::Exporter;
use crate::game::rules;
use crate::online::{OnlineHandle, OnlineState, Phase};
use crate::pages::Page;
use crate::play_layout::{PAGE_BOTTOM, PAGE_TOP, PlayLayout, side_padding};
use crate::theme::{Palette, radii};
use crate::widgets::Pressable as _;
use crate::widgets::{self, Choice, KeyboardFocus as _, Pill};

mod chat;
pub(crate) mod game;
pub(crate) mod lists;

/// The time controls offered (`timeControls`).
const TIME_CONTROLS: [(f64, &str); 6] = [
    (3., "Blitz"),
    (5., "Blitz"),
    (10., "Rapid"),
    (15., "Rapid"),
    (30., "Classical"),
    (60., "Classical"),
];
const INCREMENTS: [f64; 7] = [0., 1., 2., 3., 5., 10., 20.];
/// `CORRESPONDENCE_DAYS`.
const DAYS: [f64; 7] = [1., 2., 3., 5., 7., 10., 14.];

/// `data/variants.json`: each variant's key, name and what sets it apart.
fn time_rule(name: &str, minutes: f64, increment: f64, extra: Option<bool>) -> Value {
    let mut args = vec![json!(minutes), json!(increment)];
    args.extend(extra.map(|value| json!(value)));
    rules(name, &args).unwrap_or(Value::Null)
}

/// "5+3 · Rapid · Casual", or "3 days per move · Casual" (`summary`).
pub fn summary(form: &crate::online::Form, variant_label: Option<&str>) -> String {
    let variant = variant_label
        .filter(|_| form.variant != "standard")
        .map(|label| format!("{label} · "))
        .unwrap_or_default();
    let mode = if form.rated { "Rated" } else { "Casual" };
    if form.correspondence {
        format!(
            "{variant}{} {} per move · {mode}",
            form.days,
            if form.days == 1. { "day" } else { "days" }
        )
    } else {
        let perf = time_rule("perfFor", form.minutes, form.increment, None);
        format!(
            "{variant}{}+{} · {} · {mode}",
            form.minutes,
            form.increment,
            perf.as_str().unwrap_or_default()
        )
    }
}

pub struct OnlinePage {
    board: Entity<ChessBoard>,
    target: Entity<InputState>,
    account_select: Entity<SelectState<Vec<Choice>>>,
    variant_select: Entity<SelectState<Vec<Choice>>>,
    chat_input: Entity<InputState>,
    /// Lines of the chat read when it was last open; a new game starts again.
    chat_read: usize,
    chat_game: String,
    exporter: Entity<Exporter>,
    previews: std::collections::HashMap<String, Entity<ChessBoard>>,
    /// The game's panel beside the board, drawn as a cached view of its own
    /// (`render_game_panel`), so a sliding move redraws the page around the board alone.
    game_panel: Entity<crate::part::Part<OnlinePage>>,
    layout: PlayLayout,
    focus: FocusHandle,
    /// The mouse wheel over the board, which steps through the moves.
    wheel: crate::game::view::Wheel,
    /// A correspondence move on the board, waiting for Confirm (`confirmCorrespondence`).
    pending: Option<Pending>,
    _subscriptions: Vec<Subscription>,
}

/// A move made on the board and not sent yet: in `game`, after `after` moves.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Pending {
    game: String,
    after: usize,
    pub uci: String,
}

impl Focusable for OnlinePage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl OnlinePage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> OnlinePage {
        let online = cx.global::<OnlineHandle>().0.clone();
        let accounts = crate::accounts::handle(cx);
        let board = cx.new(|_| {
            ChessBoard::new(crate::board::BoardState::shown(
                "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w",
                "standard",
                shakmaty::Color::White,
                None,
            ))
        });
        let form = online.read(cx).form.clone();
        let target = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Lichess username")
                .default_value(form.target.clone())
        });
        let variant_select = cx.new(|cx| {
            SelectState::new(
                crate::game::variants()
                    .iter()
                    .map(|text| Choice::new(text.key.clone(), text.label.clone()))
                    .collect(),
                None,
                window,
                cx,
            )
        });
        variant_select.update(cx, |select, cx| {
            select.set_selected_value(&SharedString::from(form.variant.clone()), window, cx)
        });
        let account_select = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let chat_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Message your opponent"));
        let settings = cx.global::<crate::settings::SettingsHandle>().0.clone();
        let subscriptions = vec![
            // Blindfold and premoves change the board.
            cx.subscribe(
                &settings,
                |this, _, _: &crate::settings::SettingsChanged, cx| this.redraw(cx),
            ),
            cx.observe(&online, |this, _, cx| {
                this.redraw(cx);
                cx.notify();
            }),
            cx.observe_in(&accounts, window, |this, _, window, cx| {
                this.sync_accounts(window, cx);
            }),
            cx.subscribe(&board, |this, _, event: &BoardEvent, cx| {
                if let BoardEvent::Move(uci) = event {
                    this.board_move(uci.clone(), cx);
                }
            }),
            cx.subscribe(&target, |_, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    let text = input.read(cx).value().to_string();
                    crate::online::handle(cx).update(cx, |online, cx| {
                        online.form.target = text;
                        cx.notify();
                    });
                }
            }),
            cx.subscribe(
                &variant_select,
                |_, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(variant)) = event {
                        let variant = variant.to_string();
                        crate::online::handle(cx).update(cx, |online, cx| {
                            online.form.variant = variant;
                            cx.notify();
                        });
                    }
                },
            ),
            cx.subscribe(
                &account_select,
                |_, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(account)) = event {
                        crate::online::set_active_account(account, cx);
                        crate::accounts::handle(cx).update(cx, |_, cx| cx.notify());
                        crate::online::handle(cx)
                            .update(cx, |online, cx| online.stay_connected(cx));
                    }
                },
            ),
            cx.subscribe_in(
                &chat_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.send_chat(None, window, cx);
                    }
                },
            ),
        ];
        let mut page = OnlinePage {
            board,
            target,
            account_select,
            variant_select,
            chat_input,
            chat_read: 0,
            chat_game: String::new(),
            exporter: cx.new(|_| Exporter::default()),
            previews: std::collections::HashMap::new(),
            game_panel: crate::part::Part::panel(
                |this, window, cx| this.render_game_panel(window, cx),
                cx,
            ),
            layout: PlayLayout::default(),
            focus: cx.focus_handle(),
            wheel: Default::default(),
            pending: None,
            _subscriptions: subscriptions,
        };
        page.sync_accounts(window, cx);
        page.redraw(cx);
        page
    }

    /// The page was shown: the games in progress afresh.
    pub fn shown(&mut self, cx: &mut Context<Self>) {
        crate::online::handle(cx).update(cx, |online, cx| online.refresh_ongoing(cx));
    }

    /// Challenge a player casually.
    pub fn challenge(&mut self, username: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.challenge_rated(username, false, window, cx);
    }

    /// Challenge a player, rated or casual.
    pub fn challenge_rated(
        &mut self,
        username: &str,
        rated: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::online::handle(cx).update(cx, |online, cx| {
            online.form.rated = rated;
            cx.notify();
        });
        self.challenge_keeping_rated(username, window, cx);
    }

    /// Send the form's game, rated or not as it is set, to `username` (`challengeFriend`).
    pub fn challenge_keeping_rated(
        &mut self,
        username: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let online = crate::online::handle(cx);
        if online.read(cx).busy_with_game() {
            crate::online::notice(cx, "Finish or cancel your game before starting another");
            return;
        }
        let username = username.to_string();
        online.update(cx, |online, cx| {
            let form = &mut online.form;
            let fits =
                time_rule("canPlayOnline", form.minutes, form.increment, Some(true)) == json!(true);
            if !fits {
                form.minutes = 10.;
                form.increment = 0.;
            }
            form.correspondence = false;
            form.color = "random".into();
            form.target = username.clone();
            cx.notify();
        });
        self.target
            .update(cx, |input, cx| input.set_value(username, window, cx));
        // The challenge goes out at once, as `challengeFriend` sends it.
        online.update(cx, |online, cx| online.start(None, cx));
    }

    fn sync_accounts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let accounts = crate::accounts::handle(cx).read(cx);
        let connected = accounts.connected();
        let active = accounts.active(cx).unwrap_or_default();
        let items: Vec<Choice> = connected
            .iter()
            .map(|name| Choice::new(name.clone(), format!("@{name}")))
            .collect();
        self.account_select.update(cx, |select, cx| {
            select.set_items(items, window, cx);
            select.set_selected_value(&SharedString::from(active), window, cx);
        });
        cx.notify();
    }

    /// A move made on the board: sent, or in a correspondence game held for Confirm when
    /// `confirmCorrespondence` is on, since a sent move there cannot be taken back.
    fn board_move(&mut self, uci: String, cx: &mut Context<Self>) {
        let confirm = cx
            .global::<crate::settings::SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .confirm_correspondence;
        let online = crate::online::handle(cx);
        let held = {
            let online = online.read(cx);
            (confirm && online.correspondence()).then(|| Pending {
                game: online.id().to_string(),
                after: online.moves().len(),
                uci: uci.clone(),
            })
        };
        if held.is_some() {
            self.pending = held;
            self.redraw(cx);
            cx.notify();
        } else {
            online.update(cx, |online, cx| online.play(uci, cx));
        }
    }

    /// Send the move waiting for Confirm.
    pub(super) fn confirm_move(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        crate::online::handle(cx).update(cx, |online, cx| online.play(pending.uci, cx));
        self.redraw(cx);
        cx.notify();
    }

    /// Take the move waiting for Confirm back off the board.
    pub(super) fn cancel_move(&mut self, cx: &mut Context<Self>) {
        if self.pending.take().is_some() {
            self.redraw(cx);
            cx.notify();
        }
    }

    /// The move waiting for Confirm, while it still fits the game on the board.
    pub(super) fn pending(&self) -> Option<&Pending> {
        self.pending.as_ref()
    }

    /// The board as the game stands, or at the move looked at.
    fn redraw(&mut self, cx: &mut Context<Self>) {
        let (state, id) = {
            let online = cx.global::<OnlineHandle>().0.read(cx);
            if !matches!(
                online.phase,
                Phase::Playing | Phase::Finished | Phase::Disconnected
            ) {
                self.pending = None;
                return;
            }
            let setup = online.setup();
            let mut moves = online.moves();
            let mut ply = online.ply();
            let can_play = game::can_play(online);
            // A held move lasts while the game is where it was made: a move from Lichess, another
            // game, the game's end or looking back at earlier moves drops it.
            if self.pending.as_ref().is_some_and(|pending| {
                online.phase != Phase::Playing
                    || !can_play
                    || pending.game != online.id()
                    || pending.after != moves.len()
                    || ply != moves.len()
            }) {
                self.pending = None;
            }
            let turn = crate::game::view::turn(
                crate::game::view::position_at(&setup, &moves, moves.len()).as_ref(),
            );
            let mut movable = (can_play && turn == online.color()).then_some(online.color());
            if let Some(pending) = &self.pending {
                moves.push(pending.uci.clone());
                ply = moves.len();
                movable = None;
            }
            let mut state =
                crate::game::view::board_state(&setup, &moves, ply, online.orientation(), movable);
            let frontend = &cx
                .global::<crate::settings::SettingsHandle>()
                .0
                .read(cx)
                .settings
                .frontend;
            // Blindfold hides the pieces while the game is on (`settings.blindfold && playing`).
            state.blindfold = frontend.blindfold && online.phase == Phase::Playing;
            // On the opponent's turn a move can be queued (`premove && !correspondence`).
            let premove = frontend.premove;
            if premove
                && can_play
                && movable.is_none()
                && self.pending.is_none()
                && !online.correspondence()
            {
                state.premove = Some(online.color());
            }
            (state, online.id().to_string())
        };
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        if id != self.chat_game {
            self.chat_game = id;
            self.chat_read = 0;
        }
    }

    fn send_chat(&mut self, preset: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let typed = preset.is_none();
        let text = preset
            .map(str::to_string)
            .unwrap_or_else(|| self.chat_input.read(cx).value().to_string());
        if text.trim().is_empty() {
            return;
        }
        let sent =
            crate::online::handle(cx).update(cx, |online, cx| online.send_chat(text, "player", cx));
        let input = self.chat_input.clone();
        cx.spawn_in(window, async move |_, cx| {
            if sent.await && typed {
                let _ = cx.update(|window, cx| {
                    input.update(cx, |input, cx| input.set_value("", window, cx))
                });
            }
        })
        .detach();
    }

    /// The find-a-game form (`.online-form`).
    fn form(&self, online: &OnlineState, cx: &App, palette: &Palette) -> Div {
        let form = online.form.clone();
        let connected = crate::accounts::handle(cx).read(cx).connected();
        let active = crate::accounts::handle(cx)
            .read(cx)
            .active(cx)
            .unwrap_or_default();
        let targeted = !form.target.trim().is_empty();
        let allowed = form.correspondence
            || time_rule(
                "canPlayOnline",
                form.minutes,
                form.increment,
                Some(targeted),
            ) == json!(true);
        let blitz_needs_target = !targeted
            && time_rule("canPlayOnline", form.minutes, form.increment, Some(false)) != json!(true)
            && time_rule("canDirectChallenge", form.minutes, form.increment, None) == json!(true);
        let perf = time_rule("perfFor", form.minutes, form.increment, None)
            .as_str()
            .unwrap_or_default()
            .to_string();
        let (variant_label, variant_hint) = crate::game::variants()
            .iter()
            .find(|text| text.key == form.variant)
            .map(|text| (text.label.clone(), text.hint.clone()))
            .unwrap_or_default();
        let estimated = time_rule("totalSeconds", form.minutes, form.increment, None)
            .as_f64()
            .unwrap_or_default()
            / 60.;
        let set = |change: fn(&mut crate::online::Form, f64), value: f64| {
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                crate::online::handle(cx).update(cx, |online, cx| {
                    change(&mut online.form, value);
                    cx.notify();
                });
            }
        };
        let pills = |id: &'static str,
                     labels: Vec<(Option<gpui_kit::component::Icon>, String)>,
                     selected: usize,
                     choose: fn(&mut crate::online::Form, usize)| {
            widgets::pills(
                id,
                labels
                    .into_iter()
                    .map(|(glyph, label)| Pill {
                        icon: glyph,
                        label: label.into(),
                        badge: None,
                        disabled: false,
                    })
                    .collect(),
                selected,
                move |index, _, cx| {
                    crate::online::handle(cx).update(cx, |online, cx| {
                        choose(&mut online.form, index);
                        cx.notify();
                    })
                },
                false,
                palette,
            )
        };
        let field = |label: &str, hint: Option<String>, control: AnyElement| {
            v_flex()
                .gap(px(8.))
                .child(
                    h_flex()
                        .gap(px(4.))
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(label.to_string())
                        .children(hint.map(|hint| {
                            div()
                                .font_weight(FontWeight::NORMAL)
                                .text_color(palette.text_muted)
                                .child(format!("— {hint}"))
                        })),
                )
                .child(control)
        };
        // `.tc-grid`: the six presets in rows of three (the form's card is never under 340px).
        let columns = 3;
        let cells: Vec<AnyElement> = TIME_CONTROLS
            .iter()
            .enumerate()
            .map(|(index, (minutes, label))| {
                let chosen = form.minutes == *minutes;
                // `.tc-option`.
                v_flex()
                    .id(("time-control", index))
                    .role(Role::Button)
                    .keyboard_focus(palette.primary)
                    .aria_label(label.to_string())
                    .aria_selected(chosen)
                    .flex_1()
                    .min_w_0()
                    .items_start()
                    .gap(px(2.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(radii::LG)
                    .border_1()
                    .border_color(if chosen {
                        palette.primary
                    } else {
                        palette.border
                    })
                    .bg(if chosen {
                        crate::theme::tint(palette.primary, 12.)
                    } else {
                        palette.bg
                    })
                    .text_color(palette.text)
                    .cursor_pointer()
                    .when(!chosen, |cell| cell.pressable_tile(palette))
                    .on_click(set(|form, value| form.minutes = value, *minutes))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(format!("{minutes} min")),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(palette.text_muted)
                            .child(*label),
                    )
                    .into_any_element()
            })
            .collect();
        let mut rows = Vec::new();
        let mut cells = cells.into_iter().peekable();
        while cells.peek().is_some() {
            rows.push(h_flex().gap(px(8.)).children(cells.by_ref().take(columns)));
        }
        let time_grid = v_flex().gap(px(8.)).children(rows);
        let mut card = widgets::card(palette)
            .gap(px(16.))
            .flex_1()
            .min_w(px(340.))
            .child(widgets::heading("Find a game", palette));
        if connected.len() > 1 {
            card = card.child(field(
                "Account",
                None,
                Select::new(&self.account_select)
                    .accessibility_label("Account")
                    .into_any_element(),
            ));
        }
        card = card
            .child(field(
                "Kind of game",
                None,
                pills(
                    "online-mode",
                    vec![
                        (Some(icon("timer")), "Real time".into()),
                        (Some(icon("mail")), "Correspondence".into()),
                    ],
                    usize::from(form.correspondence),
                    |form, index| form.correspondence = index == 1,
                )
                .into_any_element(),
            ))
            .child(field(
                "Variant",
                Some(variant_hint),
                Select::new(&self.variant_select)
                    .accessibility_label("Variant")
                    .into_any_element(),
            ));
        if form.correspondence {
            card = card.child(field(
                "Days per move",
                None,
                pills(
                    "online-days",
                    DAYS.iter()
                        .map(|days| {
                            (
                                None,
                                format!("{days} {}", if *days == 1. { "day" } else { "days" }),
                            )
                        })
                        .collect(),
                    DAYS.iter().position(|days| *days == form.days).unwrap_or(2),
                    |form, index| form.days = DAYS[index],
                )
                .into_any_element(),
            ));
        } else {
            card = card
                .child(field("Time control", None, time_grid.into_any_element()))
                .child(field(
                    "Increment per move",
                    None,
                    pills(
                        "online-increment",
                        INCREMENTS.iter().map(|s| (None, format!("{s}s"))).collect(),
                        INCREMENTS
                            .iter()
                            .position(|s| *s == form.increment)
                            .unwrap_or(0),
                        |form, index| form.increment = INCREMENTS[index],
                    )
                    .into_any_element(),
                ));
        }
        card = card
            .child(field(
                "Play as",
                None,
                pills(
                    "online-color",
                    vec![
                        (Some(icon("shuffle")), "Random".into()),
                        (Some(icon("circle")), "White".into()),
                        (Some(icon("circle-dot")), "Black".into()),
                    ],
                    match form.color.as_str() {
                        "white" => 1,
                        "black" => 2,
                        _ => 0,
                    },
                    |form, index| form.color = ["random", "white", "black"][index].into(),
                )
                .into_any_element(),
            ))
            .child(field(
                "Game type",
                None,
                pills(
                    "online-rated",
                    vec![
                        (Some(icon("coffee")), "Casual".into()),
                        (Some(icon("trophy")), "Rated".into()),
                    ],
                    usize::from(form.rated),
                    |form, index| form.rated = index == 1,
                )
                .into_any_element(),
            ))
            .child(field(
                "Challenge a specific player",
                Some("optional, unlocks Blitz".into()),
                Input::new(&self.target)
                    .prefix(icon("user-search").size(px(14.)))
                    .into_any_element(),
            ));
        if !allowed {
            card = card.child(widgets::StatusBanner::new(widgets::Tone::Alert, icon("triangle-alert"), format!(
                    "Lichess blocks {perf} ({}+{}) here",
                    form.minutes, form.increment
                )
                .into()).detail(if blitz_needs_target {
                        "Add a username above to send it as a direct challenge, or pick a longer control"
                    } else {
                        "Pick Blitz or slower for a direct challenge, Rapid or slower for the public pool"
                    }
                    .into(),));
        } else if form.correspondence && !targeted {
            card = card.child(widgets::StatusBanner::new(widgets::Tone::Plain, icon("info"), "A public correspondence seek stays open on Lichess".into()).detail("Lichess has no way for apps to withdraw it; it closes when someone joins. Add a username to send a challenge you can withdraw instead.".into()));
        }
        let downloaded = crate::accounts::handle(cx).read(cx).downloaded(&active);
        let summary_line = summary(&form, Some(variant_label.as_str()));
        card.child(
            h_flex()
                .justify_between()
                .gap(px(12.))
                .pt(px(16.))
                .border_t_1()
                .border_color(palette.border)
                .child(
                    v_flex()
                        .text_size(px(13.))
                        .child(div().font_weight(FontWeight::MEDIUM).child(summary_line))
                        .when(!form.correspondence, |column| {
                            column.child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(palette.text_muted)
                                    .child(format!("~{estimated:.1} min estimated")),
                            )
                        })
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(format!(
                                    "Data used so far by @{active}: {}",
                                    widgets::format_bytes(downloaded)
                                )),
                        ),
                )
                .child(
                    Button::new("online-start")
                        .primary()
                        .large()
                        .icon(if targeted {
                            icon("send")
                        } else {
                            icon("search")
                        })
                        .label(if targeted {
                            "Send challenge"
                        } else if form.correspondence {
                            "Create seek"
                        } else {
                            "Find opponent"
                        })
                        .loading(online.busy)
                        .disabled(!allowed || online.busy)
                        .on_click(|_, _, cx| {
                            crate::online::handle(cx)
                                .update(cx, |online, cx| online.start(None, cx))
                        }),
                ),
        )
    }

    fn seeking(&self, online: &OnlineState, palette: &Palette) -> Div {
        let status = online.model()["onlineStatus"]
            .as_str()
            .filter(|status| !status.is_empty())
            .unwrap_or("Looking for an opponent…")
            .to_string();
        let label = crate::game::variants()
            .iter()
            .find(|text| text.key == online.form.variant)
            .map(|text| text.label.clone());
        h_flex().flex_1().items_center().justify_center().child(
            widgets::card(palette)
                .w(px(420.))
                .items_center()
                .gap(px(12.))
                .child(widgets::spinner(px(28.)).color(palette.primary))
                .child(widgets::heading(status, palette))
                .child(widgets::hint(
                    summary(&online.form, label.as_deref()),
                    palette,
                ))
                .child(
                    Button::new("online-cancel")
                        .outline()
                        .icon(icon("x"))
                        .label("Cancel")
                        .on_click(|_, _, cx| {
                            crate::online::handle(cx).update(cx, |online, cx| online.stop(cx))
                        }),
                ),
        )
    }
}

impl Render for OnlinePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let online_entity = cx.global::<OnlineHandle>().0.clone();
        let online = online_entity.read(cx);
        let connected = !crate::accounts::handle(cx).read(cx).connected().is_empty();
        if matches!(
            online.phase,
            Phase::Playing | Phase::Finished | Phase::Disconnected
        ) && connected
        {
            return self.game_view(window, cx).into_any_element();
        }
        let body: AnyElement = if !connected {
            widgets::card(&palette)
                .child(
                    widgets::empty_state(icon("globe"), "Connect Lichess to play", None, &palette)
                        .child(
                            h_flex()
                                .justify_center()
                                .gap(px(8.))
                                .mt(px(8.))
                                .child(
                                    Button::new("online-connect")
                                        .primary()
                                        .icon(icon("log-in"))
                                        .label("Connect Lichess")
                                        .on_click(|_, window, cx| {
                                            crate::accounts::handle(cx)
                                                .update(cx, |accounts, cx| {
                                                    accounts.connect(window, cx)
                                                })
                                        }),
                                )
                                .child(
                                    Button::new("online-computer")
                                        .outline()
                                        .label("Play the computer")
                                        .on_click(|_, window, cx| {
                                            crate::shell::open(Page::Computer, window, cx)
                                        }),
                                ),
                        ),
                )
                .into_any_element()
        } else if online.phase == Phase::Seeking {
            self.seeking(online, &palette).into_any_element()
        } else {
            let form = self.form(online, cx, &palette);
            let lists = self.lists(cx, &palette);
            h_flex()
                .items_start()
                .flex_wrap()
                .gap(px(20.))
                .child(form)
                .child(lists)
                .into_any_element()
        };
        div()
            .id("online-page")
            .test_support()
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .min_h_full()
                    .gap(px(16.))
                    .px(side_padding(window))
                    .pt(PAGE_TOP)
                    .pb(PAGE_BOTTOM)
                    .child(body),
            )
            .into_any_element()
    }
}

impl crate::pages::PageAction for game::LiveAction {
    fn info(&self) -> crate::pages::ActionInfo {
        use crate::pages::{ActionInfo, ActionRole};
        use game::LiveAction;
        let (id, label, glyph, keywords) = match self {
            LiveAction::Berserk => ("berserk", "Berserk", "flame", "arena half time"),
            LiveAction::Takeback => ("takeback", "Takeback", "undo-2", "undo take back"),
            LiveAction::OfferDraw => ("draw", "Offer draw", "handshake", "agree"),
            LiveAction::Resign => ("resign", "Resign", "flag", "give up"),
            LiveAction::Abort => ("abort", "Abort", "ban", "cancel"),
            LiveAction::Rematch => ("rematch", "Rematch", "repeat", "again"),
            LiveAction::NewOpponent => (
                "new-opponent",
                "New opponent",
                "users",
                "seek again same time control",
            ),
            LiveAction::Analyse => ("analyse", "Analyse", "microscope", "analysis review"),
            LiveAction::NewGame => ("new-game", "New game", "plus", "another seek"),
            LiveAction::CancelSeek => ("cancel-seek", "Cancel seek", "x", "stop waiting"),
            LiveAction::Flip => ("flip", "Flip board", "arrow-up-down", "turn rotate"),
        };
        let info = ActionInfo::new(format!("online:{id}"), label, glyph, keywords);
        match self {
            LiveAction::Flip => info.role(ActionRole::Flip),
            LiveAction::NewGame => info.role(ActionRole::New),
            _ => info,
        }
    }
}

/// The page shows a game: one being played, or just over, with an account to play it.
fn shows_game(cx: &App) -> bool {
    let online = crate::online::handle(cx).read(cx);
    matches!(
        online.phase,
        Phase::Playing | Phase::Finished | Phase::Disconnected
    ) && !crate::accounts::handle(cx).read(cx).connected().is_empty()
}

impl crate::pages::PageView for OnlinePage {
    const INFO: crate::pages::PageInfo = crate::pages::PageInfo {
        page: Page::Online,
        label: "Play on Lichess",
        short: "Play",
        icon: "globe-2",
        group: Some(crate::pages::NavGroup::Lichess),
        keywords: "lichess seek pairing",
    };

    type Action = game::LiveAction;

    fn create(window: &mut Window, cx: &mut Context<Self>) -> Self {
        OnlinePage::new(window, cx)
    }

    fn shown(this: &Entity<Self>, again: bool, _window: &mut Window, cx: &mut App) {
        if !again {
            this.update(cx, |online, cx| online.shown(cx));
        }
    }

    /// Zen mode leaves the game and its panel, as on the other game pages.
    fn zen(&self, cx: &App) -> bool {
        shows_game(cx)
    }

    /// The live game's actions, as its buttons offer them.
    fn actions(&self, cx: &App) -> Vec<game::LiveAction> {
        game::live_actions(crate::online::handle(cx).read(cx))
    }

    fn act(_this: &Entity<Self>, action: game::LiveAction, window: &mut Window, cx: &mut App) {
        game::run_live_action(action, window, cx);
    }
}

/// A live Lichess game of the account's on Play on Lichess.
pub fn open_game(id: &str, window: &mut Window, cx: &mut App) {
    let online = crate::online::handle(cx);
    let (shown, account) = {
        let online = online.read(cx);
        (
            online.id() == id && online.phase == Phase::Playing,
            crate::accounts::handle(cx)
                .read(cx)
                .active(cx)
                .unwrap_or_default(),
        )
    };
    // The game already on the board needs no new stream.
    if !shown {
        let id = id.to_string();
        online.update(cx, |online, cx| online.open(&account, &id, cx));
    }
    crate::shell::open(Page::Online, window, cx);
}

/// Look for a real-time game on Lichess, from Quick Pick.
pub fn play(minutes: f64, increment: f64, rated: bool, window: &mut Window, cx: &mut App) {
    crate::shell::open(Page::Online, window, cx);
    let online = crate::online::handle(cx);
    if online.read(cx).busy_with_game() {
        crate::online::notice(cx, "Finish or cancel your game before starting another");
        return;
    }
    online.update(cx, |online, cx| online.seek(minutes, increment, rated, cx));
}

/// Challenge a player to a casual game on Lichess.
pub fn challenge(username: &str, window: &mut Window, cx: &mut App) {
    let username = username.to_string();
    crate::shell::show::<OnlinePage>(window, cx, move |page, window, cx| {
        page.challenge(&username, window, cx)
    });
}

/// Challenge a player, rated or casual.
pub fn challenge_rated(username: &str, rated: bool, window: &mut Window, cx: &mut App) {
    let username = username.to_string();
    crate::shell::show::<OnlinePage>(window, cx, move |page, window, cx| {
        page.challenge_rated(&username, rated, window, cx)
    });
}

#[cfg(test)]
mod tests {
    use super::summary;
    use crate::online::Form;

    #[test]
    fn the_summary_names_the_game() {
        let mut form = Form::default();
        assert_eq!(summary(&form, Some("Standard")), "15+10 · Rapid · Casual");
        form.correspondence = true;
        form.rated = true;
        form.days = 1.;
        form.variant = "atomic".into();
        assert_eq!(
            summary(&form, Some("Atomic")),
            "Atomic · 1 day per move · Rated"
        );
    }
}
