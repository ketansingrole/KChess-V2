//! Every shared widget drawn with sample content, as Zed's component preview shows its components:
//! a change to a widget is seen everywhere it looks, in the current theme, without finding a page
//! that draws it in each state. A development build opens it from Quick Pick ("Component
//! gallery"); its test draws every sample in a light and a dark theme.
//!
//! A new shared widget adds its samples to `samples`, one for each state a page draws it in.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::icon;
use crate::theme::Palette;
use crate::widgets::{self, Look, Pill, StatusBanner, Tone};

/// One widget in one state, under the group it is listed in.
pub struct Sample {
    pub group: &'static str,
    pub name: &'static str,
    pub draw: fn(&Palette) -> AnyElement,
}

/// The gallery's window: every sample, by group, in the current theme.
pub struct Gallery {
    samples: Vec<Sample>,
}

impl Gallery {
    /// The gallery of `samples`: this crate's (`samples`), and any the app adds after them.
    pub fn new(samples: Vec<Sample>) -> Gallery {
        Gallery { samples }
    }
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let mut column = v_flex()
            .id("gallery")
            .size_full()
            .overflow_y_scroll()
            .p(px(24.))
            .gap(px(20.))
            .bg(palette.bg)
            .text_color(palette.text);
        let mut group = None;
        for (index, sample) in self.samples.iter().enumerate() {
            if group != Some(sample.group) {
                group = Some(sample.group);
                column = column.child(widgets::heading(sample.group, &palette));
            }
            column = column.child(
                v_flex()
                    .id(("sample", index))
                    .test_support()
                    .gap(px(8.))
                    .child(widgets::hint(sample.name, &palette))
                    .child(
                        div()
                            .id(("drawn", index))
                            .test_support()
                            .child((sample.draw)(&palette)),
                    ),
            );
        }
        column
    }
}

fn nothing(_: usize, _: &mut Window, _: &mut App) {}

fn tabs() -> Vec<(Option<gpui_kit::component::Icon>, SharedString)> {
    vec![
        (Some(icon("swords")), SharedString::new_static("Games")),
        (
            Some(icon("trophy")),
            SharedString::new_static("Tournaments"),
        ),
        (None, SharedString::new_static("Studies")),
    ]
}

fn pills() -> Vec<Pill> {
    let pill = |label: &'static str, badge: Option<&'static str>, disabled| Pill {
        icon: None,
        label: SharedString::new_static(label),
        badge: badge.map(SharedString::new_static),
        disabled,
    };
    vec![
        pill("All", None, false),
        pill("Rated", Some("12"), false),
        pill("Casual", None, false),
        pill("Correspondence", None, true),
    ]
}

