//! Download a player's games: the dialog that asks whose games, from Lichess or Chess.com, and
//! which of them (the newest months, some speeds, rated only). Under the name it offers players:
//! on Lichess the ones whose names start with what is typed; on both sites your accounts and the
//! players downloaded or looked up lately. The download then runs as the
//! page's import does, its progress on the import card; once the games are in, the page shows
//! them with the player's name as its filter, ready to be split by side.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;
use kchess_board::player_search::{self, NameGroup, NameMatches};
use kchess_board::widgets;
use kchess_contracts::types::{ExplorerSpeed, GameSite, PlayerGames};

use crate::game_db::GameDbHandle;
use crate::theme::Palette;
use crate::widgets::{Choice, Pill};

const SITES: [GameSite; 2] = [GameSite::Lichess, GameSite::Chesscom];
/// How far back the games go: every game, or the newest so many months.
const PERIODS: [(&str, &str); 5] = [
    ("all", "All games"),
    ("1", "This month"),
    ("3", "Last 3 months"),
    ("12", "Last 12 months"),
    ("36", "Last 3 years"),
];
const PERIOD: &str = "12";
/// Chess.com's speeds, as its games name them: bullet, blitz, rapid and daily.
const CHESSCOM_SPEEDS: [ExplorerSpeed; 4] = [
    ExplorerSpeed::Bullet,
    ExplorerSpeed::Blitz,
    ExplorerSpeed::Rapid,
    ExplorerSpeed::Correspondence,
];

fn site_label(site: GameSite) -> &'static str {
    match site {
        GameSite::Lichess => "Lichess",
        GameSite::Chesscom => "Chess.com",
    }
}

/// The speeds `site` offers.
fn speeds(site: GameSite) -> &'static [ExplorerSpeed] {
    match site {
        GameSite::Lichess => ExplorerSpeed::ALL,
        GameSite::Chesscom => &CHESSCOM_SPEEDS,
    }
}

fn speed_label(site: GameSite, speed: ExplorerSpeed) -> &'static str {
    match (site, speed) {
        (GameSite::Chesscom, ExplorerSpeed::Correspondence) => "Daily",
        _ => widgets::speed_label(speed),
    }
}

/// Opens the dialog, filled from `earlier` (an update's request) when given.
pub fn open(earlier: Option<PlayerGames>, window: &mut Window, cx: &mut App) {
    let dialog = cx.new(|cx| PlayerDialog::new(earlier, window, cx));
    widgets::open_dialog(window, cx, move |dialog_box, _, _| {
        dialog_box
            .title("Download a player's games")
            .w(px(460.))
            .child(dialog.clone())
    });
}

