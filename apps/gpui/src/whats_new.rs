//! What's new: each release's notes (`WHATS_NEW.md`, built into the app), shown once on the first
//! start after an update and whenever asked for from the sidebar's
//! account menu, Settings → Updates, Quick Pick or the menu.
//!
//! The version last shown is a view preference (`kchess:whats-new-seen`), so a profile shared by
//! several installs shows a release's notes once. A fresh profile starts with nothing to catch up
//! on; a profile from before this preference existed gets the running release's notes.

use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::Palette;
use crate::updates::parse_version;

/// The notes, newest release first, with the unreleased section that `release-version` dates.
const NOTES: &str = include_str!("../WHATS_NEW.md");
/// The release whose notes were last shown on this profile.
pub const SEEN_KEY: &str = "kchess:whats-new-seen";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    /// `YYYY-MM-DD`.
    pub date: Option<String>,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub title: Option<String>,
    pub text: String,
}

/// The released sections of `text`, in its order: `## 2026.11.4 (2026-11-20)` headings, each with
/// `- **Title**: text` items. The `## Unreleased` section and anything before the first heading
/// are left out.
pub fn parse(text: &str) -> Vec<Release> {
    sections(text)
        .into_iter()
        .filter(|release| parse_version(&release.version).is_some())
        .collect()
}

/// Every `## ` section of `text` with its items, `Unreleased` among them as a version of that name.
fn sections(text: &str) -> Vec<Release> {
    let mut releases: Vec<Release> = Vec::new();
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            let (version, date) = match heading.trim().split_once(' ') {
                Some((version, date)) => (
                    version,
                    Some(date.trim().trim_matches(['(', ')']).to_string()),
                ),
                None => (heading.trim(), None),
            };
            releases.push(Release {
                version: version.to_string(),
                date: date.filter(|date| !date.is_empty()),
                items: Vec::new(),
            });
            continue;
        }
        let Some(release) = releases.last_mut() else {
            continue;
        };
        let trimmed = line.trim();
        if let Some(item) = trimmed.strip_prefix("- ") {
            release.items.push(item_of(item));
        } else if !trimmed.is_empty()
            && line.starts_with(' ')
            && let Some(item) = release.items.last_mut()
        {
            // A wrapped item carries on, indented.
            item.text.push(' ');
            item.text.push_str(&plain(trimmed));
        }
    }
    releases
}

fn item_of(text: &str) -> Item {
    if let Some(rest) = text.strip_prefix("**")
        && let Some((title, text)) = rest.split_once("**")
    {
        let text = text.trim_start_matches([':', '.']).trim();
        return Item {
            title: Some(plain(title.trim_end_matches(':'))),
            text: plain(text),
        };
    }
    Item {
        title: None,
        text: plain(text),
    }
}

/// Markdown's emphasis and code marks dropped, links as their text.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let Some((label, after)) = rest[start + 1..].split_once("](") else {
            break;
        };
        let Some(end) = after.find(')') else {
            break;
        };
        out.push_str(&rest[..start]);
        out.push_str(label);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out.replace("**", "").replace('`', "")
}

/// Every release up to `current`, newest first.
pub fn releases_up_to(text: &str, current: &str) -> Vec<Release> {
    let current = parse_version(current);
    let mut releases: Vec<Release> = parse(text)
        .into_iter()
        .filter(|release| parse_version(&release.version) <= current)
        .collect();
    releases.sort_by_key(|release| std::cmp::Reverse(parse_version(&release.version)));
    releases
}

/// What to show at startup, from the release last shown (`seen`) and whether the profile was in use
/// before (any view preference saved): the releases after `seen` up to `current`, or for a profile
/// that never recorded one, the running release's alone. A fresh profile is shown nothing.
pub fn to_show(text: &str, current: &str, seen: Option<&str>, used: bool) -> Vec<Release> {
    let all = releases_up_to(text, current);
    match seen.map(parse_version) {
        Some(Some(seen)) => all
            .into_iter()
            .filter(|release| parse_version(&release.version).is_some_and(|v| v > seen))
            .collect(),
        // An unreadable mark counts as an old profile's.
        Some(None) | None if used || seen.is_some() => all
            .into_iter()
            .filter(|release| release.version == current)
            .collect(),
        _ => Vec::new(),
    }
}

