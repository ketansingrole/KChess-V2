//! Home (`pages/index.vue`): where to start, the recent games played here, and connecting Lichess.
//! It reads the unfinished computer game and the accounts again each time it is shown.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;

use crate::assets::icon;
use crate::core::AppCore;
use crate::game::computer::{ComputerGame, Host};
use crate::game::rules;
use crate::pages::Page;
use crate::play_layout::{PAGE_BOTTOM, PAGE_TOP, side_padding};
use crate::played::PlayedHandle;
use crate::theme::Palette;
use crate::widgets;

mod lichess;

/// Recent games on Home (`localGames`).
const RECENT: usize = 6;
/// The page header's empty row above the tiles.
const HEADER: Pixels = px(20.);

pub struct HomePage {
    /// "Move 4 · White · Club Player · ~1600" while a computer game is unfinished.
    continue_game: Option<String>,
    /// Your Lichess activity, once an account is connected, and the account it shows.
    activity: Option<(String, Entity<lichess::LichessActivity>)>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for HomePage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// `computerDescription`: the move to play, your side and the level.
fn continue_line(session: &Value) -> Option<String> {
    let moves = session["moves"].as_array().map_or(0, Vec::len);
    if moves == 0 {
        return None;
    }
    let host = Host {
        now: 0.,
        allowed: true,
        ready: false,
    };
    if ComputerGame::new(Some(session), &host).ok()?.over(&host) {
        return None;
    }
    let side = if session["color"] == "black" {
        "Black"
    } else {
        "White"
    };
    let level = rules("engineLevelLabel", &[session["level"].clone()])
        .ok()
        .and_then(|label| label.as_str().map(str::to_string))
        .unwrap_or_default();
    Some(format!("Move {} · {side} · {level}", moves / 2 + 1))
}

/// `localResult`: "1–0", "½–½", or "Unfinished".
fn result_text(result: &str) -> String {
    match result {
        "1/2-1/2" => "½–½".into(),
        "*" => "Unfinished".into(),
        result => result.replace('-', "–"),
    }
}

/// "Oct 22": the day a game started, in local time.
fn short_date(millis: f64) -> String {
    use chrono::TimeZone as _;
    chrono::Local
        .timestamp_millis_opt(millis as i64)
        .single()
        .map(|date| date.format("%b %-d").to_string())
        .unwrap_or_default()
}

impl HomePage {
    pub fn new(cx: &mut Context<Self>) -> HomePage {
        let played = cx.global::<PlayedHandle>().0.clone();
        let mut page = HomePage {
            continue_game: None,
            activity: None,
            focus: cx.focus_handle(),
            _subscriptions: vec![
                cx.observe(&played, |_, _, cx| cx.notify()),
                cx.observe(&crate::accounts::handle(cx), |_, _, cx| cx.notify()),
            ],
        };
        page.refresh(cx);
        page
    }

