//! Finding a player by name, wherever a page asks for one (Players, the guess trainer's game
//! finder, the game database's player download): the Lichess players whose names start with what
//! is typed, the players looked up lately on each site, and the list under the field that offers them. Also a name that opens the player's profile (`player_link`),
//! which the Players page answers (`OpenPlayer`).

use gpui_kit::component::input::InputEvent;
use gpui_kit::*;
use kchess_app::log::ResultExt as _;
use kchess_contracts::calls;
use kchess_contracts::check::Username;
use kchess_contracts::types::{GameSite, PlayerSuggestion};

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

/// The players looked up lately on each site, newest first, shared by every name search.
fn recent_key(site: GameSite) -> &'static str {
    match site {
        GameSite::Lichess => "kchess:players-recent",
        GameSite::Chesscom => "kchess:chesscom-players-recent",
    }
}
const RECENT_KEPT: usize = 8;
/// Names offered under a search.
pub const SUGGESTED: usize = 12;
/// How long typing settles before names are searched.
const SEARCH_WAIT: std::time::Duration = std::time::Duration::from_millis(250);

/// Put a player looked up on `site` first among its recent ones, wherever players are searched.
pub fn remember(site: GameSite, username: &str, cx: &mut App) {
    let mut recent = recent(site, cx);
    recent.retain(|name| !name.eq_ignore_ascii_case(username));
    recent.insert(0, username.to_string());
    recent.truncate(RECENT_KEPT);
    crate::prefs::set(cx, recent_key(site), recent);
}

/// The players looked up lately on `site`, newest first.
pub fn recent(site: GameSite, cx: &App) -> Vec<String> {
    crate::prefs::get(cx, recent_key(site), Vec::new())
}

pub fn clear_recent(site: GameSite, cx: &mut App) {
    crate::prefs::set(cx, recent_key(site), Vec::<String>::new());
}

/// The players whose names start with what is typed, once typing settles, and the list under
/// the field that offers them (`name_search`): open while the field has the keys, with the row
/// the arrows or the pointer chose.
#[derive(Default)]
pub struct NameMatches {
    found: Vec<PlayerSuggestion>,
    epoch: Epoch,
    open: bool,
    /// The row chosen with the arrows or the pointer, and its name; none until one is.
    chosen: Option<(usize, String)>,
}

impl NameMatches {
    pub fn found(&self) -> &[PlayerSuggestion] {
        &self.found
    }

    /// Forget the names found, and any search still to answer, and close the list.
    pub fn clear(&mut self) {
        self.forget();
        self.close();
    }

    /// Forget the names found, and any search still to answer; the list stays as it is, for
    /// the next search to fill.
    pub fn forget(&mut self) {
        self.epoch.invalidate();
        self.found.clear();
        self.chosen = None;
    }

    /// Close the list; the row chosen goes with it.
    pub fn close(&mut self) {
        self.open = false;
        self.chosen = None;
    }

    /// Follow the search field (`focused` while it has the keys): the list opens as the field
    /// takes the keys or is typed in, and closes as it loses them; typing chooses afresh.
    pub fn follow(&mut self, event: &InputEvent, focused: bool) {
        match event {
            InputEvent::Focus => self.open = true,
            InputEvent::Blur => self.close(),
            InputEvent::Change => {
                self.chosen = None;
                self.open = focused;
            }
            InputEvent::PressEnter { .. } => {}
        }
    }

