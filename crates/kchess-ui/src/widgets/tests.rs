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
