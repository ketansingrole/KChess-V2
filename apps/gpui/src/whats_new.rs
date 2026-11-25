//! What's new: each release's notes (`WHATS_NEW.md`, built into the app), shown once on the first
//! start after an update and whenever asked for from the sidebar's
//! account menu, Settings → Updates, Quick Pick or the menu; and the notes of the update about to
//! install (its GitHub release's text), before it does (`open_before_update`).
//!
//! A release reads as its summary, then its items under Added, Changed and Fixed, each with its
//! group's icon. A release from before the groups lists its titled items as it was published.
//!
//! The version last shown is a view preference (`kchess:whats-new-seen`), so a profile shared by
//! several installs shows a release's notes once. A fresh profile starts with nothing to catch up
//! on; a profile from before this preference existed gets the running release's notes.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::assets::icon;
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
    /// What the release is about, in a few sentences.
    pub summary: Option<String>,
    pub items: Vec<Item>,
}

impl Release {
    fn of(section: &kchess_delta::notes::Section) -> Release {
        Release {
            version: section.version.to_string(),
            date: section.date.map(str::to_string),
            summary: section.summary(),
            items: section.items(),
        }
    }

    /// Release `version`'s notes from its GitHub release's text (its section's Markdown,
    /// `kchess_delta::notes::Section::body`), released on `date` (`YYYY-MM-DD`).
    pub fn from_body(version: &str, date: Option<&str>, body: &str) -> Release {
        Release::of(&kchess_delta::notes::Section {
            version,
            date,
            lines: body.lines().collect(),
        })
    }

    /// Whether there is anything to read.
    pub fn is_empty(&self) -> bool {
        self.summary.is_none() && self.items.is_empty()
    }
}

pub use kchess_delta::notes::{Item, Kind};

/// The released sections of `text`, in its order: `## 2026.11.4 (2026-11-20)` headings, each with
/// its summary and items. The `## Unreleased` section and anything before the first heading are
/// left out.
pub fn parse(text: &str) -> Vec<Release> {
    kchess_delta::notes::sections(text)
        .into_iter()
        .filter(|section| section.release().is_some())
        .map(|section| Release::of(&section))
        .collect()
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

/// Every release's notes up to the running one, newest first. A development build shows the
/// unreleased notes first, as players will read them.
pub fn releases() -> Vec<Release> {
    let mut releases = releases_up_to(NOTES, &crate::updates::current_version());
    if cfg!(debug_assertions)
        && let Some(next) = unreleased(NOTES)
    {
        releases.insert(0, next);
    }
    releases
}

/// The `## Unreleased` notes, when there are any.
fn unreleased(text: &str) -> Option<Release> {
    kchess_delta::notes::section(text, kchess_delta::notes::UNRELEASED)
        .map(|section| Release::of(&section))
        .filter(|release| !release.is_empty())
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
        let (width, room) = room(window, px(180.));
        dialog
            .title("What's new")
            .w(width)
            .child(list(&releases, &palette, room))
    });
}

/// The notes of `release`, the update about to install, with what restarting would cost now
/// (`cost`): Restart and install puts it in place (`install`), Cancel leaves it for later, and
/// the box turns these notes off for later updates (`updateShowNotes`).
pub fn open_before_update(
    release: Release,
    cost: Option<&'static str>,
    install: impl Fn(&mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let install = std::rc::Rc::new(install);
    crate::widgets::open_dialog(window, cx, move |dialog, window, cx| {
        let palette = *cx.global::<Palette>();
        let (width, room) = room(window, px(240.));
        let skip = !crate::settings::frontend(cx).update_show_notes;
        let install = install.clone();
        let footer = h_flex()
            .w_full()
            .gap(px(8.))
            .child(
                Checkbox::new("whats-new-skip")
                    .checked(skip)
                    .label("Don't show what's new before updating")
                    .on_click(|checked, _, cx| {
                        let show = !*checked;
                        crate::settings::change(cx, move |s| s.frontend.update_show_notes = show);
                    }),
            )
            .child(div().flex_1())
            .child(
                Button::new("whats-new-cancel")
                    .outline()
                    .small()
                    .label("Cancel")
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
            .child(
                Button::new("whats-new-install")
                    .small()
                    .when(cost.is_some(), |button| button.danger())
                    .when(cost.is_none(), |button| button.primary())
                    .label("Restart and install")
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        install(cx);
                    }),
            );
        dialog
            .title("What's new")
            .w(width)
            .child(
                v_flex()
                    .gap(px(12.))
                    .child(list(std::slice::from_ref(&release), &palette, room))
                    .children(cost.map(|cost| {
                        div()
                            .id("whats-new-cost")
                            .test_support()
                            .aria_label(cost)
                            .text_size(px(13.))
                            .text_color(palette.warning)
                            .child(cost)
                    })),
            )
            .footer(footer)
    });
}

/// The dialog's width, and the height its notes may take before they scroll: a short window
/// scrolls them rather than cut them off, leaving `chrome` for the title and buttons.
fn room(window: &Window, chrome: Pixels) -> (Pixels, Pixels) {
    let viewport = window.viewport_size();
    // Wide enough that most notes take a line or two, inside a narrow window's margins.
    let width = (viewport.width - px(64.)).clamp(px(320.), px(640.));
    (width, (viewport.height - chrome).max(px(160.)))
}

/// `Nov 4, 2026` from `2026-11-04`.
fn date_label(date: &str) -> Option<String> {
    jiff::civil::Date::strptime("%Y-%m-%d", date)
        .ok()
        .map(|date| date.strftime("%b %-d, %Y").to_string())
}