    /// The name Enter opens: the row chosen, or else what is typed. The list closes.
    pub fn entered(&mut self, typed: &str) -> Option<String> {
        let chosen = self.chosen.take().map(|(_, name)| name);
        self.close();
        let typed = typed.trim();
        chosen.or_else(|| (!typed.is_empty()).then(|| typed.to_string()))
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
            this.update(cx, |owner, cx| {
                let matches = matches(owner);
                if !matches.epoch.is_current(epoch) {
                    return;
                }
                if let Some(mut found) = found.warn_on_err() {
                    found.truncate(SUGGESTED);
                    matches.found = found;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
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
        .rounded(radii::BOX)
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

/// Names offered under a search until what is typed matches a player: a row's label, the id of
/// its chips, its names, and an action at its end (Clear).
pub struct NameGroup {
    pub label: &'static str,
    pub id: &'static str,
    pub names: Vec<String>,
    pub action: Option<AnyElement>,
}

impl NameGroup {
    pub fn new(label: &'static str, id: &'static str, names: Vec<String>) -> NameGroup {
        NameGroup {
            label,
            id,
            names,
            action: None,
        }
    }
}

/// Each name once, in the first group that has it, at most `SUGGESTED` in all, leaving out
/// `shown` (the player already in the field).
fn once_each(groups: Vec<Vec<String>>, shown: Option<&str>) -> Vec<Vec<String>> {
    let mut seen: std::collections::HashSet<String> =
        shown.map(str::to_lowercase).into_iter().collect();
    let mut left = SUGGESTED;
    groups
        .into_iter()
        .map(|names| {
            let kept: Vec<String> = names
                .into_iter()
                .filter(|name| seen.insert(name.to_lowercase()))
                .take(left)
                .collect();
            left -= kept.len();
            kept
        })
        .collect()
}

/// A player search: its field (`field`, holding `typed`) and, while the field has the keys, a
/// list under it as the command palette lists its results. Before anything is typed it offers
/// the `known` groups (each name once, `shown` left out); as a name is typed, the Lichess
/// players it starts, or else the known names it starts. With `typed_hint` the name typed comes
/// first, `typed_hint` saying what opening it does. The arrows choose a row, Enter or a press
/// opens it (`pick`, which the owner's Enter runs with `NameMatches::entered`), and Escape closes
/// the list. `matches` finds the search's `NameMatches` in its owner.
pub struct NameSearch<'a> {
    /// The rows' ids: the typed name's is `{id}-typed`, the players found `(id, index)`.
    pub id: &'static str,
    pub field: AnyElement,
    pub typed: &'a str,
    pub typed_hint: Option<&'static str>,
    pub known: Vec<NameGroup>,
    pub shown: Option<&'a str>,
}

/// One row of a search's list.
struct NameRow {
    id: ElementId,
    name: String,
    title: Option<String>,
    online: bool,
    /// What opening the typed name does, on its row.
    hint: Option<&'static str>,
}

pub fn name_search<V: 'static>(
    search: NameSearch<'_>,
    found: &NameMatches,
    matches: fn(&mut V) -> &mut NameMatches,
    pick: fn(&mut V, String, &mut Window, &mut Context<V>),
    palette: &Palette,
    cx: &mut Context<V>,
) -> Div {
    use gpui_kit::component::{h_flex, v_flex};
    use gpui_kit::prelude::FluentBuilder as _;
    let NameSearch {
        id,
        field,
        typed,
        typed_hint,
        known,
        shown,
    } = search;
    let typed = typed.trim();
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let mut sections: Vec<(Option<&'static str>, Option<AnyElement>, Vec<NameRow>)> = Vec::new();
    if let Some(hint) = typed_hint
        && crate::game::is_username(typed)
        && !shown.is_some_and(|shown| same(shown, typed))
        && !found
            .found
            .iter()
            .any(|player| same(&player.username, typed))
    {
        sections.push((
            None,
            None,
            vec![NameRow {
                id: SharedString::from(format!("{id}-typed")).into(),
                name: typed.to_string(),
                title: None,
                online: false,
                hint: Some(hint),
            }],
        ));
    }
    if !found.found.is_empty() {
        let rows = found
            .found
            .iter()
            .enumerate()
            .map(|(index, player)| NameRow {
                id: (id, index).into(),
                name: player.username.clone(),
                title: player.title.clone(),
                online: player.online == Some(true),
                hint: None,
            })
            .collect();
        sections.push((Some("Players"), None, rows));
    } else {
        let typed_lower = typed.to_lowercase();
        let starts = |name: &String| name.to_lowercase().starts_with(&typed_lower);
        let names = once_each(
            known
                .iter()
                .map(|group| {
                    group
                        .names
                        .iter()
                        .filter(|name| starts(name))
                        .cloned()
                        .collect()
                })
                .collect(),
            shown,
        );
        for (group, names) in known.into_iter().zip(names) {
            if names.is_empty() {
                continue;
            }
            let rows = names
                .into_iter()
                .enumerate()
                .map(|(index, name)| NameRow {
                    id: (group.id, index).into(),
                    name,
                    title: None,
                    online: false,
                    hint: None,
                })
                .collect();
            sections.push((Some(group.label), group.action, rows));
        }
    }
    let names: Vec<String> = sections
        .iter()
        .flat_map(|(_, _, rows)| rows.iter().map(|row| row.name.clone()))
        .collect();
    // The typed name is chosen until the arrows or the pointer choose another row.
    let chosen = found.chosen.as_ref().map(|(at, _)| *at).or_else(|| {
        (typed_hint.is_some()
            && sections
                .first()
                .is_some_and(|(label, _, _)| label.is_none()))
        .then_some(0)
    });
    let open = found.open && !names.is_empty();
    let keys = {
        let names = names.clone();
        cx.listener(move |owner: &mut V, event: &KeyDownEvent, _, cx| {
            let list = matches(owner);
            if !list.open || names.is_empty() {
                return;
            }
            let count = names.len();
            let at = list.chosen.as_ref().map(|(at, _)| *at).or(chosen);
            let next = match event.keystroke.key.as_str() {
                "down" => at.map_or(0, |at| (at + 1) % count),
                "up" => at.map_or(count - 1, |at| (at + count - 1) % count),
                "escape" => {
                    list.close();
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                _ => return,
            };
            list.chosen = Some((next, names[next].clone()));
            cx.stop_propagation();
            cx.notify();
        })
    };
    let mut at = 0usize;
    let list = open.then(|| {
        let sections: Vec<AnyElement> = sections
            .into_iter()
            .enumerate()
            .map(|(number, (label, action, rows))| {
                let heading = label.map(|label| {
                    h_flex()
                        .h(px(24.))
                        .px(px(8.))
                        .justify_between()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.text_muted)
                        .child(label)
                        .children(action)
                });
                let rows: Vec<AnyElement> = rows
                    .into_iter()
                    .map(|row| {
                        let index = at;
                        at += 1;
                        let selected = chosen == Some(index);
                        let (hover_name, pick_name) = (row.name.clone(), row.name.clone());
                        let label = match &row.title {
                            Some(title) => format!("{title} {}", row.name),
                            None => row.name.clone(),
                        };
                        h_flex()
                            .id(row.id)
                            .test_support()
                            .role(Role::ListBoxOption)
                            .aria_label(match row.hint {
                                Some(hint) => format!("{hint} {label}"),
                                None if row.online => format!("{label}, online"),
                                None => label,
                            })
                            .aria_selected(selected)
                            .h(px(32.))
                            .px(px(8.))
                            .gap(px(8.))
                            .rounded(radii::BOX)
                            .text_size(px(13.))
                            .cursor_pointer()
                            .when(selected, |row| {
                                row.bg(crate::theme::tint(palette.primary, 12.))
                            })
                            .on_mouse_move(cx.listener(move |owner: &mut V, _, _, cx| {
                                let list = matches(owner);
                                if list.chosen.as_ref().map(|(at, _)| *at) != Some(index) {
                                    list.chosen = Some((index, hover_name.clone()));
                                    cx.notify();
                                }
                            }))
                            // Down, not click: the field loses the keys, and the list closes,
                            // before a click lands.
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |owner: &mut V, _, window, cx| {
                                    cx.stop_propagation();
                                    matches(owner).close();
                                    pick(owner, pick_name.clone(), window, cx);
                                    cx.notify();
                                }),
                            )
                            // Names start where the section's label does; only the typed row has an
                            // icon before it.
                            .when(row.hint.is_some(), |row| {
                                row.child(
                                    kchess_ui::icon("search")
                                        .size(px(14.))
                                        .flex_none()
                                        .text_color(palette.text_muted),
                                )
                            })
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(4.))
                                    .children(row.title.map(|title| {
                                        div()
                                            .flex_none()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(palette.warning)
                                            .child(title)
                                    }))
                                    .child(
                                        div()
                                            .truncate()
                                            .text_color(palette.highlighted)
                                            .child(row.name),
                                    )
                                    .when(row.online, |name| {
                                        name.child(
                                            div()
                                                .size(px(6.))
                                                .flex_none()
                                                .rounded_full()
                                                .bg(palette.success),
                                        )
                                    }),
                            )
                            .children(row.hint.map(|hint| {
                                div()
                                    .flex_none()
                                    .text_size(px(12.))
                                    .text_color(palette.text_muted)
                                    .child(hint)
                            }))
                            .into_any_element()
                    })
                    .collect();
                v_flex()
                    .py(px(2.))
                    .when(number > 0, |section| {
                        section.border_t_1().border_color(palette.border)
                    })
                    .children(heading)
                    .children(rows)
                    .into_any_element()
            })
            .collect();
        // Over what follows the field, the page's clip left behind.
        deferred(
            v_flex()
                .id(SharedString::from(format!("{id}-list")))
                .test_support()
                .role(Role::ListBox)
                .absolute()
                .top(relative(1.))
                .left_0()
                .right_0()
                .mt(px(4.))
                .max_h(px(336.))
                .overflow_y_scroll()
                .occlude()
                .p(px(4.))
                .rounded(radii::BOX)
                .border_1()
                .border_color(palette.border_accented)
                .bg(if palette.dark {
                    palette.elevated
                } else {
                    palette.bg
                })
                .shadow(vec![BoxShadow {
                    color: hsla(0., 0., 0., 0.3),
                    offset: point(px(0.), px(12.)),
                    blur_radius: px(32.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .children(sections),
        )
        .with_priority(1)
    });
    div()
        .relative()
        .w_full()
        .on_key_down(keys)
        .child(field)
        .children(list)
}

/// A page's player search as Players and Following lay it out: the field centred, as wide as a
/// name needs, with what goes before it (a way back) at the left. The two sides share the room
/// left, so the field stays in the middle.
pub fn centred_search(search: impl IntoElement, before: Option<AnyElement>) -> Div {
    use gpui_kit::component::h_flex;
    h_flex()
        .w_full()
        .gap(px(12.))
        .child(div().flex_1().min_w_0().children(before))
        // A name is at most 30 characters: the field stops at 480px.
        .child(
            div()
                .flex_auto()
                .max_w(px(480.))
                .min_w(px(240.))
                .child(search),
        )
        .child(div().flex_1().min_w_0())
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

    /// A name offered in two groups shows in the first, the player in the field in neither, and
    /// no more than `SUGGESTED` in all.
    #[test]
    fn known_names_are_offered_once_each() {
        let names = |names: &[&str]| names.iter().map(|name| name.to_string()).collect();
        let groups = super::once_each(
            vec![names(&["Alice", "bob", "Dave"]), names(&["Bob", "Carol"])],
            Some("dave"),
        );
        assert_eq!(groups, [vec!["Alice", "bob"], vec!["Carol"]]);
        let many = (0..20).map(|n| format!("p{n}")).collect();
        let groups = super::once_each(vec![many, names(&["late"])], None);
        assert_eq!(groups[0].len(), super::SUGGESTED);
        assert!(groups[1].is_empty());
    }
}
