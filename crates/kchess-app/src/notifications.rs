//! The notification centre's list: the notifications KChess showed on this profile, newest
//! first, as the core keeps them (`notifications`). Every window and the command line read the
//! same list, and a change made in one reaches the others (`profile:changed`).
//!
//! `announce` is the one way to alert the player: the app decides whether and how to show it
//! (its Settings switches, the window in front) and keeps what it showed here.

use gpui_kit::{App, AppContext as _, Context, Entity, Global};
use kchess_contracts::calls;
use kchess_contracts::check::NotificationId;
use kchess_contracts::error::CoreError;
use kchess_contracts::types::{
    NotificationEntry, NotificationInput, NotificationKind, NotificationLink, ProfileArea,
};

use crate::core::AppCore;
use crate::loading::Epoch;
use crate::log::ResultExt as _;

/// An alert for the player: what it says, where it leads, and which Settings switch allows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Announcement {
    pub kind: NotificationKind,
    pub title: String,
    pub body: String,
    pub link: NotificationLink,
    /// The same alert announced again has the same key (`NotificationInput::key`).
    pub key: Option<String>,
}

/// How the app shows an announcement and records it (`set_announcer`).
type Announcer = fn(Announcement, &mut App);

struct AnnouncerGlobal(Announcer);

impl Global for AnnouncerGlobal {}

/// Name how announcements are shown: the app's alerts (`apps/gpui/src/notifications`).
pub fn set_announcer(announcer: Announcer, cx: &mut App) {
    cx.set_global(AnnouncerGlobal(announcer));
}

/// Alert the player, as Settings allow, and keep the alert in the notification centre. Without
/// an app to show it (a test of one crate), it is kept unread.
pub fn announce(announcement: Announcement, cx: &mut App) {
    match cx.try_global::<AnnouncerGlobal>() {
        Some(AnnouncerGlobal(announcer)) => announcer(announcement, cx),
        None => record(input(announcement, false), cx),
    }
}

/// What to keep of `announcement`; `read` when the player saw it arrive.
pub fn input(announcement: Announcement, read: bool) -> NotificationInput {
    NotificationInput {
        title: announcement.title,
        body: announcement.body,
        link: announcement.link,
        read,
        key: announcement.key,
    }
}

pub struct NotificationCentre {
    /// Newest first.
    entries: Vec<NotificationEntry>,
    /// The latest request: only its answer replaces `entries`, since each answers the whole list
    /// after its change.
    epoch: Epoch,
}

impl NotificationCentre {
    pub fn entries(&self) -> &[NotificationEntry] {
        &self.entries
    }

    pub fn unread(&self) -> usize {
        self.entries.iter().filter(|entry| !entry.read).count()
    }

    /// Send `call` and show the list it answers, unless a later request was sent meanwhile.
    fn send<C>(&mut self, call: C, cx: &mut Context<Self>)
    where
        C: kchess_contracts::calls::CoreCall<Answer = Vec<NotificationEntry>> + Send + 'static,
    {
        let turn = self.epoch.start();
        let answer = cx.global::<AppCore>().request(call);
        cx.spawn(async move |this, cx| {
            let answer: Result<Vec<NotificationEntry>, CoreError> = answer.await;
            let Some(entries) = answer.log_err() else {
                return;
            };
            this.update(cx, |this, cx| {
                if this.epoch.is_current(turn) && this.entries != entries {
                    this.entries = entries;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Read the list again: another window or the command line changed it.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.send(calls::Notifications {}, cx);
    }

    pub fn record(&mut self, input: NotificationInput, cx: &mut Context<Self>) {
        self.send(calls::RecordNotification { entry: input }, cx);
    }

    /// Mark `id` read, or every notification: shown at once, then as the core keeps it.
    pub fn mark_read(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        let mut changed = false;
        for entry in &mut self.entries {
            if !entry.read && id.as_ref().is_none_or(|id| &entry.id == id) {
                entry.read = true;
                changed = true;
            }
        }
        if !changed {
            return;
        }
        cx.notify();
        let id = id.map(NotificationId);
        self.send(calls::MarkNotificationsRead { id }, cx);
    }

    /// Remove `id`, or every notification: gone at once, then as the core keeps it.
    pub fn clear(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        let before = self.entries.len();
        self.entries
            .retain(|entry| id.as_ref().is_some_and(|id| &entry.id != id));
        if self.entries.len() == before {
            return;
        }
        cx.notify();
        let id = id.map(NotificationId);
        self.send(calls::ClearNotifications { id }, cx);
    }
}

/// How views reach the list: `handle(cx)`, observed to redraw when it changes.
#[derive(Clone)]
struct CentreHandle(Entity<NotificationCentre>);

impl Global for CentreHandle {}

/// Read the list kept, and again whenever another window or the command line changes it.
pub fn init(cx: &mut App) {
    let centre = cx.new(|_| NotificationCentre {
        entries: Vec::new(),
        epoch: Epoch::default(),
    });
    centre.update(cx, |centre, cx| centre.reload(cx));
    crate::core::on_profile_change(cx, &[ProfileArea::Notifications], {
        let centre = centre.downgrade();
        move |cx| {
            centre.update(cx, |centre, cx| centre.reload(cx)).ok();
        }
    });
    cx.set_global(CentreHandle(centre));
}

/// The list, made empty on first use where `init` has not run (a test of one crate).
pub fn handle(cx: &mut App) -> Entity<NotificationCentre> {
    if let Some(CentreHandle(centre)) = cx.try_global::<CentreHandle>() {
        return centre.clone();
    }
    let centre = cx.new(|_| NotificationCentre {
        entries: Vec::new(),
        epoch: Epoch::default(),
    });
    cx.set_global(CentreHandle(centre.clone()));
    centre
}

/// The list, where `init` ran.
pub fn try_handle(cx: &App) -> Option<Entity<NotificationCentre>> {
    cx.try_global::<CentreHandle>()
        .map(|handle| handle.0.clone())
}

/// How many notifications are unread.
pub fn unread(cx: &App) -> usize {
    try_handle(cx).map_or(0, |centre| centre.read(cx).unread())
}

/// Keep a notification the app showed.
pub fn record(input: NotificationInput, cx: &mut App) {
    handle(cx).update(cx, |centre, cx| centre.record(input, cx));
}

/// Mark `id` read, or every notification.
pub fn mark_read(id: Option<String>, cx: &mut App) {
    handle(cx).update(cx, |centre, cx| centre.mark_read(id, cx));
}

/// Remove `id`, or every notification.
pub fn clear(id: Option<String>, cx: &mut App) {
    handle(cx).update(cx, |centre, cx| centre.clear(id, cx));
}
