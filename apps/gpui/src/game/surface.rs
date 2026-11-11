//! The board surface every game page shares (Play with computer, Over the board, the Lichess
//! game, Watch, Broadcasts and a game gone through in Game history): the keys and the mouse
//! wheel that step through the moves, a click giving the keys back to the page, the move box,
//! the running clock that redraws only when a face changes, the low-time warning, and the
//! board's blindfold and premove. F turns the board through the app's keymap
//! (`menu::FlipBoard`), so every page that turns its board answers it.

use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use shakmaty::Color;
use shakmaty::variant::VariantPosition;

use crate::board::BoardState;
use crate::theme::Palette;

/// How often a running clock is looked at: a face shows tenths in its last ten seconds.
pub const TICK: Duration = Duration::from_millis(100);

/// A page whose board steps through the moves of a game.
pub trait Steps: Sized + 'static {
    /// The move shown and how many there are; `None` while the page shows no moves to step
    /// through (the chess clock).
    fn steps(&self, cx: &App) -> Option<(usize, usize)>;

    /// Show the position after `ply` moves.
    fn step_to(&mut self, ply: usize, window: &mut Window, cx: &mut Context<Self>);

    /// The mouse wheel's turn so far.
    fn wheel(&mut self) -> &mut crate::game::view::Wheel;
}

/// The page's root as a board surface: it keeps the keys (a click anywhere gives them back,
/// before a text field such as the move box takes them for itself), and the arrows, Home and End
/// step through the moves when no text field has them.
pub fn keys<V: Steps, E: InteractiveElement>(
    element: E,
    focus: &FocusHandle,
    cx: &mut Context<V>,
) -> E {
    let page = focus.clone();
    element
        .track_focus(focus)
        .capture_any_mouse_down(cx.listener(move |_, event: &MouseDownEvent, window, cx| {
            if event.button == MouseButton::Left {
                window.focus(&page, cx);
            }
        }))
        .on_key_down({
            let page = focus.clone();
            cx.listener(move |this: &mut V, event: &KeyDownEvent, window, cx| {
                if !crate::board::page_has_keys(&page, window, cx)
                    || event.keystroke.modifiers.modified()
                {
                    return;
                }
                let Some((ply, count)) = this.steps(cx) else {
                    return;
                };
                if let Some(next) =
                    crate::game::view::navigate(event.keystroke.key.as_str(), ply, count)
                {
                    this.step_to(next, window, cx);
                    cx.stop_propagation();
                }
            })
        })
}

/// The board's column, where the mouse wheel steps through the moves, as on Lichess.
pub fn wheel<V: Steps>(column: Div, cx: &mut Context<V>) -> Div {
    column.on_scroll(
        cx.listener(|this: &mut V, event: &ScrollEvent, window, cx| {
            let Some(key) = this
                .wheel()
                .turn(event.delta.pixel_delta(window.line_height()))
            else {
                return;
            };
            if let Some((ply, count)) = this.steps(cx)
                && let Some(next) = crate::game::view::navigate(key, ply, count)
            {
                this.step_to(next, window, cx);
            }
            cx.stop_propagation();
        }),
    )
}

/// Run `tick` every `period` while the page lives and `tick` says to go on.
pub fn every<V: 'static>(
    period: Duration,
    cx: &mut Context<V>,
    mut tick: impl FnMut(&mut V, &mut Context<V>) -> bool + 'static,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(period).await;
            match this.update(cx, |this, cx| tick(this, cx)) {
                Ok(true) => {}
                Ok(false) | Err(_) => break,
            }
        }
    })
}

/// What the clock faces showed at the last tick: a tick redraws only when one changes.
pub struct Faces<T>(Option<T>);

impl<T> Default for Faces<T> {
    fn default() -> Self {
        Faces(None)
    }
}

impl<T: PartialEq> Faces<T> {
    /// The faces show `shown` now: whether that differs from the last tick's.
    pub fn changed(&mut self, shown: T) -> bool {
        let changed = self.0.as_ref() != Some(&shown);
        self.0 = Some(shown);
        changed
    }
}

impl<T: PartialEq + Clone, const N: usize> Faces<[T; N]> {
    /// Which of the faces show something else than at the last tick.
    pub fn each_changed(&mut self, shown: [T; N]) -> [bool; N] {
        let before = self.0.replace(shown.clone());
        std::array::from_fn(|ix| before.as_ref().map(|before| &before[ix]) != Some(&shown[ix]))
    }
}

/// The blindfold and premove settings on a board: blindfold hides the pieces while the game is
/// `live`, and `premove`, the side waiting for its turn, may queue a move when premoves are on.
pub fn board_options(state: &mut BoardState, live: bool, premove: Option<Color>, cx: &App) {
    let frontend = crate::settings::frontend(cx);
    state.blindfold = frontend.blindfold && live;
    state.premove = premove.filter(|_| frontend.premove);
}

/// The box a move is typed into, SAN or UCI, and what it says of a move it cannot play.
pub struct MoveBox {
    pub input: Entity<InputState>,
    message: Option<SharedString>,
}

impl MoveBox {
    /// A move box whose Enter calls `typed` on the page.
    pub fn new<V: 'static>(
        typed: fn(&mut V, &mut Window, &mut Context<V>),
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> (MoveBox, Subscription) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type a move"));
        let subscription = cx.subscribe_in(&input, window, move |this, _, event, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                typed(this, window, cx);
            }
        });
        (
            MoveBox {
                input,
                message: None,
            },
            subscription,
        )
    }

    /// The text typed, trimmed.
    pub fn text(&self, cx: &App) -> String {
        self.input.read(cx).value().trim().to_string()
    }

    /// The move typed, as UCI, when it is legal in `position` of `variant`: the box is emptied.
    /// Otherwise it says what it takes. `None` too when nothing is typed.
    pub fn take(
        &mut self,
        variant: &str,
        position: Option<&VariantPosition>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<String> {
        let text = self.text(cx);
        if text.is_empty() {
            return None;
        }
        let uci = position.and_then(|position| {
            kchess_games::typed_uci(variant, &kchess_domain::rules::make_fen(position), &text)
        });
        match &uci {
            Some(_) => {
                self.message = None;
                self.input
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
            None => self.message = Some("Enter a legal move, such as Nf3 or g1f3".into()),
        }
        uci
    }

    /// What the box says under the board, if anything.
    pub fn message(&self, palette: &Palette) -> Option<Div> {
        self.message.clone().map(|message| {
            div()
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(message)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Faces;

    /// A tick redraws only the faces that show something new.
    #[test]
    fn a_tick_redraws_only_a_changed_face() {
        let mut faces = Faces::default();
        assert_eq!(faces.each_changed(["5:00", "5:00"]), [true, true]);
        assert_eq!(faces.each_changed(["4:59", "5:00"]), [true, false]);
        assert_eq!(faces.each_changed(["4:59", "5:00"]), [false, false]);
        let mut both = Faces::default();
        assert!(both.changed(("1:00", "1:00")));
        assert!(!both.changed(("1:00", "1:00")));
        assert!(both.changed(("0:59", "1:00")));
    }
}
