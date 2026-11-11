//! Settings as a dialog over the page it was opened from: its categories in headed groups on the
//! left, the one chosen on the right.

use std::cell::Cell;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::sidebar::{SidebarItem as _, SidebarMenu};
use gpui_kit::component::{ActiveTheme as _, Collapsible as _, Sizable as _, h_flex, v_flex};
use gpui_kit::*;

use super::{NavBlock, TOPBAR_HEIGHT, Workspace, nav_item, page_style};
use crate::assets::icon;
use crate::pages::settings::{Section, SectionGroup, SettingsPage};
use crate::theme::Palette;

/// The categories' list: to the top as the dialog opens, and to the category it shows.
#[derive(Default)]
pub(super) struct Sections {
    scroll: ScrollHandle,
    /// The group to scroll to once the list is drawn: one asked for as the dialog opens comes
    /// before the list exists.
    pending: Cell<Option<usize>>,
}

impl Sections {
    pub(super) fn to_top(&self) {
        self.scroll.set_offset(point(px(0.), px(0.)));
    }

    /// Scroll to the group of the category shown, which the palette may have chosen, or Settings
    /// remembered, out of a short window's sight.
    pub(super) fn scroll_to(&self, section: Section) {
        let group = SectionGroup::ALL
            .iter()
            .position(|group| *group == section.group())
            .unwrap_or(0);
        self.scroll.scroll_to_item(group);
        self.pending.set(Some(group));
    }
}

impl Workspace {
    pub(super) fn choose_section(&mut self, section: Section, cx: &mut Context<Self>) {
        if let Some(settings) = self.pages.get::<SettingsPage>() {
            settings.update(cx, |page, cx| page.show(section, cx));
        }
        self.sections.scroll_to(section);
        cx.notify();
    }

    /// Settings' categories in its dialog, in their groups.
    fn section_blocks(&self, cx: &mut Context<Self>) -> Vec<NavBlock> {
        let shown = self
            .pages
            .get::<SettingsPage>()
            .map_or(Section::Appearance, |page| page.read(cx).section());
        SectionGroup::ALL
            .into_iter()
            .map(|group| {
                NavBlock::Group(
                    group.heading(),
                    Box::new(SidebarMenu::new().gap_0().children(group.sections().map(
                        |section| {
                            nav_item(section.label(), section.icon())
                                .active(shown == section)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.choose_section(section, cx)
                                }))
                        },
                    ))),
                )
            })
            .collect()
    }

    /// Settings as a dialog over the page it was opened from, below the top bar (whose search
    /// finds settings, and whose window controls stay in reach): its categories on the left,
    /// the one chosen on the right. The close button, Escape or a press outside it closes it.
    pub(super) fn settings_dialog(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = *cx.global::<Palette>();
        let overlay = cx.theme().overlay;
        let viewport = window.viewport_size();
        let margin = px(24.);
        let width = (viewport.width - margin * 2.).min(px(1040.));
        let height = viewport.height - TOPBAR_HEIGHT - margin * 2.;
        let blocks: Vec<AnyElement> = self
            .section_blocks(cx)
            .into_iter()
            .enumerate()
            .map(|(ix, block)| {
                block
                    .collapsed(false)
                    .render(SharedString::from(format!("s{}", ix + 1)), window, cx)
                    .into_any_element()
            })
            .collect();
        // GPUI drops a scroll asked for before the list's first layout, when it does not know yet
        // that the list scrolls: ask again once it has been laid out.
        if let Some(group) = self.sections.pending.get() {
            if self.sections.scroll.bounds().size.height > px(0.) {
                self.sections.pending.set(None);
                self.sections.scroll.scroll_to_item(group);
            } else {
                window.request_animation_frame();
            }
        }
        let close = cx.listener(|this, _, window, cx| this.choose(this.back, window, cx));
        div()
            .id("settings-backdrop")
            .absolute()
            .top(TOPBAR_HEIGHT)
            .left_0()
            .right_0()
            .bottom_0()
            .occlude()
            .bg(overlay)
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.choose(this.back, window, cx)),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if crate::widgets::is_escape(event) {
                    cx.stop_propagation();
                    this.choose(this.back, window, cx);
                }
            }))
            .child(
                v_flex()
                    .id("settings-dialog")
                    .test_support()
                    .role(Role::Dialog)
                    .aria_label("Settings")
                    .w(width)
                    .h(height)
                    .rounded(crate::theme::radii::XL)
                    .border_1()
                    .border_color(palette.border)
                    .bg(cx.theme().background)
                    .overflow_hidden()
                    // A press inside is the dialog's own, not one outside it.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .flex_none()
                            .h(px(52.))
                            .px(px(20.))
                            .border_b_1()
                            .border_color(palette.border)
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(palette.highlighted)
                                    .child("Settings"),
                            )
                            .child(
                                Button::new("settings-close")
                                    .ghost()
                                    .small()
                                    .icon(icon("x"))
                                    .accessibility_label("Close settings")
                                    .tooltip("Close (Esc)")
                                    .on_click(close),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .items_stretch()
                            .child(
                                v_flex()
                                    .id("settings-sections")
                                    .test_support()
                                    .w(px(232.))
                                    .flex_none()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.sections.scroll)
                                    .px_3()
                                    .py_2()
                                    .gap_1()
                                    .border_r_1()
                                    .border_color(palette.border)
                                    .bg(cx.theme().tokens.sidebar)
                                    .children(blocks),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .child(self.pages.view().cached(page_style())),
                            ),
                    ),
            )
            .into_any_element()
    }
}