/// A group's icon and its colour.
fn look(kind: Kind, palette: &Palette) -> (&'static str, Hsla) {
    match kind {
        Kind::Added => ("circle-plus", palette.success),
        Kind::Changed => ("sliders-horizontal", palette.primary),
        Kind::Fixed => ("circle-check", palette.info),
    }
}

/// An item's text, its title (in a release from before the groups) leading in bold.
fn item_text(item: &Item) -> StyledText {
    let lead = item.title.as_ref().map_or(0, |title| title.len() + 2);
    let line = match &item.title {
        Some(title) => format!("{title}. {}", item.text),
        None => item.text.clone(),
    };
    let bold = HighlightStyle {
        font_weight: Some(FontWeight::MEDIUM),
        ..HighlightStyle::default()
    };
    StyledText::new(line).with_highlights((lead > 0).then_some((0..lead, bold)))
}

fn release_block(index: usize, release: &Release, palette: &Palette) -> impl IntoElement + use<> {
    let text = |child: AnyElement| {
        div()
            .text_size(px(13.))
            .line_height(relative(1.5))
            .text_color(palette.text)
            .child(child)
    };
    let groups = Kind::ALL.into_iter().filter_map(|kind| {
        let items: Vec<&Item> = release
            .items
            .iter()
            .filter(|item| item.kind == Some(kind))
            .collect();
        (!items.is_empty()).then(|| {
            let (name, color) = look(kind, palette);
            v_flex()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.text_muted)
                        .child(kind.heading()),
                )
                .children(items.into_iter().map(move |item| {
                    h_flex()
                        .items_start()
                        .gap(px(10.))
                        .child(
                            div()
                                .flex_none()
                                .pt(px(2.))
                                .child(icon(name).size(px(16.)).text_color(color)),
                        )
                        .child(text(item_text(item).into_any_element()).flex_1().min_w_0())
                }))
        })
    });
    let untitled = release.items.iter().filter(|item| item.kind.is_none());
    // "KChess 2026.11.9"; the unreleased notes a development build shows are "Unreleased".
    let name = match kchess_delta::Version::parse(&release.version) {
        Some(_) => format!("KChess {}", release.version),
        None => release.version.clone(),
    };
    v_flex()
        .id(("whats-new-release", index))
        .test_support()
        .aria_label(name.clone())
        .gap(px(14.))
        .when(index > 0, |block| {
            block.pt(px(20.)).border_t_1().border_color(palette.border)
        })
        .child(
            h_flex()
                .items_baseline()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.highlighted)
                        .child(name),
                )
                .children(release.date.as_deref().and_then(date_label).map(|date| {
                    div()
                        .text_size(px(12.))
                        .text_color(palette.text_muted)
                        .child(date)
                })),
        )
        .children(release.summary.clone().map(|summary| {
            text(summary.clone().into_any_element())
                .id(("whats-new-summary", index))
                .test_support()
                .aria_label(summary)
        }))
        .children(groups)
        .children(untitled.map(|item| text(item_text(item).into_any_element())))
}

fn list(releases: &[Release], palette: &Palette, height: Pixels) -> impl IntoElement {
    v_flex()
        .id("whats-new")
        .max_h(height)
        .overflow_y_scroll()
        .gap(px(20.))
        .when(releases.is_empty(), |list| {
            list.child(
                div()
                    .text_size(px(13.))
                    .text_color(palette.text_muted)
                    .child("No release notes in this build"),
            )
        })
        .children(
            releases
                .iter()
                .enumerate()
                .map(|(index, release)| release_block(index, release, palette)),
        )
        .test_support()
}

#[cfg(test)]
mod tests {
    use super::{Item, Kind, NOTES, date_label, parse, releases_up_to, to_show};

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
                    kind: None,
                    title: Some("Guess the move".into()),
                    text: "a drill, see Practice. It carries on.".into(),
                },
                Item {
                    kind: None,
                    title: None,
                    text: "A plain fix.".into(),
                },
            ]
        );
        assert_eq!(date_label("2026-11-04").as_deref(), Some("Nov 4, 2026"));
    }

    /// A release written with its summary and groups reads them; one from its GitHub release's
    /// text reads the same.
    #[test]
    fn a_release_reads_its_summary_and_groups() {
        let notes = "## 2026.11.10 (2026-11-25)\n\nKChess is a database.\n\n### Added\n\n\
            - You can import games.\n\n### Fixed\n\n- Clocks no longer skip.\n";
        let release = &parse(notes)[0];
        assert_eq!(release.summary.as_deref(), Some("KChess is a database."));
        let kinds: Vec<_> = release.items.iter().map(|item| item.kind).collect();
        assert_eq!(kinds, [Some(Kind::Added), Some(Kind::Fixed)]);
        let body = notes.split_once('\n').unwrap().1;
        assert_eq!(
            &super::Release::from_body("2026.11.10", Some("2026-11-25"), body),
            release
        );
        assert!(super::Release::from_body("2026.11.10", None, "").is_empty());
        let next = super::unreleased(SAMPLE).expect("the unreleased notes");
        assert_eq!(next.version, "Unreleased");
        assert_eq!(super::unreleased(notes), None);
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

    /// The notes built into the app read (`kchess_delta::notes::problems`, which
    /// `cargo xtask release-verify` also runs), and the running release has its section.
    #[test]
    fn the_shipped_notes_are_well_formed() {
        assert_eq!(kchess_delta::notes::problems(NOTES), Vec::<String>::new());
        let all = parse(NOTES);
        for release in &all {
            assert!(
                release.date.as_deref().and_then(date_label).is_some(),
                "{} has a date",
                release.version
            );
        }
        let current = crate::updates::current_version();
        assert!(
            all.iter().any(|release| release.version == current),
            "WHATS_NEW.md has a section for {current}"
        );
    }
}
