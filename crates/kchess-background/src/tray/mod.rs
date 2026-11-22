//! KChess's icon in the menu bar (macOS) or the system tray (Windows, Linux), and its menu.
//!
//! The menu is data (`Item`), rebuilt whole whenever what it shows changes. What the person
//! picks comes back as an `Event` through the callback given to `Tray::new`, which may run on
//! another thread (Linux's D-Bus service has its own): the app forwards it to its main thread.

use std::sync::Arc;

#[cfg_attr(target_os = "macos", path = "macos.rs")]
#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(
    not(any(target_os = "macos", windows, target_os = "linux")),
    path = "none.rs"
)]
mod platform;

/// One entry of the menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// An entry to pick, reported as `Event::Chose(id)`; `checked` shows a check mark beside it
    /// when `Some(true)`.
    Action {
        id: u32,
        label: String,
        enabled: bool,
        checked: Option<bool>,
    },
    /// A label with entries of its own.
    Submenu {
        label: String,
        items: Vec<Item>,
    },
    Separator,
}

impl Item {
    /// An entry to pick.
    pub fn action(id: u32, label: impl Into<String>) -> Item {
        Item::Action {
            id,
            label: label.into(),
            enabled: true,
            checked: None,
        }
    }

    /// An entry with a check mark when `on`.
    pub fn check(id: u32, label: impl Into<String>, on: bool) -> Item {
        Item::Action {
            id,
            label: label.into(),
            enabled: true,
            checked: Some(on),
        }
    }

    /// A line of text that cannot be picked.
    pub fn note(label: impl Into<String>) -> Item {
        Item::Action {
            id: 0,
            label: label.into(),
            enabled: false,
            checked: None,
        }
    }
}

/// What the person did with the icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Clicked the icon itself where that opens the app (Windows, and Linux trays that report
    /// it); the menu opens on macOS instead.
    Open,
    /// Picked the menu entry with this id.
    Chose(u32),
}

/// Where events go: called on the thread the system reports on.
pub type Events = Arc<dyn Fn(Event) + Send + Sync>;

/// How the icon is drawn.
pub struct Icon {
    /// macOS: a template image, black on transparent, which the menu bar tints.
    pub template_png: &'static [u8],
    /// Linux: the app's icon in colour.
    pub color_png: &'static [u8],
}

/// The icon, shown until it is dropped. Not `Send` where the system needs the main thread.
pub struct Tray(platform::Tray);

impl Tray {
    /// Show the icon with `menu`, named `tooltip`. Fails when the system has no place for it (a
    /// Linux desktop without a tray), so the app knows nothing shows it is running.
    pub fn new(
        tooltip: &str,
        icon: &Icon,
        menu: Vec<Item>,
        events: Events,
    ) -> Result<Tray, String> {
        platform::Tray::new(tooltip, icon, menu, events).map(Tray)
    }

    /// Show `menu` from now on.
    pub fn set_menu(&mut self, menu: Vec<Item>) {
        self.0.set_menu(menu);
    }
}

/// Whether this system shows a tray icon at all: false where `Tray::new` always fails.
pub const SUPPORTED: bool = cfg!(any(target_os = "macos", windows, target_os = "linux"));

/// The ids of `menu`'s entries that can be picked, in order: what a click can report.
pub fn ids(menu: &[Item]) -> Vec<u32> {
    let mut found = Vec::new();
    for item in menu {
        match item {
            Item::Action {
                id, enabled: true, ..
            } => found.push(*id),
            Item::Submenu { items, .. } => found.extend(ids(items)),
            _ => {}
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_entries_that_can_be_picked_report_an_id() {
        let menu = vec![
            Item::action(1, "Open KChess"),
            Item::Separator,
            Item::note("Nothing waiting"),
            Item::Submenu {
                label: "Challenge".into(),
                items: vec![Item::action(7, "Accept"), Item::action(8, "Decline")],
            },
            Item::check(3, "Do not disturb", true),
        ];
        assert_eq!(ids(&menu), [1, 7, 8, 3]);
    }
}