pub struct PlayerDialog {
    site: GameSite,
    username: Entity<InputState>,
    period_select: Entity<SelectState<Vec<Choice>>>,
    months: Option<u32>,
    /// None chosen: every speed.
    speeds: Vec<ExplorerSpeed>,
    rated: bool,
    /// Lichess players whose names start with what is typed.
    names: NameMatches,
    /// The name last picked from the suggestions, so it is not searched again.
    picked: Option<String>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl PlayerDialog {
    fn new(earlier: Option<PlayerGames>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let username = cx.new(|cx| InputState::new(window, cx).placeholder("Username"));
        let choices: Vec<Choice> = PERIODS
            .iter()
            .map(|(value, label)| Choice::new(*value, *label))
            .collect();
        let period_select = cx.new(|cx| SelectState::new(choices, None, window, cx));
        let mut dialog = PlayerDialog {
            site: GameSite::Lichess,
            username,
            period_select,
            months: PERIOD.parse().ok(),
            speeds: Vec::new(),
            rated: true,
            names: NameMatches::default(),
            picked: None,
            error: None,
            _subscriptions: Vec::new(),
        };
        let period = match &earlier {
            Some(earlier) => {
                dialog.site = earlier.site;
                dialog.months = earlier.months;
                dialog.speeds = earlier.speeds.clone().unwrap_or_default();
                dialog.rated = earlier.rated;
                dialog.username.update(cx, |input, cx| {
                    input.set_value(earlier.username.clone(), window, cx)
                });
                earlier
                    .months
                    .map_or_else(|| "all".to_string(), |months| months.to_string())
            }
            None => PERIOD.to_string(),
        };
        // A period no longer offered (from an earlier release) shows as none chosen and is kept.
        dialog.period_select.update(cx, |state, cx| {
            state.set_selected_value(&SharedString::from(period), window, cx)
        });
        dialog._subscriptions = vec![
            cx.subscribe(
                &dialog.period_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        this.months = value.parse::<u32>().ok();
                        cx.notify();
                    }
                },
            ),
            cx.observe(&dialog.username, |this, _, cx| {
                this.error = None;
                cx.notify();
            }),
            cx.subscribe(&dialog.username, |this, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    this.search_names(cx);
                }
            }),
        ];
        dialog
            .username
            .update(cx, |input, cx| input.focus(window, cx));
        dialog
    }

    fn choose_site(&mut self, site: GameSite, cx: &mut Context<Self>) {
        self.site = site;
        // A speed the other site does not offer is dropped.
        self.speeds.retain(|speed| speeds(site).contains(speed));
        self.error = None;
        self.search_names(cx);
        cx.notify();
    }

    /// Lichess players whose names start with what is typed, once typing settles; Chess.com has
    /// no search to ask.
    fn search_names(&mut self, cx: &mut Context<Self>) {
        if self.site != GameSite::Lichess {
            self.names.clear();
            return;
        }
        let typed = self.username.read(cx).value().to_string();
        let picked = self.picked.clone();
        self.names.search(
            &typed,
            picked.as_deref(),
            |dialog: &mut Self| &mut dialog.names,
            cx,
        );
    }

    /// Put a suggested player's name in the field.
    fn pick(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.picked = Some(name.clone());
        self.names.clear();
        self.username
            .update(cx, |input, cx| input.set_value(name, window, cx));
        cx.notify();
    }

    /// The players offered under the name before it matches anyone on Lichess: your accounts on
    /// the site, the players downloaded or looked up lately and, on Lichess, the ones followed;
    /// once a name is typed, only those it starts.
    fn known_names(&self, cx: &App) -> Vec<NameGroup> {
        let accounts = kchess_app::accounts::handle(cx).read(cx);
        let groups = match self.site {
            GameSite::Lichess => vec![
                NameGroup::new("Yours", "player-yours", accounts.connected()),
                NameGroup::new(
                    "Recent",
                    "player-recent",
                    player_search::recent(GameSite::Lichess, cx),
                ),
                NameGroup::new(
                    "Following",
                    "player-followed",
                    accounts
                        .tracked()
                        .into_iter()
                        .map(|account| account.username)
                        .collect(),
                ),
            ],
            GameSite::Chesscom => vec![
                NameGroup::new(
                    "Recent",
                    "player-recent",
                    player_search::recent(GameSite::Chesscom, cx),
                ),
                NameGroup::new(
                    "Synced",
                    "player-synced",
                    accounts
                        .chesscom()
                        .iter()
                        .map(|account| account.username.clone())
                        .collect(),
                ),
            ],
        };
        let typed = self.username.read(cx).value().trim().to_lowercase();
        groups
            .into_iter()
            .map(|mut group| {
                group
                    .names
                    .retain(|name| name.to_lowercase().starts_with(&typed));
                group
            })
            .collect()
    }

    fn toggle_speed(&mut self, speed: ExplorerSpeed, cx: &mut Context<Self>) {
        if let Some(at) = self.speeds.iter().position(|chosen| *chosen == speed) {
            self.speeds.remove(at);
        } else {
            self.speeds.push(speed);
        }
        cx.notify();
    }

    fn request(&self, cx: &App) -> PlayerGames {
        PlayerGames {
            site: self.site,
            username: self.username.read(cx).value().trim().to_string(),
            months: self.months,
            speeds: (!self.speeds.is_empty()).then(|| self.speeds.clone()),
            rated: self.rated,
        }
    }

    /// Starts the download, once the request is one the core takes, and closes the dialog: its
    /// progress shows on the page.
    fn download(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let request = match kchess_contracts::check::player_games(&self.request(cx)) {
            Ok(request) => request,
            Err(refusal) => {
                self.error = Some(refusal.into());
                cx.notify();
                return;
            }
        };
        window.close_dialog(cx);
        // A player's games are seldom large, but they need the room the others do.
        kchess_board::game_db::with_room(None, "Download anyway", window, cx, move |_, cx| {
            let db = cx.global::<GameDbHandle>().0.clone();
            db.update(cx, |db, cx| db.install_player(request.clone(), cx));
        });
    }

    fn render_speeds(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let any = widgets::toggle_button("player-speed-any", "Any speed", self.speeds.is_empty())
            .on_click(cx.listener(|this, _, _, cx| {
                this.speeds.clear();
                cx.notify();
            }));
        h_flex()
            .flex_wrap()
            .gap(px(4.))
            .child(any)
            .children(speeds(self.site).iter().map(|&speed| {
                widgets::toggle_button(
                    SharedString::from(format!("player-speed-{}", speed.as_str())),
                    speed_label(self.site, speed),
                    self.speeds.contains(&speed),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_speed(speed, cx)))
            }))
    }
}

