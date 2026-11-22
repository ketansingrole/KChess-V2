//! The notification centre: a bell at the right end of the top bar, with a dot while something
//! is unread, and under it a panel of the notifications KChess showed (the core keeps the last
//! 100, for 30 days: `kchess_app::notifications`). Opening one goes where it points and marks it
//! read; it closes again without leaving the page, so it never takes you away from a game.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::types::{NotificationEntry, NotificationLink};

use crate::assets::icon;
use crate::pages::Page;
use crate::theme::Palette;

pub use kchess_app::notifications::{clear, handle, mark_read, record, unread};

/// The panel's width, and the most of the window's height it takes.
const PANEL_WIDTH: Pixels = px(360.);
const PANEL_HEIGHT: Pixels = px(440.);

/// The bell and its panel, one in each window's top bar.
pub struct Bell {
    open: bool,
    /// Redraws the open panel when a challenge it lists is answered or withdrawn.
    challenges: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

/// The bell of the window made last, for the shortcut to open.
struct BellGlobal(WeakEntity<Bell>);

impl Global for BellGlobal {}

impl Bell {
    pub fn new(cx: &mut Context<Self>) -> Bell {
        let centre = handle(cx);
        cx.set_global(BellGlobal(cx.entity().downgrade()));
        Bell {
            open: false,
            challenges: None,
            _subscriptions: vec![cx.observe(&centre, |_, _, cx| cx.notify())],
        }
    }

    fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        self.challenges = open.then(|| {
            let online = crate::online::handle(cx);
            cx.observe(&online, |_, _, cx| cx.notify())
        });
        cx.notify();
    }
}

/// Open or close the bell's panel in the window in front (the shortcut).
pub fn toggle_panel(_: &mut Window, cx: &mut App) {
    if let Some(bell) = cx
        .try_global::<BellGlobal>()
        .and_then(|bell| bell.0.upgrade())
    {
        bell.update(cx, |bell, cx| bell.set_open(!bell.open, cx));
    }
}

/// Open the bell's panel in the window in front (the menu bar or tray menu's Notifications).
pub fn open_panel(cx: &mut App) {
    if let Some(bell) = cx
        .try_global::<BellGlobal>()
        .and_then(|bell| bell.0.upgrade())
    {
        bell.update(cx, |bell, cx| bell.set_open(true, cx));
    }
}

/// The bell's accessible name: the count of unread notifications, when there are any.
pub fn bell_label(unread: usize) -> String {
    match unread {
        0 => "Notifications".into(),
        1 => "Notifications, 1 unread".into(),
        n => format!("Notifications, {n} unread"),
    }
}

impl Render for Bell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let unread = unread(cx);
        let palette = *cx.global::<Palette>();
        let bell = cx.entity().downgrade();
        let popover = Popover::new("notifications")
            .anchor(Anchor::TopRight)
            .open(self.open)
            .on_open_change({
                let bell = bell.clone();
                move |open, _, cx| {
                    let open = *open;
                    bell.update(cx, |bell, cx| bell.set_open(open, cx)).ok();
                }
            })
            .trigger(
                Button::new("notifications-button")
                    .ghost()
                    .small()
                    .icon(icon("bell"))
                    .accessibility_label(bell_label(unread))
                    .tooltip("Notifications"),
            )
            .content(move |_, _, cx| panel(bell.clone(), cx));
        crate::shell::over_title_bar(div().relative().child(popover).when(unread > 0, |bell| {
            bell.child(
                div()
                    .id("notifications-unread")
                    .test_support()
                    .absolute()
                    .top(px(3.))
                    .right(px(3.))
                    .size(px(8.))
                    .rounded_full()
                    .border_1()
                    .border_color(cx.theme().background)
                    .bg(palette.primary),
            )
        }))
    }
}

