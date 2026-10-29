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
    _owner_changed: Subscription,
}

impl<V: 'static> Part<V> {
    /// A part of the view `cx` belongs to, drawn by `render`.
    pub fn new(render: RenderPart<V>, cx: &mut Context<V>) -> Entity<Self> {
        let owner = cx.entity();
        cx.new(|cx| Part {
            owner: owner.downgrade(),
            render,
            _owner_changed: cx.observe(&owner, |_, _, cx| cx.notify()),
        })
    }
}

impl<V: 'static> Render for Part<V> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let render = self.render;
        self.owner
            .update(cx, |owner, cx| render(owner, window, cx))
            .unwrap_or_else(|_| Empty.into_any_element())
    }
}
