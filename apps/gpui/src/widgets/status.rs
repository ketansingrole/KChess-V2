//! What a page says about its state: the status banner, spinners and retry lines.

use super::*;

/// Something under way: the loader turning, at `size`, in the text's color unless given one.
/// Every wait in the app uses it; a still `loader-circle` icon reads as stuck.
pub fn spinner(size: Pixels) -> gpui_kit::component::spinner::Spinner {
    gpui_kit::component::spinner::Spinner::new()
        .icon(icon("loader-circle"))
        .with_size(size)
        // An even turn: easing in and out reads as a stutter on a ring of dots.
        .ease(|delta| delta)
}

/// Escape with no modifier, which takes a detail view (a game, an event, a line) back to its list.
pub(crate) fn is_escape(event: &KeyDownEvent) -> bool {
    event.keystroke.key == "escape" && !event.keystroke.modifiers.modified()
}

/// A clock's last ten seconds, which alone show tenths ("0:09.4").
pub(crate) fn low_time(text: &str) -> bool {
    text.starts_with("0:0") && text.contains('.')
}
/// The kind of a status banner.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Win,
    Loss,
    Alert,
}

/// The game's state, its icon, and optionally a detail line, with
/// controls at the end of the row. Whose turn it is goes on a plain line of its own instead (`turn`).
#[derive(IntoElement)]
pub struct StatusBanner {
    tone: Tone,
    glyph: Option<Icon>,
    /// Something is under way: the spinner in place of the glyph.
    busy: bool,
    title: SharedString,
    detail: Option<SharedString>,
    extra: Option<AnyElement>,
    announce: bool,
}

impl StatusBanner {
    pub fn new(tone: Tone, glyph: Icon, title: SharedString) -> StatusBanner {
        StatusBanner {
            tone,
            glyph: Some(glyph),
            busy: false,
            title,
            detail: None,
            extra: None,
            announce: false,
        }
    }

    /// Show the spinner in place of the glyph while `busy`.
    pub fn busy(mut self, busy: bool) -> StatusBanner {
        self.busy = busy;
        self
    }

    /// The game going on as usual: its title and detail on one plain line, no surface or icon.
    pub fn turn(title: SharedString) -> StatusBanner {
        StatusBanner {
            tone: Tone::Plain,
            glyph: None,
            busy: false,
            title,
            detail: None,
            extra: None,
            announce: false,
        }
    }

    /// Say the title when it appears or changes (a live status): results and
    /// feedback, not a turn line the board already says.
    pub fn announce(mut self) -> StatusBanner {
        self.announce = true;
        self
    }

    pub fn detail(mut self, detail: SharedString) -> StatusBanner {
        self.detail = Some(detail);
        self
    }

    pub fn extra(mut self, extra: AnyElement) -> StatusBanner {
        self.extra = Some(extra);
        self
    }
}

impl RenderOnce for StatusBanner {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let (background, icon_color) = match self.tone {
            Tone::Plain => (palette.accented, palette.text),
            Tone::Win => (tint(palette.win, 18.), palette.win),
            Tone::Loss => (tint(palette.loss, 16.), palette.loss),
            Tone::Alert => (tint(palette.warning, 16.), palette.text),
        };
        let plain = self.glyph.is_none();
        // Read as a status: its title names it, its detail describes it.
        h_flex()
            .id("status-banner")
            .test_support()
            .role(Role::Status)
            .aria_label(self.title.clone())
            .when_some(self.detail.clone(), |banner, detail| {
                banner.aria_description(detail)
            })
            .items_center()
            .gap(px(8.))
            .when(plain, |banner| banner.h(px(24.)).text_size(px(14.)))
            .when(!plain, |banner| {
                banner
                    .min_h(px(56.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(radii::LG)
                    .text_size(px(13.))
                    .bg(background)
            })
            .when(self.announce, |banner| {
                banner.child(live_region("status-live", self.title.clone()))
            })
            .children(self.glyph.map(|glyph| {
                if self.busy {
                    div()
                        .flex_none()
                        .child(spinner(px(16.)).color(icon_color))
                        .into_any_element()
                } else {
                    glyph
                        .size(px(16.))
                        .flex_none()
                        .text_color(icon_color)
                        .into_any_element()
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .when(plain, |text| text.items_baseline().gap(px(8.)))
                    .when(!plain, |text| text.flex_col())
                    .child(
                        div()
                            .line_height(px(20.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette.highlighted)
                            .child(self.title),
                    )
                    .children(self.detail.map(|detail| {
                        div()
                            .line_height(px(18.))
                            .text_size(px(12.))
                            .text_color(palette.toned)
                            .child(detail)
                    })),
            )
            // Its action sits at the end of the row, centred against the text, not under it.
            .children(self.extra.map(|extra| div().flex_none().child(extra)))
    }
}

/// The banner action that leaves an earlier position for the game's last move.
pub fn back_to_game(on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> AnyElement {
    Button::new("back-to-game")
        .outline()
        .small()
        .label("Back to game")
        .on_click(on_click)
        .into_any_element()
}
