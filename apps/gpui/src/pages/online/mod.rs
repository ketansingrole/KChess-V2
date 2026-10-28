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
use crate::theme::Palette;
use crate::widgets::{self, Choice, KeyboardFocus as _, Pill};

mod chat;
mod game;
mod lists;

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
fn variants() -> Vec<(String, String, String)> {
    let text = include_str!("../../../../../crates/kchess-domain/data/variants.json");
    serde_json::from_str::<Vec<Value>>(text)
        .unwrap_or_default()
        .into_iter()
        .map(|entry| {
            let field = |key: &str| entry[key].as_str().unwrap_or_default().to_string();
            (field("key"), field("label"), field("hint"))
        })
        .collect()
}

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
    layout: PlayLayout,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
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
                variants()
                    .into_iter()
                    .map(|(key, label, _)| Choice::new(key, label))
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
            cx.subscribe(&board, |_, _, event: &BoardEvent, cx| {
                if let BoardEvent::Move(uci) = event {
                    let uci = uci.clone();
                    crate::online::handle(cx).update(cx, |online, cx| online.play(uci, cx));
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
            layout: PlayLayout::default(),
            focus: cx.focus_handle(),
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

    /// Challenge a player casually (`challengeFriend`).
    pub fn challenge(&mut self, username: &str, window: &mut Window, cx: &mut Context<Self>) {
        crate::online::handle(cx).update(cx, |online, cx| {
            online.form.rated = false;
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
            crate::online::notice(cx, "Finish or cancel your game before starting another.");
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

    /// The board as the game stands, or at the move looked at.
    fn redraw(&mut self, cx: &mut Context<Self>) {
        let (state, id) = {
            let online = cx.global::<OnlineHandle>().0.read(cx);
            if !matches!(
                online.phase,
                Phase::Playing | Phase::Finished | Phase::Disconnected
            ) {
                return;
            }
            let setup = online.setup();
            let moves = online.moves();
            let ply = online.ply();
            let can_play = game::can_play(online);
            let turn = crate::game::view::turn(
                crate::game::view::position_at(&setup, &moves, moves.len()).as_ref(),
            );
            let movable = (can_play && turn == online.color()).then_some(online.color());
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
            if premove && can_play && movable.is_none() && !online.correspondence() {
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
        let all = variants();
        let (variant_label, variant_hint) = all
            .iter()
            .find(|(key, _, _)| *key == form.variant)
            .map(|(_, label, hint)| (label.clone(), hint.clone()))
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
                .gap(px(6.))
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
                    .py(px(10.))
                    .rounded(px(10.))
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
                    .when(!chosen, |cell| {
                        cell.hover(|cell| {
                            cell.border_color(palette.border_accented)
                                .bg(palette.accented)
                        })
                    })
                    .on_click(set(|form, value| form.minutes = value, *minutes))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::BOLD)
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
                Select::new(&self.account_select).into_any_element(),
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
                Select::new(&self.variant_select).into_any_element(),
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
                        "Add a username above to send it as a direct challenge, or pick a longer control."
                    } else {
                        "Pick Blitz or slower for a direct challenge, Rapid or slower for the public pool."
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
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(summary_line))
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
        let label = variants()
            .into_iter()
            .find(|(key, _, _)| *key == online.form.variant)
            .map(|(_, label, _)| label);
        h_flex().justify_center().pt(px(40.)).child(
            widgets::card(palette)
                .w(px(420.))
                .items_center()
                .gap(px(12.))
                .child(
                    icon("loader-circle")
                        .size(px(28.))
                        .text_color(palette.primary),
                )
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
                                .mt(px(10.))
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
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .gap(px(16.))
                    .px(side_padding(window))
                    .pt(PAGE_TOP)
                    .pb(PAGE_BOTTOM)
                    .child(body),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{summary, variants};
    use crate::online::Form;

    #[test]
    fn every_variant_has_a_name_and_a_hint() {
        let all = variants();
        assert!(
            all.iter()
                .any(|(key, label, _)| key == "standard" && label == "Standard")
        );
        assert!(
            all.iter()
                .all(|(_, label, hint)| !label.is_empty() && !hint.is_empty())
        );
    }

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
