//! The rows every category is built of (`Rows`): a title, its hint and control, a reset to the
//! default, the pickers laid out in a grid, and the row a search revealed, lit.

use super::*;

/// How long a revealed row stays lit.
pub(super) const REVEAL_LIT: std::time::Duration = std::time::Duration::from_millis(2000);
/// The room left above a revealed row.
pub(super) const REVEAL_ROOM: Pixels = px(16.);

/// A row the page scrolls to and lights.
pub(super) struct Reveal {
    pub(super) title: &'static str,
    /// Where the page keeps the row while it is lit.
    pub(super) pin: std::rc::Rc<std::cell::Cell<Pin>>,
    pub(super) scroll: ScrollHandle,
    pub(super) _fade: Task<()>,
}

impl Reveal {
    pub(super) fn mark(&self) -> RowMark {
        RowMark {
            title: self.title,
            pin: self.pin.clone(),
            scroll: self.scroll.clone(),
        }
    }
}

/// How the page keeps a revealed row in view while it is lit: the pictures above it (piece sets,
/// boards) can still be loading, so the page keeps the row where it scrolled it until they are,
/// and lets go once the person scrolls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Pin {
    /// Not scrolled yet.
    Start,
    /// Scrolled to this offset.
    At(Pixels),
    /// The person scrolled: the page leaves the scroll to them.
    Left,
}

/// The scroll offset that puts a row at `row_top` (in the window, as the page is scrolled now)
/// `REVEAL_ROOM` below the top of a page whose view starts at `view_top`, as far as `max` allows;
/// `None` once the person has scrolled away from where the page put it.
pub(super) fn pin_scroll(
    pin: Pin,
    offset: Pixels,
    view_top: Pixels,
    row_top: Pixels,
    max: Pixels,
) -> Option<Pixels> {
    match pin {
        Pin::Left => None,
        Pin::At(set) if set != offset => None,
        Pin::Start | Pin::At(_) => {
            Some((offset + view_top + REVEAL_ROOM - row_top).clamp(-max, px(0.)))
        }
    }
}

/// What `Rows` needs to scroll to and light the revealed row.
#[derive(Clone)]
pub(super) struct RowMark {
    pub(super) title: &'static str,
    pub(super) pin: std::rc::Rc<std::cell::Cell<Pin>>,
    pub(super) scroll: ScrollHandle,
}