/// Every release's notes up to the running one, newest first.
pub fn releases() -> Vec<Release> {
    releases_up_to(NOTES, &crate::updates::current_version())
}

/// The first start after an update: show what it brought, once. Waits for a later start while a
/// Lichess game is on, so nothing covers its board.
pub fn on_start(window: &mut Window, cx: &mut App) {
    let current = crate::updates::current_version();
    let seen: Option<String> = crate::prefs::get(cx, SEEN_KEY, None);
    if seen.as_deref() == Some(current.as_str()) {
        return;
    }
    let used = crate::prefs::used(cx);
    let shown = to_show(NOTES, &current, seen.as_deref(), used);
    if !shown.is_empty() {
        let live = cx
            .try_global::<crate::online::OnlineHandle>()
            .is_some_and(|_| crate::online::handle(cx).read(cx).live_game_on());
        if live {
            return;
        }
        open_releases(shown, window, cx);
    }
    crate::prefs::set(cx, SEEN_KEY, current);
}

/// Every release's notes, from the account menu, Settings → Updates, Quick Pick and the menu.
pub fn open(window: &mut Window, cx: &mut App) {
    open_releases(releases(), window, cx);
}

fn open_releases(releases: Vec<Release>, window: &mut Window, cx: &mut App) {
    crate::widgets::open_dialog(window, cx, move |dialog, window, cx| {
        let palette = *cx.global::<Palette>();
        // A short window scrolls the notes rather than cut them off.
        let room = (window.viewport_size().height - px(180.)).max(px(160.));
        // Wide enough that most notes take a line or two, inside a narrow window's margins.
        let width = (window.viewport_size().width - px(64.)).clamp(px(320.), px(640.));
        dialog
            .title("What's new")
            .w(width)
            .child(list(&releases, &palette, room))
    });
}

/// `Nov 4, 2026` from `2026-11-04`.
fn date_label(date: &str) -> Option<String> {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .map(|date| date.format("%b %-d, %Y").to_string())
}

fn list(releases: &[Release], palette: &Palette, height: Pixels) -> impl IntoElement {
    v_flex()
        .id("whats-new")
        .max_h(height)
        .overflow_y_scroll()
        .gap(px(16.))
        .when(releases.is_empty(), |list| {
            list.child(
                div()
                    .text_size(px(13.))
                    .text_color(palette.text_muted)
                    .child("No release notes in this build"),
            )
        })
        .children(releases.iter().map(|release| {
            v_flex()
                .gap(px(6.))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.highlighted)
                                .child(format!("KChess {}", release.version)),
                        )
                        .children(release.date.as_deref().and_then(date_label).map(|date| {
                            div()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(date)
                        })),
                )
                .children(release.items.iter().map(|item| {
                    // The title leads its line in bold, wrapping with the text after it.
                    let lead = item.title.as_ref().map_or(0, |title| title.len() + 2);
                    let line = match &item.title {
                        Some(title) => format!("{title}. {}", item.text),
                        None => item.text.clone(),
                    };
                    let bold = HighlightStyle {
                        font_weight: Some(FontWeight::MEDIUM),
                        ..HighlightStyle::default()
                    };
                    div().text_size(px(13.)).text_color(palette.text).child(
                        StyledText::new(line)
                            .with_highlights((lead > 0).then_some((0..lead, bold))),
                    )
                }))
        }))
        .test_support()
}

#[cfg(test)]
mod tests {
    use super::{Item, NOTES, date_label, parse, parse_version, releases_up_to, sections, to_show};