/// Today's notifications and the earlier ones, each newest first.
fn grouped(
    entries: &[NotificationEntry],
    now: f64,
) -> [(&'static str, Vec<&NotificationEntry>); 2] {
    let day = |at: f64| crate::dates::local_format(at, "%Y-%m-%d");
    let today = day(now);
    let (recent, earlier) = entries.iter().partition(|entry| day(entry.at) == today);
    [("Today", recent), ("Earlier", earlier)]
}

/// The challenges that can still be answered, by id.
pub fn standing(cx: &App) -> Vec<String> {
    crate::online::handle(cx)
        .read(cx)
        .challenges
        .iter()
        .map(|challenge| challenge.id.clone())
        .collect()
}

/// Whether `link` is a challenge gone since (answered, withdrawn or expired).
pub fn expired(link: &NotificationLink, standing: &[String]) -> bool {
    matches!(link, NotificationLink::Challenge { id } if !standing.contains(id))
}

/// The line under a notification's title: its body, after "Expired" for a challenge gone since.
fn detail(body: &str, expired: bool) -> String {
    match (expired, body.is_empty()) {
        (true, true) => "Expired".into(),
        (true, false) => format!("Expired · {body}"),
        (false, _) => body.into(),
    }
}

fn panel(bell: WeakEntity<Bell>, cx: &mut App) -> AnyElement {
    let centre = handle(cx);
    let entries = centre.read(cx).entries().to_vec();
    let unread = centre.read(cx).unread();
    let palette = *cx.global::<Palette>();
    let now = kchess_games::now_ms();
    let standing = standing(cx);
    let header = h_flex()
        .gap(px(4.))
        .px(px(8.))
        .pb(px(4.))
        .child(
            div()
                .flex_1()
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Notifications"),
        )
        .when(unread > 0, |header| {
            header.child(
                crate::widgets::text_button("notifications-mark-read")
                    .label("Mark all read")
                    .on_click(|_, _, cx| mark_read(None, cx)),
            )
        })
        .when(!entries.is_empty(), |header| {
            header.child(
                crate::widgets::text_button("notifications-clear")
                    .label("Clear all")
                    .on_click(|_, _, cx| clear(None, cx)),
            )
        });
    let mut list = v_flex()
        .id("notifications-list")
        .gap(px(2.))
        .overflow_y_scroll()
        .max_h(PANEL_HEIGHT);
    if entries.is_empty() {
        list = list.child(
            div()
                .id("notifications-empty")
                .test_support()
                .py(px(24.))
                .text_center()
                .text_size(px(14.))
                .text_color(palette.text_muted)
                .child("No notifications"),
        );
    }
    for (heading, group) in grouped(&entries, now) {
        if group.is_empty() {
            continue;
        }
        list = list.child(
            div()
                .px(px(8.))
                .pt(px(8.))
                .pb(px(2.))
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(heading),
        );
        for entry in group {
            let expired = expired(&entry.link, &standing);
            list = list.child(row(entry, expired, now, bell.clone(), &palette, cx));
        }
    }
    v_flex()
        .id("notifications-panel")
        .test_support()
        .w(PANEL_WIDTH)
        .py(px(4.))
        .child(header)
        .child(list)
        .into_any_element()
}

/// Each link's icon.
fn glyph(link: &NotificationLink) -> &'static str {
    match link {
        NotificationLink::None => "bell",
        NotificationLink::Challenge { .. } => "swords",
        NotificationLink::Correspondence { .. } => "mail",
        NotificationLink::LiveGame => "swords",
        NotificationLink::ComputerGame => "cpu",
        NotificationLink::BroadcastRound { .. } => "radio",
        NotificationLink::Update { .. } => "download",
    }
}

