//! A part of a view drawn as a cached view of its own.
//!
//! GPUI redraws a view when it is notified and also when any view inside it is: a board sliding a
//! piece, or a clock ticking, redraws the page and the shell around it, and their whole trees are
//! built and laid out again. A `Part` holds a slice of its owner's render (the shell's sidebar, a
//! page's panel) and is embedded with `AnyView::cached`. It redraws when its owner is notified
//! (the owner's own state changed), when a view inside it is, or when the windows refresh
//! (settings, preferences); while the owner only redraws for a view beside it, the part's last
//! frame, layout included, is reused.

use gpui_kit::*;

type RenderPart<V> = fn(&mut V, &mut Window, &mut Context<V>) -> AnyElement;

pub struct Part<V: 'static> {
    owner: WeakEntity<V>,
    render: RenderPart<V>,
    /// Whether the part fills the slot it is cached in (`Part::panel`).
    fill: bool,
    _owner_changed: Subscription,
}

impl<V: 'static> Part<V> {
    /// A part of the view `cx` belongs to, drawn by `render`, drawn again whenever its owner is
    /// notified.
    pub fn new(render: RenderPart<V>, cx: &mut Context<V>) -> Entity<Self> {
        let owner = cx.entity();
        cx.new(|cx| Part {
            owner: owner.downgrade(),
            render,
            fill: false,
            _owner_changed: cx.observe(&owner, |_, _, cx| cx.notify()),
        })
    }

    /// A part as `new`, drawn across the whole slot it is cached in. A cached view is laid out
    /// alone in its slot, so a panel sized for a flex row (`flex_1`, a minimum and a maximum
    /// width) would take its content's width instead: long engine lines pushed the analysis
    /// panel past the window's edge.
    pub fn panel(render: RenderPart<V>, cx: &mut Context<V>) -> Entity<Self> {
        let part = Self::new(render, cx);
        part.update(cx, |part, _| part.fill = true);
        part
    }

    /// A part drawn again when its owner is notified only if `inputs`, everything `render`
    /// shows of the owner and the models it reads, changed. A notification that changes none of
    /// them (a model only the page shows, a redraw of the owner for another part) reuses the
    /// part's last frame. `inputs` must cover all of `render`'s reads that are not settings or
    /// the window's size, which refresh the window.
    pub fn keyed<K: PartialEq + 'static>(
        render: RenderPart<V>,
        inputs: fn(&V, &App) -> K,
        cx: &mut Context<V>,
    ) -> Entity<Self> {
        let owner = cx.entity();
        cx.new(|cx| {
            let mut shown: Option<K> = None;
            Part {
                owner: owner.downgrade(),
                render,
                fill: false,
                _owner_changed: cx.observe(&owner, move |_, owner, cx| {
                    let now = inputs(owner.read(cx), cx);
                    if shown.as_ref() != Some(&now) {
                        shown = Some(now);
                        cx.notify();
                    }
                }),
            }
        })
    }
}

impl<V: 'static> Render for Part<V> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let render = self.render;
        let element = self
            .owner
            .update(cx, |owner, cx| render(owner, window, cx))
            .unwrap_or_else(|_| Empty.into_any_element());
        if self.fill {
            div().size_full().child(element).into_any_element()
        } else {
            element
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::{h_flex, v_flex};
    use gpui_kit::test::{TestSupportExt as _, TestWindowExt as _};
    use gpui_kit::{
        AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
        ParentElement as _, Render, Styled as _, TestAppContext, Window, div, px, size,
    };

    use super::Part;
    use crate::play_layout::{PANEL_MAX, PANEL_MIN, panel_style};

    /// A board page's row: a 600 px board and its panel, cached, whose content is wider than the
    /// room left (an engine line).
    struct Row {
        panel: Option<Entity<Part<Row>>>,
    }

    impl Row {
        fn render_panel(&mut self, _: &mut Window, _: &mut Context<Self>) -> AnyElement {
            v_flex()
                .id("panel")
                .test_support()
                .h(px(200.))
                .flex_1()
                .min_w(PANEL_MIN)
                .max_w(PANEL_MAX)
                .child(div().flex_none().w(px(540.)).h(px(20.)))
                .into_any_element()
        }
    }

    impl Render for Row {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let panel = self.panel.clone().expect("the panel");
            h_flex()
                .size_full()
                .items_start()
                .child(div().flex_none().w(px(600.)).h(px(200.)))
                .child(panel.cached(panel_style(px(200.), PANEL_MAX)))
        }
    }

    /// The analysis panel took its engine lines' width and ran past the window's edge.
    #[gpui_kit::test]
    fn a_cached_panel_fills_its_slot_however_wide_its_content(cx: &mut TestAppContext) {
        let window = cx.add_window(|_, cx| {
            let mut row = Row { panel: None };
            row.panel = Some(Part::panel(
                |this, window, cx| this.render_panel(window, cx),
                cx,
            ));
            row
        });
        cx.simulate_window_resize(window.into(), size(px(1000.), px(400.)));
        cx.run_until_parked();
        let width = cx
            .update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                window.find("panel").bounds().size.width
            })
            .unwrap();
        assert_eq!(width, px(400.), "the room beside the board");
    }
}