    const SAMPLE: &str = "# What's new\n\nIntro text.\n\n## Unreleased\n\n- **Next**: not yet.\n\n\
        ## 2026.11.4 (2026-11-20)\n\n- **Guess the move**: a drill, see [Practice](https://x).\n  \
        It carries on.\n- A plain `fix`.\n\n## 2026.11.3 (2026-11-04)\n\n- **Older**: before.\n";

    #[test]
    fn the_notes_read_their_releases_and_items() {
        let releases = parse(SAMPLE);
        assert_eq!(releases.len(), 2, "Unreleased is left out");
        assert_eq!(releases[0].version, "2026.11.4");
        assert_eq!(releases[0].date.as_deref(), Some("2026-11-20"));
        assert_eq!(
            releases[0].items,
            [
                Item {
                    title: Some("Guess the move".into()),
                    text: "a drill, see Practice. It carries on.".into(),
                },
                Item {
                    title: None,
                    text: "A plain fix.".into(),
                },
            ]
        );
        assert_eq!(date_label("2026-11-04").as_deref(), Some("Nov 4, 2026"));
    }

    /// Only releases this build has reached are listed: a newer section is a later build's.
    #[test]
    fn a_build_lists_the_releases_up_to_its_own() {
        let versions = |current| {
            releases_up_to(SAMPLE, current)
                .into_iter()
                .map(|release| release.version)
                .collect::<Vec<_>>()
        };
        assert_eq!(versions("2026.11.4"), ["2026.11.4", "2026.11.3"]);
        assert_eq!(versions("2026.11.3"), ["2026.11.3"]);
    }

    #[test]
    fn an_update_shows_what_it_brought_once() {
        let shown = |current, seen, used| {
            to_show(SAMPLE, current, seen, used)
                .into_iter()
                .map(|release| release.version)
                .collect::<Vec<_>>()
        };
        // Updated from 11.3: the releases since.
        assert_eq!(shown("2026.11.4", Some("2026.11.3"), true), ["2026.11.4"]);
        // Several releases skipped: every one of them.
        assert_eq!(
            shown("2026.11.4", Some("2026.11.1"), true),
            ["2026.11.4", "2026.11.3"]
        );
        // Already seen, or moved back to an older release: nothing.
        assert!(shown("2026.11.4", Some("2026.11.4"), true).is_empty());
        assert!(shown("2026.11.3", Some("2026.11.4"), true).is_empty());
        // A profile from before the mark existed: the running release's notes.
        assert_eq!(shown("2026.11.4", None, true), ["2026.11.4"]);
        // A fresh profile: nothing to catch up on.
        assert!(shown("2026.11.4", None, false).is_empty());
    }

    /// The notes built into the app read: every released section has a date and items with
    /// titles, versions descend, and the running release has its section.
    #[test]
    fn the_shipped_notes_are_well_formed() {
        // A player reads a line at a glance: a longer one is skipped, however much it holds.
        const MAX_ITEM: usize = 280;
        for release in sections(NOTES) {
            for item in &release.items {
                let title = item.title.as_deref().unwrap_or_default();
                let length = title.chars().count() + item.text.chars().count();
                assert!(
                    length <= MAX_ITEM,
                    "{} has a {length}-character line, over {MAX_ITEM}: keep the few points a \
                     player notices most. {title}: {}",
                    release.version,
                    item.text
                );
            }
        }
        let all = parse(NOTES);
        assert!(!all.is_empty());
        for release in &all {
            assert!(
                release.date.as_deref().and_then(date_label).is_some(),
                "{} has a date",
                release.version
            );
            assert!(!release.items.is_empty(), "{} has items", release.version);
            for item in &release.items {
                assert!(item.title.is_some(), "{}: {}", release.version, item.text);
                assert!(!item.text.is_empty(), "{}", release.version);
            }
        }
        let versions: Vec<_> = all.iter().map(|r| parse_version(&r.version)).collect();
        assert!(versions.windows(2).all(|pair| pair[0] > pair[1]));
        let current = crate::updates::current_version();
        assert!(
            all.iter().any(|release| release.version == current),
            "WHATS_NEW.md has a section for {current}"
        );
    }
}