/// This crate's widgets, each in the states the pages draw it in.
pub fn samples() -> Vec<Sample> {
    vec![
        Sample {
            group: "Text",
            name: "Heading and hint",
            draw: |palette| {
                v_flex()
                    .gap(px(4.))
                    .child(widgets::heading("Recent games", palette))
                    .child(widgets::hint(
                        "Your last 20 games on this computer.",
                        palette,
                    ))
                    .into_any_element()
            },
        },
        Sample {
            group: "Text",
            name: "Section title with a count",
            draw: |palette| {
                widgets::section_title("Studies", Some("12".into()), palette).into_any_element()
            },
        },
        Sample {
            group: "Text",
            name: "Dotted details",
            draw: |_| widgets::dotted(["3+2", "Rated", "Blitz"]).into_any_element(),
        },
        Sample {
            group: "Badges",
            name: "Every look",
            draw: |palette| {
                let looks = [
                    ("Won", Look::Success),
                    ("Lost", Look::Error),
                    ("Low time", Look::Warning),
                    ("New", Look::Primary),
                    ("Live", Look::Info),
                    ("Casual", Look::NeutralSoft),
                    ("Draft", Look::NeutralSubtle),
                ];
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .children(
                        looks
                            .into_iter()
                            .map(|(label, look)| widgets::badge(label, look, palette)),
                    )
                    .into_any_element()
            },
        },
        Sample {
            group: "Badges",
            name: "With an icon",
            draw: |palette| {
                widgets::icon_badge(icon("trophy"), "Arena", Look::Primary, palette)
                    .into_any_element()
            },
        },
        Sample {
            group: "Buttons",
            name: "Try again and a text button",
            draw: |_| {
                h_flex()
                    .gap(px(8.))
                    .child(widgets::try_again("gallery-try-again"))
                    .child(widgets::text_button("gallery-text-button").label("Show all"))
                    .into_any_element()
            },
        },
        Sample {
            group: "Pills",
            name: "Tabs",
            draw: |palette| {
                widgets::pill_tabs("gallery-tabs", tabs(), 0, nothing, palette).into_any_element()
            },
        },
        Sample {
            group: "Pills",
            name: "A setting's choices",
            draw: |palette| {
                widgets::setting_pills("gallery-setting", tabs(), 1, nothing, palette)
                    .into_any_element()
            },
        },
        Sample {
            group: "Pills",
            name: "Compact, with a count and a disabled choice",
            draw: |palette| {
                widgets::pills("gallery-pills", pills(), 1, nothing, true, palette)
                    .into_any_element()
            },
        },
        Sample {
            group: "Pills",
            name: "In a header",
            draw: |palette| {
                widgets::header_pills("gallery-header", pills(), 0, nothing, palette)
                    .into_any_element()
            },
        },
        Sample {
            group: "Pills",
            name: "Wrapping",
            draw: |palette| {
                widgets::wrapping_pills("gallery-wrapping", pills(), 2, nothing, palette)
                    .into_any_element()
            },
        },
        Sample {
            group: "Status",
            name: "Banners in every tone",
            draw: |_| {
                let banner = |tone, glyph, title: &'static str| {
                    StatusBanner::new(tone, icon(glyph), SharedString::new_static(title))
                };
                v_flex()
                    .gap(px(8.))
                    .child(StatusBanner::turn("White to move".into()))
                    .child(
                        banner(Tone::Plain, "circle-dot", "Waiting for your opponent").busy(true),
                    )
                    .child(
                        banner(Tone::Win, "trophy", "You won")
                            .detail("Checkmate · +8 rating".into()),
                    )
                    .child(banner(Tone::Loss, "flag", "You lost").detail("On time".into()))
                    .child(banner(Tone::Alert, "terminal", "Connection lost"))
                    .into_any_element()
            },
        },
        Sample {
            group: "Status",
            name: "Loading and a failure to retry",
            draw: |palette| {
                v_flex()
                    .gap(px(8.))
                    .child(widgets::loading("Loading games…", palette))
                    .child(widgets::retry_line(
                        "gallery-retry",
                        "The games could not be loaded.",
                        palette.error,
                        |_, _, _| {},
                    ))
                    .into_any_element()
            },
        },
        Sample {
            group: "Surfaces",
            name: "Card and side panel",
            draw: |palette| {
                h_flex()
                    .gap(px(16.))
                    .items_start()
                    .child(
                        widgets::card(palette)
                            .w(px(240.))
                            .child(widgets::heading("A card", palette)),
                    )
                    .child(
                        widgets::side_panel(palette)
                            .w(px(240.))
                            .child(widgets::heading("A side panel", palette))
                            .child(widgets::hint("Its parts stack.", palette)),
                    )
                    .into_any_element()
            },
        },
        Sample {
            group: "Surfaces",
            name: "Empty state",
            draw: |palette| {
                widgets::empty_state(
                    icon("book-open"),
                    "No studies yet",
                    Some("Import a PGN to start one.".into()),
                    palette,
                )
                .into_any_element()
            },
        },
        Sample {
            group: "Results",
            name: "A result row",
            draw: |palette| {
                widgets::result_row("As White", None, (12., 3., 5.), palette).into_any_element()
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, TestAppContext, px};

    use super::{Gallery, samples};
    use crate::theme::{Palette, ThemeChoice, apply, colors};
    use kchess_contracts::types::Appearance;

    /// Every sample draws, with room on the page, in a light and a dark theme: a widget that
    /// panics, or draws nothing, in some state fails here before any page shows it.
    #[gpui_kit::test]
    fn every_sample_draws_in_light_and_dark(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        // The window draws once as it opens, before a theme is applied to it.
        cx.update(|cx| cx.set_global(Palette::of(&colors("kchess", &[], false), false)));
        let count = samples().len();
        let window = cx.add_window(|_, _| Gallery::new(samples()));
        for appearance in [Appearance::Light, Appearance::Dark] {
            let choice = ThemeChoice {
                appearance,
                light: "kchess".into(),
                dark: "kchess".into(),
                custom: Vec::new(),
            };
            cx.update_window(window.into(), |_, window, cx| {
                apply(window, cx, &choice);
                window.render_frame(cx);
                for index in 0..count {
                    let drawn = window
                        .try_find(("drawn", index))
                        .unwrap_or_else(|| panic!("sample {index} is not drawn"));
                    let size = drawn.bounds().size;
                    assert!(
                        size.width > px(0.) && size.height > px(0.),
                        "sample {index} draws nothing in {appearance:?}"
                    );
                }
            })
            .unwrap();
        }
    }

    /// Samples are listed by group, each group in one run, so the gallery heads each once.
    #[test]
    fn each_group_is_listed_together() {
        let groups: Vec<&str> = samples().iter().map(|sample| sample.group).collect();
        let mut seen: Vec<&str> = Vec::new();
        for group in groups {
            if seen.last() != Some(&group) {
                assert!(!seen.contains(&group), "{group} is split");
                seen.push(group);
            }
        }
    }
}