impl RowMark {
    /// The row lit, with a probe that scrolls the page to it on its first frame.
    pub(super) fn light(&self, row: AnyElement, palette: &Palette) -> AnyElement {
        let (pin, scroll) = (self.pin.clone(), self.scroll.clone());
        let probe = canvas(
            move |bounds, window, _| {
                let offset = scroll.offset();
                let Some(target) = pin_scroll(
                    pin.get(),
                    offset.y,
                    scroll.bounds().origin.y,
                    bounds.origin.y,
                    scroll.max_offset().y,
                ) else {
                    pin.set(Pin::Left);
                    return;
                };
                pin.set(Pin::At(target));
                if target != offset.y {
                    scroll.set_offset(point(offset.x, target));
                    window.on_next_frame(|window, _| window.refresh());
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        div()
            .id("revealed-row")
            .test_support()
            .relative()
            .m(px(-8.))
            .p(px(8.))
            .rounded(radii::SM)
            .bg(crate::theme::tint(palette.primary, 12.))
            .child(row)
            .child(probe)
            .into_any_element()
    }
}

/// A row's title and its optional hint.
pub(super) fn info(
    title: impl Into<SharedString>,
    hint: Option<SharedString>,
    palette: &Palette,
) -> Div {
    widgets::setting_info(title, hint, palette)
}

/// The category's rows, with dividers between them.
pub(super) struct Rows {
    pub(super) narrow: bool,
    /// The row being revealed, lit when it is added.
    pub(super) reveal: Option<RowMark>,
    pub(super) rows: Vec<AnyElement>,
    /// The settings as shown, read once for the rows' reset buttons.
    pub(super) shown: std::cell::OnceCell<defaults::Shown>,
}

impl Rows {
    /// Add a row titled `title`, lit if it is the one being revealed.
    pub(super) fn push(&mut self, title: &str, row: AnyElement, palette: &Palette) {
        let row = match &self.reveal {
            Some(mark) if mark.title == title => mark.light(row, palette),
            _ => row,
        };
        self.rows.push(row);
    }

    /// The reset button of the row titled `title`, while its setting differs from its default.
    pub(super) fn reset(
        &self,
        title: &str,
        reset: Option<defaults::Reset<'_>>,
    ) -> Option<AnyElement> {
        let reset = reset?;
        self.shown
            .get_or_init(|| defaults::Shown::of(reset.settings))
            .reset(title, reset)
    }

    /// A setting row: the title and its hint on the left, the control at the right edge, then
    /// its reset (`defaults::Reset`) while the setting differs from its default.
    pub(super) fn row(
        self,
        title: impl Into<SharedString>,
        hint: Option<SharedString>,
        control: impl IntoElement,
        reset: Option<defaults::Reset<'_>>,
        palette: &Palette,
    ) -> Rows {
        let title = title.into();
        self.row_with(
            &title.clone(),
            info(title, hint, palette),
            control,
            reset,
            palette,
        )
    }

    /// A row whose title column says more than a title and a hint (a level meter).
    pub(super) fn row_with(
        mut self,
        title: &str,
        info: Div,
        control: impl IntoElement,
        reset: Option<defaults::Reset<'_>>,
        palette: &Palette,
    ) -> Rows {
        let reset = self.reset(title, reset);
        let row = if self.narrow {
            v_flex()
                .gap(px(8.))
                .child(
                    h_flex()
                        .gap(px(8.))
                        .child(info.flex_1())
                        .child(widgets::setting_reset(reset)),
                )
                .child(div().max_w_full().child(control))
                .into_any_element()
        } else {
            widgets::resettable_row(info.min_w(px(200.)), control, reset).into_any_element()
        };
        self.push(title, row, palette);
        self
    }

    /// A yes/no row (`toggles`): the switch sits at the start of the control column.
    pub(super) fn switch(
        mut self,
        id: &'static str,
        settings: &Settings,
        enabled: bool,
        palette: &Palette,
    ) -> Rows {
        let toggle = toggles::toggle(id);
        let set = toggle.set;
        let control = h_flex().child(
            Switch::new(id)
                .large()
                .checked((toggle.on)(settings))
                .disabled(!enabled)
                .accessibility_label(toggle.title)
                .on_click(move |checked, _, cx| {
                    let checked = *checked;
                    crate::settings::change(cx, move |settings| set(settings, checked));
                }),
        );
        let reset = self.reset(toggle.title, defaults::Reset::toggle(toggle, settings));
        // A switch is small enough to stay beside its title in the narrowest window.
        let row = widgets::resettable_row(
            info(toggle.title, toggle.hint.map(SharedString::from), palette).min_w_0(),
            control,
            reset,
        )
        .into_any_element();
        self.push(toggle.title, row, palette);
        self
    }

    /// A control that needs the full width goes under its title, and its reset at the end of the
    /// title's line.
    pub(super) fn stacked(
        mut self,
        title: &'static str,
        hint: Option<&'static str>,
        control: impl IntoElement,
        reset: Option<defaults::Reset<'_>>,
        palette: &Palette,
    ) -> Rows {
        let reset = self.reset(title, reset);
        let row = v_flex()
            .gap(px(8.))
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(info(title, hint.map(SharedString::from), palette).flex_1())
                    .child(widgets::setting_reset(reset)),
            )
            .child(control)
            .into_any_element();
        self.push(title, row, palette);
        self
    }

    pub(super) fn card(self, palette: &Palette) -> Div {
        self.card_under(None, palette)
    }

    /// The rows in a card that opens with `header` (a title and its status).
    pub(super) fn card_under(self, header: Option<AnyElement>, palette: &Palette) -> Div {
        // 4px above and below: the rows bring their own padding.
        widgets::card(palette)
            .py(px(4.))
            .children(header.map(|header| div().pt(px(14.)).pb(px(4.)).px(px(4.)).child(header)))
            .children(self.rows.into_iter().enumerate().map(|(index, row)| {
                div()
                    .py(px(18.))
                    .px(px(4.))
                    .when(index > 0, |row| {
                        row.border_t_1().border_color(palette.border)
                    })
                    .child(row)
            }))
    }
}

/// A segmented choice that changes one setting.
pub(super) fn choice<T: Copy + PartialEq + 'static>(
    id: &'static str,
    items: &[(T, &'static str, Option<Icon>)],
    current: T,
    set: impl Fn(&mut Settings, T) + Clone + 'static,
    palette: &Palette,
) -> impl IntoElement {
    let values: Vec<T> = items.iter().map(|(value, _, _)| *value).collect();
    let selected = values
        .iter()
        .position(|value| *value == current)
        .unwrap_or(0);
    widgets::setting_pills(
        id,
        items
            .iter()
            .map(|(_, label, glyph)| (glyph.clone(), SharedString::from(*label)))
            .collect(),
        selected,
        move |index, _, cx| {
            if let Some(&value) = values.get(index) {
                let set = set.clone();
                crate::settings::change(cx, move |settings| set(settings, value));
            }
        },
        palette,
    )
}

/// A picture and its name; the chosen one is ringed in the primary color.
#[expect(
    clippy::too_many_arguments,
    reason = "one argument per part of the option"
)]
pub(super) fn option(
    id: impl Into<ElementId>,
    width: Pixels,
    chosen: bool,
    picture: impl IntoElement,
    name: impl Into<SharedString>,
    tag: Option<&'static str>,
    choose: impl Fn(&mut App) + 'static,
    palette: &Palette,
) -> impl IntoElement {
    let name = name.into();
    v_flex()
        .id(id)
        .test_support()
        .role(gpui_kit::Role::Button)
        .aria_label(name.clone())
        .aria_selected(chosen)
        .w(width)
        .items_center()
        .gap(px(4.))
        .py(px(6.))
        .px(px(4.))
        .rounded(radii::LG)
        .bg(palette.bg)
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .when(chosen, |option| {
            option
                .border_2()
                .border_color(palette.primary)
                .py(px(4.))
                .px(px(4.))
                .text_color(palette.highlighted)
        })
        .when(!chosen, |option| {
            option
                .border_1()
                .border_color(palette.border)
                .text_color(palette.text_muted)
                .press(
                    |option| {
                        option
                            .border_color(palette.border_accented)
                            .bg(palette.accented)
                            .text_color(palette.toned)
                    },
                    |option| option.bg(palette.pressed(palette.accented)),
                )
        })
        .on_click(move |_, _, cx| choose(cx))
        .child(picture)
        .child(name)
        .children(tag.map(|tag| {
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.primary)
                .child(tag)
        }))
}

/// Lay pickers out in rows of `count`, `OPTION_GAP` apart.
pub(super) fn grid(count: usize, options: Vec<AnyElement>) -> impl IntoElement {
    let mut rows = Vec::new();
    let mut options = options.into_iter().peekable();
    while options.peek().is_some() {
        rows.push(
            h_flex()
                .gap(OPTION_GAP)
                .children(options.by_ref().take(count)),
        );
    }
    v_flex().gap(OPTION_GAP).children(rows)
}