    /// Read the unfinished computer game and the accounts again (the page was shown).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let core = cx.global::<AppCore>().clone();
        let library = core.call("library", Vec::new());
        crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.refresh(cx));
        if let Some((account, activity)) = &self.activity {
            let account = account.clone();
            activity.update(cx, |activity, cx| activity.load(&account, cx));
        }
        cx.spawn(async move |this, cx| {
            let library = library.await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(library) = library {
                    this.continue_game = continue_line(&library["sessions"]["computer"]);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// `.home-tile`: an activity to start, its icon in the primary color.
fn tile(
    id: &'static str,
    glyph: Icon,
    label: &'static str,
    sub: Option<String>,
    primary: bool,
    page: Page,
    palette: &Palette,
) -> impl IntoElement {
    h_flex()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .flex_1()
        .min_w_0()
        .min_h(px(72.))
        .gap(px(12.))
        .px(px(16.))
        .py(px(14.))
        .rounded(px(14.))
        .border_1()
        .cursor_pointer()
        .when(primary, |tile| {
            tile.bg(palette.primary)
                .border_color(palette.primary)
                .text_color(palette.bg)
                .hover(|tile| tile.opacity(0.92))
        })
        .when(!primary, |tile| {
            tile.bg(palette.elevated)
                .border_color(palette.border)
                .text_color(palette.highlighted)
                .hover(|tile| {
                    tile.bg(palette.accented)
                        .border_color(palette.border_accented)
                })
        })
        .on_click(move |_, window, cx| crate::shell::open(page, window, cx))
        .child(
            glyph
                .size(px(24.))
                .flex_none()
                .when(!primary, |glyph| glyph.text_color(palette.primary)),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(label))
                .children(
                    sub.map(|sub| div().truncate().text_size(px(13.)).opacity(0.8).child(sub)),
                ),
        )
        .when(primary, |tile| {
            tile.child(icon("arrow-right").size(px(18.)).flex_none())
        })
}

/// A quiet shortcut under the tiles.
fn shortcut(id: &'static str, glyph: Icon, label: &'static str, page: Page) -> impl IntoElement {
    Button::new(id)
        .ghost()
        .icon(glyph)
        .label(label)
        .on_click(move |_, window, cx| crate::shell::open(page, window, cx))
}

/// `.card-header` with a `.section-title`.
fn card_title(title: &'static str, palette: &Palette) -> Div {
    div()
        .text_size(px(15.))
        .font_weight(FontWeight(650.))
        .text_color(palette.highlighted)
        .child(title)
}

/// `UEmpty variant="naked" size="sm"`: an icon, a title and the action that fills it.
fn empty(glyph: Icon, title: &'static str, action: Option<Button>, palette: &Palette) -> Div {
    v_flex()
        .items_center()
        .gap(px(10.))
        .py(px(24.))
        .child(glyph.size(px(20.)).text_color(palette.text_muted))
        .child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.highlighted)
                .child(title),
        )
        .children(action)
}

impl Render for HomePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let (connected, busy) = {
            let accounts = crate::accounts::handle(cx);
            let accounts = accounts.read(cx);
            (accounts.active(cx), accounts.busy)
        };
        let width = window.viewport_size().width;
        let columns = if width < px(560.) {
            1
        } else if width < px(1100.) {
            2
        } else {
            4
        };
        let continuing = self.continue_game.clone();
        let tiles: Vec<AnyElement> = vec![
            tile(
                "home-computer",
                icon("monitor"),
                if continuing.is_some() {
                    "Continue your game"
                } else {
                    "Play the computer"
                },
                continuing,
                true,
                Page::Computer,
                &palette,
            )
            .into_any_element(),
            tile(
                "home-puzzles",
                icon("puzzle"),
                "Practice puzzles",
                None,
                false,
                Page::Puzzles,
                &palette,
            )
            .into_any_element(),
            tile(
                "home-analysis",
                icon("microscope"),
                "Analysis board",
                None,
                false,
                Page::Analysis,
                &palette,
            )
            .into_any_element(),
            tile(
                "home-online",
                icon("globe-2"),
                "Play on Lichess",
                None,
                false,
                Page::Online,
                &palette,
            )
            .into_any_element(),
        ];
        let mut tile_rows = Vec::new();
        let mut tiles = tiles.into_iter().peekable();
        while tiles.peek().is_some() {
            tile_rows.push(
                h_flex()
                    .items_stretch()
                    .gap(px(12.))
                    .children(tiles.by_ref().take(columns)),
            );
        }

