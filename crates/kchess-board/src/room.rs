//! Room on the disk before a download: the profile's free space is read when the download is
//! asked for, and one that would leave the disk almost full asks first. The game databases, the
//! puzzles, the speech model and Stockfish all go through it.

use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::*;
use kchess_app::log::ResultExt as _;
use kchess_contracts::calls;

use crate::core::AppCore;
use crate::widgets;

/// What a download must leave free, so the computer and its other programs keep working.
const KEEP_FREE: u64 = 2 * 1024 * 1024 * 1024;

/// The bytes free on the profile's disk when they are too few for something that takes `takes`
/// bytes (unknown: none) and `KEEP_FREE` besides; `None` when there is room or the system did
/// not say.
pub fn short_of_room(free: Option<u64>, takes: Option<u64>) -> Option<u64> {
    let free = free?;
    let need = takes.unwrap_or(0).saturating_add(KEEP_FREE);
    (free < need).then_some(free)
}

/// What the warning says: the room left, and what this takes when that is known.
fn room_left(free: u64, takes: Option<u64>) -> String {
    let free = widgets::format_bytes(free as f64);
    match takes {
        Some(takes) => format!(
            "{free} is free. This could take about {}.",
            widgets::format_bytes(takes as f64)
        ),
        None => format!("{free} is free."),
    }
}

/// The free space a test pins, whatever the disk running it says.
#[cfg(any(test, feature = "test-support"))]
pub struct PinnedFreeSpace(pub u64);

#[cfg(any(test, feature = "test-support"))]
impl Global for PinnedFreeSpace {}

/// Run `start` when the disk has room for `takes` bytes more (`short_of_room`), its free space
/// read now; otherwise only once the person says to `go` ahead anyway ("Download anyway").
pub fn with_room(
    takes: Option<u64>,
    go: &'static str,
    window: &mut Window,
    cx: &mut App,
    start: impl Fn(&mut Window, &mut App) + 'static,
) {
    let asked = cx.global::<AppCore>().request(calls::FreeSpace {});
    window
        .spawn(cx, async move |cx| {
            // A disk that cannot be read is not called full: the download goes ahead.
            let free = asked.await.warn_on_err().flatten();
            cx.update(|window, cx| {
                #[cfg(any(test, feature = "test-support"))]
                let free = cx
                    .try_global::<PinnedFreeSpace>()
                    .map_or(free, |pinned| Some(pinned.0));
                match short_of_room(free, takes) {
                    None => start(window, cx),
                    Some(free) => ask_for_room(room_left(free, takes), go, window, cx, start),
                }
            })
            .ok();
        })
        .detach();
}

/// Ask before a download or import the disk may have no room for, saying `description`.
fn ask_for_room(
    description: String,
    go: &'static str,
    window: &mut Window,
    cx: &mut App,
    start: impl Fn(&mut Window, &mut App) + 'static,
) {
    let start = std::rc::Rc::new(start);
    widgets::open_alert_dialog(window, cx, move |dialog: AlertDialog, _, _| {
        let start = start.clone();
        dialog
            .title("Your disk is almost full")
            .description(description.clone())
            .ok_text(go)
            .ok_variant(ButtonVariant::Danger)
            .cancel_text("Cancel")
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                start(window, cx);
                true
            })
    });
}

#[cfg(test)]
mod tests {
    use super::{KEEP_FREE, room_left, short_of_room};

    /// The warning names the room left, and what this takes when that is known.
    #[test]
    fn the_warning_says_what_is_free_and_what_it_takes() {
        let gb = 1024 * 1024 * 1024;
        assert_eq!(
            room_left(gb, Some(3 * gb)),
            "1.0 GB is free. This could take about 3.0 GB."
        );
        assert_eq!(room_left(gb, None), "1.0 GB is free.");
    }

    /// A download needs its own room and `KEEP_FREE` besides; a disk the system says nothing
    /// about is never called full.
    #[test]
    fn a_download_is_short_of_room_below_what_it_takes_and_a_margin() {
        let takes = 3_000_000;
        let need = takes + KEEP_FREE;
        assert_eq!(short_of_room(Some(need - 1), Some(takes)), Some(need - 1));
        assert_eq!(short_of_room(Some(need), Some(takes)), None);
        assert_eq!(
            short_of_room(Some(KEEP_FREE - 1), None),
            Some(KEEP_FREE - 1)
        );
        assert_eq!(short_of_room(Some(KEEP_FREE), None), None);
        assert_eq!(short_of_room(None, Some(u64::MAX)), None);
    }
}