impl Render for PlayerDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let empty = self.username.read(cx).value().trim().is_empty();
        let busy = cx.global::<GameDbHandle>().0.read(cx).busy();
        let chosen = SITES
            .iter()
            .position(|site| *site == self.site)
            .unwrap_or_default();
        let dialog = cx.entity().downgrade();
        let sites = widgets::pills(
            "player-site",
            SITES
                .iter()
                .map(|site| Pill {
                    icon: None,
                    label: SharedString::new_static(site_label(*site)),
                    badge: None,
                    disabled: false,
                })
                .collect(),
            chosen,
            move |index, _, cx| {
                if let Some(&site) = SITES.get(index) {
                    dialog
                        .update(cx, |this, cx| this.choose_site(site, cx))
                        .ok();
                }
            },
            true,
            &palette,
        );
        let rated = {
            let dialog = cx.entity().downgrade();
            widgets::labelled_switch(
                "player-rated",
                "Rated games only",
                self.rated,
                false,
                move |on, _, cx| {
                    let on = *on;
                    dialog
                        .update(cx, |this, cx| {
                            this.rated = on;
                            cx.notify();
                        })
                        .ok();
                },
            )
        };
        // The name in the field is not offered again.
        let typed = self.username.read(cx).value().trim().to_string();
        let dialog = cx.entity().downgrade();
        let suggestions = player_search::name_rows(
            "player-match",
            &self.names,
            self.known_names(cx),
            Some(typed.as_str()),
            &palette,
            |name| {
                let dialog = dialog.clone();
                Box::new(move |_, window, cx| {
                    dialog
                        .update(cx, |this, cx| this.pick(name.clone(), window, cx))
                        .ok();
                })
            },
        );
        v_flex()
            .gap(px(12.))
            .child(sites)
            .child(
                v_flex()
                    .gap(px(8.))
                    .child(Input::new(&self.username).aria_label("Username"))
                    .children(suggestions),
            )
            .child(
                div().w(px(200.)).child(
                    Select::new(&self.period_select)
                        .small()
                        .accessibility_label("Period"),
                ),
            )
            .child(self.render_speeds(cx))
            .child(rated)
            .child(widgets::dialog_footer(
                self.error.clone().map(|error| ("player-error", error)),
                None,
                [
                    Button::new("player-cancel")
                        .ghost()
                        .label("Cancel")
                        .on_click(|_, window, cx| window.close_dialog(cx))
                        .into_any_element(),
                    Button::new("player-download")
                        .primary()
                        .icon(kchess_ui::icon("download"))
                        .label("Download")
                        // One download runs at a time.
                        .disabled(empty || busy)
                        .on_click(cx.listener(|this, _, window, cx| this.download(window, cx)))
                        .into_any_element(),
                ],
                &palette,
            ))
    }
}