/// One notification: its icon, its line and when, a dot while unread, and Dismiss. A challenge
/// gone since shows as expired, and opening it shows the challenges as they stand now.
fn row(
    entry: &NotificationEntry,
    expired: bool,
    now: f64,
    bell: WeakEntity<Bell>,
    palette: &Palette,
    cx: &App,
) -> impl IntoElement {
    let id = entry.id.clone();
    let open = entry.clone();
    let when = crate::widgets::time_ago(entry.at, now);
    let hovered = cx.theme().accent;
    let detail = detail(&entry.body, expired);
    h_flex()
        .id(SharedString::from(format!("notification-{}", entry.id)))
        .test_support()
        .group("notification")
        .items_start()
        .gap(px(10.))
        .px(px(8.))
        .py(px(6.))
        .rounded(cx.theme().radius)
        .cursor_pointer()
        .hover(move |row| row.bg(hovered))
        .on_click(move |_, window, cx| {
            bell.update(cx, |bell, cx| bell.set_open(false, cx)).ok();
            go(&open, window, cx);
        })
        .child(
            icon(glyph(&entry.link))
                .size(px(16.))
                .mt(px(2.))
                .flex_none()
                .text_color(if entry.read {
                    palette.text_muted
                } else {
                    palette.text
                }),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(14.))
                        .when(!entry.read, |title| title.font_weight(FontWeight::SEMIBOLD))
                        .child(entry.title.clone()),
                )
                .when(!detail.is_empty(), |text| {
                    text.child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .truncate()
                            .child(detail),
                    )
                })
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(palette.dimmed)
                        .child(when),
                ),
        )
        .child(
            h_flex()
                .flex_none()
                .gap(px(4.))
                .when(!entry.read, |end| {
                    end.child(div().size(px(6.)).rounded_full().bg(palette.primary))
                })
                .child(
                    Button::new(SharedString::from(format!(
                        "notification-dismiss-{}",
                        entry.id
                    )))
                    .ghost()
                    .small()
                    .icon(icon("x"))
                    .accessibility_label("Dismiss")
                    .tooltip("Dismiss")
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        clear(Some(id.clone()), cx)
                    }),
                ),
        )
}

/// Go where `entry` points, and mark it read.
pub fn go(entry: &NotificationEntry, window: &mut Window, cx: &mut App) {
    mark_read(Some(entry.id.clone()), cx);
    match &entry.link {
        NotificationLink::None => {}
        NotificationLink::Challenge { .. } | NotificationLink::LiveGame => {
            crate::shell::open(Page::Online, window, cx)
        }
        NotificationLink::ComputerGame => crate::shell::open(Page::Computer, window, cx),
        NotificationLink::Correspondence { account, game } => {
            crate::shell::open(Page::Online, window, cx);
            let (account, game) = (account.clone(), game.clone());
            crate::online::handle(cx).update(cx, |online, cx| online.open(&account, &game, cx));
        }
        NotificationLink::BroadcastRound { round, game } => crate::pages::watch::open_broadcast(
            crate::game::BroadcastLink::Round {
                round: round.clone(),
                game: game.clone(),
            },
            window,
            cx,
        ),
        NotificationLink::Update { .. } => {
            crate::shell::open_settings(crate::pages::settings::Section::Updates, window, cx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{bell_label, detail, expired};
    use kchess_contracts::types::NotificationLink;

    #[test]
    fn the_bell_says_how_many_are_unread() {
        assert_eq!(bell_label(0), "Notifications");
        assert_eq!(bell_label(1), "Notifications, 1 unread");
        assert_eq!(bell_label(4), "Notifications, 4 unread");
    }

    #[test]
    fn a_challenge_gone_since_reads_as_expired() {
        let challenge = NotificationLink::Challenge { id: "abc".into() };
        assert!(!expired(&challenge, &["abc".into()]));
        assert!(expired(&challenge, &[]));
        assert!(!expired(&NotificationLink::LiveGame, &[]));
        assert_eq!(detail("5+3 rated", true), "Expired · 5+3 rated");
        assert_eq!(detail("", true), "Expired");
        assert_eq!(detail("5+3 rated", false), "5+3 rated");
    }
}
