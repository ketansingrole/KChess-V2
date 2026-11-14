//! Pieces the board pages share: the player
//! line above and below a board, the status banner and the move list panel.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::assets::icon;
use crate::theme::{Palette, radii, tint};

/// What a button runs when it is clicked.
pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
/// What opening a card does; shared by its picture and its name.
pub type OpenHandler = std::rc::Rc<dyn Fn(&mut Window, &mut App)>;

/// Keyboard access for a clickable element that is not a kit button: a Tab stop that Enter and
/// Space click (GPUI clicks the focused element), outlined in `ring` while the keyboard has it, as
/// the kit outlines its buttons. A mouse press leaves the focus where it was, as a native button's
/// does, so a page keeps its keys (the arrows, Space) after a move or a row is clicked.
pub trait KeyboardFocus: InteractiveElement {
    fn keyboard_focus(self, ring: Hsla) -> Self {
        self.tab_index(0)
            // Runs before GPUI's own focus-on-press for this element, which then stands down.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .focus_visible(move |style| {
                style.shadow(vec![BoxShadow {
                    color: ring,
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                    inset: false,
                }])
            })
    }
}

impl<E: InteractiveElement> KeyboardFocus for E {}

/// How far a pressed link or text button fades.
const PRESSED_OPACITY: f32 = 0.7;

/// Pointer feedback for a clickable element that is not a kit button: its hover style, and while
/// the pointer is held down on it a stronger one at once, with no transition, as the kit's buttons
/// press. Only an element with an id keeps a pressed state, so these are on `Stateful` elements.
pub trait Pressable: StatefulInteractiveElement + Sized {
    /// Rows, tiles and chips: the accented fill on hover, a step stronger while pressed.
    fn pressable(self, palette: &Palette) -> Self {
        self.pressable_fill(palette.accented, palette)
    }

    /// `fill` on hover, a step stronger while pressed.
    fn pressable_fill(self, fill: Hsla, palette: &Palette) -> Self {
        let pressed = palette.pressed(fill);
        self.press(|style| style.bg(fill), |style| style.bg(pressed))
    }

    /// Bordered cards: the accented border on hover, and the accented fill too while pressed.
    fn pressable_card(self, palette: &Palette) -> Self {
        let (border, fill) = (palette.border_accented, palette.accented);
        self.press(|style| style.border_color(border), |style| style.bg(fill))
    }

    /// Bordered tiles that fill on hover: the accented border and fill, the fill a step stronger
    /// while pressed.
    fn pressable_tile(self, palette: &Palette) -> Self {
        let (border, fill) = (palette.border_accented, palette.accented);
        let pressed = palette.pressed(fill);
        self.press(
            |style| style.border_color(border).bg(fill),
            |style| style.bg(pressed),
        )
    }

    /// Text buttons and tabs: the text in `color` on hover, faded while pressed.
    fn pressable_text(self, color: Hsla) -> Self {
        self.press(
            |style| style.text_color(color),
            |style| style.opacity(PRESSED_OPACITY),
        )
    }

    /// Links: underlined on hover, faded while pressed.
    fn pressable_link(self) -> Self {
        self.press(
            |style| style.underline(),
            |style| style.opacity(PRESSED_OPACITY),
        )
    }

    /// Pictures and colored buttons: faded to `opacity` on hover, further while pressed.
    fn pressable_fade(self, opacity: f32) -> Self {
        self.press(
            |style| style.opacity(opacity),
            |style| style.opacity(opacity - 0.12),
        )
    }

    /// `hover` on hover; while pressed, `hover` with `pressed` over it. A touch press has no
    /// hover, so the pressed style repeats the hover one.
    fn press(
        self,
        hover: impl Fn(StyleRefinement) -> StyleRefinement,
        pressed: impl FnOnce(StyleRefinement) -> StyleRefinement,
    ) -> Self {
        self.hover(&hover).active(|style| pressed(hover(style)))
    }
}

impl<E: StatefulInteractiveElement> Pressable for E {}

/// Text a screen reader speaks whenever it changes, without moving the
/// reader's place (a move just played, a game's result). GPUI's elements cannot mark themselves
/// live, so this one writes AccessKit's `live` itself; each platform adapter announces a live
/// node when its text changes (macOS reads the value, Windows and Linux the name). Unseen: a
/// 1px box in its parent's corner, since a node with no area may be left out.
pub struct LiveRegion {
    id: ElementId,
    text: SharedString,
}

/// A polite live region saying `text`; `id` keeps it the same node from frame to frame, so only
/// a change of text is announced.
pub fn live_region(id: impl Into<ElementId>, text: impl Into<SharedString>) -> LiveRegion {
    LiveRegion {
        id: id.into(),
        text: text.into(),
    }
}

impl IntoElement for LiveRegion {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for LiveRegion {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let style = Style {
            position: Position::Absolute,
            inset: Edges {
                top: px(0.).into(),
                left: px(0.).into(),
                ..Default::default()
            },
            size: size(px(1.).into(), px(1.).into()),
            ..Default::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn a11y_role(&self) -> Option<Role> {
        Some(Role::Status)
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        node.set_label(self.text.to_string());
        node.set_value(self.text.to_string());
        node.set_live(accesskit::Live::Polite);
    }
}

mod badges;
mod cards;
mod dialogs;
mod moves;
mod panel;
mod pills;
mod players;
mod setting_rows;
mod status;
#[cfg(test)]
mod tests;

pub use badges::*;
pub use cards::*;
pub use dialogs::*;
pub use moves::*;
pub use panel::*;
pub use pills::*;
pub use players::*;
pub use setting_rows::*;
pub use status::*;