        let mut games: Vec<Value> = cx
            .global::<PlayedHandle>()
            .0
            .read(cx)
            .games()
            .iter()
            .filter(|game| game["source"] != "clock")
            .cloned()
            .collect();
        games.sort_by(|a, b| {
            let started = |game: &Value| game["startedAt"].as_f64().unwrap_or_default();
            started(b).total_cmp(&started(a))
        });
        games.truncate(RECENT);
        let recent = widgets::card(&palette)
            .flex_1()
            .min_w_0()
            .child(
                h_flex()
                    .justify_between()
                    .mb(px(14.))
                    .child(card_title("Recent games", &palette))
                    .when(!games.is_empty(), |header| {
                        header.child(
                            // The arrow trails the label (`trailing-icon`).
                            Button::new("home-view-all")
                                .text()
                                .xsmall()
                                .accessibility_label("View all")
                                .child(
                                    h_flex()
                                        .gap(px(4.))
                                        .child("View all")
                                        .child(icon("arrow-right").size(px(14.))),
                                )
                                .on_click(|_, window, cx| {
                                    crate::shell::open(Page::History, window, cx)
                                }),
                        )
                    }),
            )
            .when(games.is_empty(), |card| {
                card.child(empty(icon("history"), "No saved games yet", None, &palette))
            })
            .child(
                v_flex()
                    .mx(px(-8.))
                    .children(games.into_iter().enumerate().map(|(index, game)| {
                        let id = game["id"].as_str().unwrap_or_default().to_string();
                        let computer = game["source"] == "computer";
                        h_flex()
                            .id(("home-game", index))
                            .role(Role::Button)
                            .aria_label(format!(
                                "{} vs {}",
                                game["white"].as_str().unwrap_or_default(),
                                game["black"].as_str().unwrap_or_default()
                            ))
                            .gap(px(12.))
                            .px(px(8.))
                            .py(px(9.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .hover(|row| row.bg(palette.accented))
                            .on_click(move |_, window, cx| crate::shell::open_game(&id, window, cx))
                            .child(
                                div().w(px(20.)).flex_none().child(
                                    if computer {
                                        icon("monitor")
                                    } else {
                                        icon("users-round")
                                    }
                                    .size(px(16.))
                                    .text_color(palette.text_muted),
                                ),
                            )
                            .child(div().flex_1().min_w_0().truncate().child(format!(
                                "{} vs {}",
                                game["white"].as_str().unwrap_or_default(),
                                game["black"].as_str().unwrap_or_default()
                            )))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(result_text(game["result"].as_str().unwrap_or("*"))),
                            )
                            .child(
                                div()
                                    .w(px(64.))
                                    .flex_none()
                                    .text_right()
                                    .text_size(px(13.))
                                    .text_color(palette.text_muted)
                                    .child(short_date(
                                        game["startedAt"].as_f64().unwrap_or_default(),
                                    )),
                            )
                    })),
            );
        let lichess = widgets::card(&palette)
            .flex_1()
            .min_w_0()
            .child(card_title("Lichess", &palette))
            .child(match &connected {
                None => empty(
                    icon("chart-line"),
                    "Ratings, games and puzzle progress",
                    Some(
                        Button::new("home-connect")
                            .primary()
                            .small()
                            .icon(icon("log-in"))
                            .label("Connect Lichess")
                            .loading(busy)
                            .disabled(busy)
                            .on_click(|_, window, cx| {
                                crate::accounts::handle(cx)
                                    .update(cx, |accounts, cx| accounts.connect(window, cx))
                            }),
                    ),
                    &palette,
                ),
                // `Your Lichess activity` (ratings, chart and synced games) is not drawn here yet.
                Some(username) => {
                    empty(icon("chart-line"), "Your Lichess activity", None, &palette).child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(format!("@{username}")),
                    )
                }
            });
        let stacked = width < px(900.);
        // A connected account's activity replaces the local cards (`hasConnectedAccount`).
        let activity = match connected.clone() {
            Some(account) => {
                let stale = self
                    .activity
                    .as_ref()
                    .is_none_or(|(shown, _)| *shown != account);
                if stale {
                    let view = self
                        .activity
                        .take()
                        .map(|(_, view)| view)
                        .unwrap_or_else(|| cx.new(|cx| lichess::LichessActivity::new(window, cx)));
                    view.update(cx, |view, cx| view.load(&account, cx));
                    self.activity = Some((account, view));
                }
                self.activity.as_ref().map(|(_, view)| view.clone())
            }
            None => {
                self.activity = None;
                None
            }
        };
        // `PageHeader`: Sync games for the connected account, above the tiles.
        let online = crate::network::online(cx);
        let header = connected.clone().map(|account| {
            h_flex().justify_end().mb(px(20.)).child(
                Button::new("home-sync")
                    .outline()
                    .icon(icon("refresh-cw"))
                    .label("Sync games")
                    .loading(busy)
                    .disabled(!online)
                    .on_click(move |_, window, cx| {
                        let name = account.clone();
                        crate::accounts::handle(cx)
                            .update(cx, |accounts, cx| accounts.sync(Some(name), window, cx))
                    }),
            )
        });
        div()
            .id("home-page")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .pt(PAGE_TOP + if header.is_some() { px(0.) } else { HEADER })
                    .pb(PAGE_BOTTOM)
                    .px(side_padding(window))
                    .children(header)
                    .child(
                        v_flex().gap(px(12.)).mb(px(32.)).children(tile_rows).child(
                            h_flex()
                                .flex_wrap()
                                .gap(px(8.))
                                .ml(px(-10.))
                                .child(shortcut(
                                    "home-history",
                                    icon("history"),
                                    "Your game history",
                                    Page::History,
                                ))
                                .child(shortcut(
                                    "home-local",
                                    icon("users-round"),
                                    "Play over the board",
                                    Page::Local,
                                ))
                                .child(shortcut(
                                    "home-practice",
                                    icon("graduation-cap"),
                                    "Practice chess skills",
                                    Page::Practice,
                                ))
                                .child(shortcut(
                                    "home-studies",
                                    icon("library-big"),
                                    "Your studies",
                                    Page::Studies,
                                )),
                        ),
                    )
                    .child(match activity {
                        Some(activity) => activity.into_any_element(),
                        None => {
                            // `.two-columns`: 1.15 to 1, the cards as tall as each other.
                            if stacked {
                                v_flex().gap(px(20.))
                            } else {
                                h_flex().items_stretch().gap(px(20.))
                            }
                            .child(
                                div()
                                    .flex_basis(relative(0.535))
                                    .min_w_0()
                                    .child(recent.h_full()),
                            )
                            .child(
                                div()
                                    .flex_basis(relative(0.465))
                                    .min_w_0()
                                    .child(lichess.h_full()),
                            )
                            .into_any_element()
                        }
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::result_text;

    #[test]
    fn results_read_as_on_a_scoresheet() {
        assert_eq!(result_text("1-0"), "1–0");
        assert_eq!(result_text("1/2-1/2"), "½–½");
        assert_eq!(result_text("*"), "Unfinished");
    }
}
