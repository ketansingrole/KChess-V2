#[test]
fn a_count_of_one_takes_the_singular() {
    use super::format_counted;
    assert_eq!(format_counted(1., "request", "requests"), "1 request");
    assert_eq!(format_counted(0., "game", "games"), "0 games");
    assert_eq!(
        format_counted(2048., "request", "requests"),
        "2,048 requests"
    );
}

/// GPUI announces nothing by itself: a move or a result was heard only when the reader went
/// back to the board. Each adapter speaks a live node's text when it changes: macOS its value,
/// Windows and Linux its name.
#[test]
fn a_live_region_is_spoken_on_every_platform() {
    use gpui_kit::{Element as _, accesskit};
    let region = super::live_region("live", "Last move e2–e4 · Black to move");
    assert_eq!(region.a11y_role(), Some(gpui_kit::Role::Status));
    let mut node = accesskit::Node::new(gpui_kit::Role::Status);
    region.write_a11y_info(&mut node);
    assert_eq!(node.live(), Some(accesskit::Live::Polite));
    assert_eq!(node.value(), Some("Last move e2–e4 · Black to move"));
    assert_eq!(node.label(), Some("Last move e2–e4 · Black to move"));
}

/// A clock turns red only in its last ten seconds, when it shows tenths.
#[test]
fn only_the_last_ten_seconds_are_low() {
    assert!(super::low_time("0:09.4"));
    assert!(super::low_time("0:00.0"));
    assert!(!super::low_time("0:10"));
    assert!(!super::low_time("1:05"));
    assert!(!super::low_time("1:00:05"));
    assert!(!super::low_time("2d 4h"));
}

/// A list of one page shows that page alone, with no gaps.
#[test]
fn a_single_page_shows_only_itself() {
    use super::{PagerSlot::*, pager_slots};
    assert_eq!(pager_slots(0, 1), vec![Page(0)]);
    // A current page past the end is the last one.
    assert_eq!(pager_slots(7, 1), vec![Page(0)]);
    assert!(pager_slots(0, 0).is_empty());
}

/// A few pages are each shown while they fit beside the current one; a page left out leaves a gap.
#[test]
fn a_few_pages_show_the_ends_and_the_neighbours() {
    use super::{PagerSlot::*, pager_slots};
    assert_eq!(pager_slots(0, 3), vec![Page(0), Page(1), Page(2)]);
    assert_eq!(pager_slots(1, 4), vec![Page(0), Page(1), Page(2), Page(3)]);
    assert_eq!(pager_slots(0, 5), vec![Page(0), Page(1), Gap, Page(4)]);
    assert_eq!(pager_slots(4, 5), vec![Page(0), Gap, Page(3), Page(4)]);
    assert_eq!(
        pager_slots(2, 5),
        vec![Page(0), Page(1), Page(2), Page(3), Page(4)]
    );
    assert_eq!(
        pager_slots(3, 7),
        vec![Page(0), Gap, Page(2), Page(3), Page(4), Gap, Page(6)]
    );
}

/// Thousands of pages keep one short row: the ends, the current page and its neighbours.
#[test]
fn thousands_of_pages_keep_a_short_row() {
    use super::{PagerSlot::*, pager_slots};
    let pages = 225_617;
    assert_eq!(
        pager_slots(0, pages),
        vec![Page(0), Page(1), Gap, Page(pages - 1)]
    );
    assert_eq!(
        pager_slots(4_000, pages),
        vec![
            Page(0),
            Gap,
            Page(3_999),
            Page(4_000),
            Page(4_001),
            Gap,
            Page(pages - 1)
        ]
    );
    assert_eq!(
        pager_slots(pages - 1, pages),
        vec![Page(0), Gap, Page(pages - 2), Page(pages - 1)]
    );
    assert!(pager_slots(usize::MAX - 1, usize::MAX).len() <= 7);
}

/// The range line counts from 1 and stops at the last game.
#[test]
fn the_range_says_which_games_show() {
    use super::page_range;
    assert_eq!(page_range(0, 20, 0), "0 games");
    assert_eq!(page_range(0, 20, 1), "1–1 of 1 game");
    assert_eq!(page_range(1, 20, 45), "21–40 of 45 games");
    assert_eq!(page_range(2, 20, 45), "41–45 of 45 games");
    assert_eq!(
        page_range(50, 20, 4_512_331),
        "1,001–1,020 of 4,512,331 games"
    );
}
