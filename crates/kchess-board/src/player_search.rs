//! Finding a Lichess player by name, wherever a page asks for one (Players, the guess trainer's
//! game finder): the players whose names start with what is typed, the players looked up lately,
//! and the chips that offer them. Also a name that opens the player's profile (`player_link`),
//! which the Players page answers (`OpenPlayer`).

use gpui_kit::*;
use kchess_contracts::calls;
use kchess_contracts::check::Username;
use kchess_contracts::types::PlayerSuggestion;

use crate::core::AppCore;
use crate::loading::Epoch;
use crate::theme::{Palette, radii};
use crate::widgets::{KeyboardFocus as _, Pressable as _};

/// How a name opens its player: the Players page's `open_player`, which the app sets as it starts
/// (`cx.set_global(OpenPlayer(..))`); until then a name opens nothing.
#[derive(Clone, Copy)]
pub struct OpenPlayer(pub fn(&str, &mut Window, &mut App));

impl Global for OpenPlayer {}

/// Open `username`'s profile on the Players page.
pub fn open_player(username: &str, window: &mut Window, cx: &mut App) {
    if let Some(OpenPlayer(open)) = cx.try_global::<OpenPlayer>().copied() {
        open(username, window, cx);
    }
}

/// A name that opens the player's profile; `ring` outlines it under the keyboard.
pub fn player_link(id: impl Into<ElementId>, name: String, ring: Hsla) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Link)
        .keyboard_focus(ring)
        .aria_label(name.clone())
        .truncate()
        .cursor_pointer()
        .pressable_link()
        .child(name.clone())
        .on_click(move |_, window, cx| {
            // A name inside a clickable row opens the player, not the row.
            cx.stop_propagation();
            open_player(&name, window, cx)
        })
}

/// The players looked up lately, newest first, shared by every name search.
const RECENT_KEY: &str = "kchess:players-recent";
const RECENT_KEPT: usize = 8;
/// Names offered under a search.
pub const SUGGESTED: usize = 12;
/// How long typing settles before names are searched.
const SEARCH_WAIT: std::time::Duration = std::time::Duration::from_millis(250);

/// Put a player looked up first among the recent ones, wherever players are searched.
pub fn remember(username: &str, cx: &mut App) {
    let mut recent = recent(cx);
    recent.retain(|name| !name.eq_ignore_ascii_case(username));
    recent.insert(0, username.to_string());
    recent.truncate(RECENT_KEPT);
    crate::prefs::set(cx, RECENT_KEY, recent);
}

/// The players looked up lately, newest first.
pub fn recent(cx: &App) -> Vec<String> {
    crate::prefs::get(cx, RECENT_KEY, Vec::new())
}

pub fn clear_recent(cx: &mut App) {
    crate::prefs::set(cx, RECENT_KEY, Vec::<String>::new());
}

/// The players whose names start with what is typed, once typing settles.
#[derive(Default)]
pub struct NameMatches {
    found: Vec<PlayerSuggestion>,
    epoch: Epoch,
}

impl NameMatches {
    pub fn found(&self) -> &[PlayerSuggestion] {
        &self.found
    }

    /// Forget the names found, and any search still to answer.
    pub fn clear(&mut self) {
        self.epoch.invalidate();
        self.found.clear();
    }

    /// Search the names starting with `typed`, unless it is too short to say, not a username,
    /// the player `shown` already, or there is no connection. `matches` finds this in its owner.
    pub fn search<V: 'static>(
        &mut self,
        typed: &str,
        shown: Option<&str>,
        matches: fn(&mut V) -> &mut NameMatches,
        cx: &mut Context<V>,
    ) {
        let epoch = self.epoch.start();
        let term = typed.trim().to_string();
        if term.chars().count() < 3
            || !crate::game::is_username(&term)
            || shown.is_some_and(|name| name.eq_ignore_ascii_case(&term))
            || !crate::network::online(cx)
        {
            self.found.clear();
            cx.notify();
            return;
        }
        let core = cx.global::<AppCore>().clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_WAIT).await;
            let current = this
                .update(cx, |owner, _| matches(owner).epoch.is_current(epoch))
                .unwrap_or(false);
            if !current {
                return;
            }
            let found = core
                .request(calls::SearchPlayers {
                    term: Username(term),
                })
                .await;
            let _ = this.update(cx, |owner, cx| {
                let matches = matches(owner);
                if !matches.epoch.is_current(epoch) {
                    return;
                }
                if let Ok(mut found) = found {
                    found.truncate(SUGGESTED);
                    matches.found = found;
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// A group of name chips under a search: its label in a fixed column on the left, so it reads
/// as a heading and not as one more name, then its chips, wrapping, and an action at the end.
pub fn chip_row(
    label: &'static str,
    chips: Vec<AnyElement>,
    action: Option<AnyElement>,
    palette: &Palette,
) -> Div {
    use gpui_kit::component::h_flex;
    h_flex()
        .items_start()
        .gap(px(8.))
        .child(
            div()
                .w(px(72.))
                .flex_none()
                .h(px(24.))
                .flex()
                .items_center()
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(label),
        )
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .children(chips)
                .children(action),
        )
}

/// A player's name as a filled chip under a search: their title in the leaderboard's colour,
/// and a dot while they are online.
pub fn name_chip(
    id: impl Into<ElementId>,
    name: &str,
    title: Option<&str>,
    online: bool,
    palette: &Palette,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    use gpui_kit::component::h_flex;
    use gpui_kit::prelude::FluentBuilder as _;
    let label = match title {
        Some(title) => format!("{title} {name}"),
        None => name.to_string(),
    };
    h_flex()
        .id(id)
        .on_click(on_click)
        .test_support()
        .role(Role::Button)
        .aria_label(if online {
            format!("{label}, online")
        } else {
            label
        })
        .keyboard_focus(palette.primary)
        .cursor_pointer()
        .h(px(24.))
        .px(px(8.))
        .gap(px(4.))
        .rounded(radii::SM)
        .bg(palette.accented)
        .text_size(px(13.))
        .text_color(palette.toned)
        .pressable_text(palette.highlighted)
        .when(online, |chip| {
            chip.child(div().size(px(6.)).rounded_full().bg(palette.success))
        })
        .children(title.map(|title| {
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.warning)
                .child(title.to_string())
        }))
        .child(name.to_string())
}

/// The chips of the names found, under `id`, each running `pick` with its name.
pub fn match_chips(
    id: &'static str,
    matches: &NameMatches,
    palette: &Palette,
    pick: impl Fn(String) -> Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
) -> Vec<AnyElement> {
    matches
        .found
        .iter()
        .enumerate()
        .map(|(index, found)| {
            let name = found.username.clone();
            let on_click = pick(name.clone());
            name_chip(
                (id, index),
                &name,
                found.title.as_deref(),
                found.online == Some(true),
                palette,
                move |event, window, cx| on_click(event, window, cx),
            )
            .into_any_element()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    thread_local! {
        static OPENED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    /// A name opens its player through the Players page the app set, and nothing before it is set.
    #[gpui_kit::test]
    fn a_name_opens_its_player_through_the_page_the_app_set(cx: &mut gpui_kit::TestAppContext) {
        let mut app = crate::testing::Started::new(cx);
        app.with_window(|window, cx| {
            super::open_player("before", window, cx);
            cx.set_global(super::OpenPlayer(|name, _, _| {
                OPENED.with(|opened| opened.borrow_mut().push(name.to_string()))
            }));
            super::open_player("alice", window, cx);
        });
        assert_eq!(OPENED.with(|opened| opened.take()), ["alice"]);
    }
}
