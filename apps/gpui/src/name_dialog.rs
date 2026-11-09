//! A dialog that asks for a name (Rename study, Add chapter, Save as study): its field, Cancel and
//! the confirm button, which Return presses too (the dialog's OK). `done` gets the trimmed name and closes the
//! dialog itself, or keeps it open with an error.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Disableable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;

use crate::theme::Palette;

type Done = Rc<dyn Fn(String, Entity<NameDialog>, &mut Window, &mut App)>;

pub struct NameDialog {
    input: Entity<InputState>,
    confirm: &'static str,
    error: Option<SharedString>,
    done: Done,
}

impl NameDialog {
    /// Open the dialog with `value` in its field.
    pub fn open(
        title: &'static str,
        value: &str,
        confirm: &'static str,
        window: &mut Window,
        cx: &mut App,
        done: impl Fn(String, Entity<NameDialog>, &mut Window, &mut App) + 'static,
    ) {
        let value = value.to_string();
        let view = cx.new(|cx| NameDialog {
            input: cx.new(|cx| InputState::new(window, cx).default_value(value)),
            confirm,
            error: None,
            done: Rc::new(done),
        });
        let input = view.read(cx).input.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let submit = view.clone();
            dialog
                .title(title)
                .w(px(420.))
                .child(view.clone())
                // Return is the dialog's OK; the name closes it only once it is taken.
                .on_ok(move |_, window, cx| {
                    submit.update(cx, |dialog, cx| dialog.submit(window, cx));
                    false
                })
        });
        // The dialog takes the focus as it opens; the field gets it once it is shown.
        window.defer(cx, move |window, cx| {
            input.update(cx, |input, cx| input.focus(window, cx));
        });
    }

    /// Show why the name was not taken; the dialog stays open.
    pub fn fail(&mut self, error: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.error = Some(error.into());
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.input.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        self.error = None;
        (self.done)(name, cx.entity(), window, cx);
    }
}

impl Render for NameDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let empty = self.input.read(cx).value().trim().is_empty();
        v_flex()
            .gap(px(12.))
            .child(Input::new(&self.input))
            .children(self.error.clone().map(|error| {
                div()
                    .text_size(px(14.))
                    .text_color(palette.error)
                    .child(error)
            }))
            .child(
                h_flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("name-cancel")
                            .outline()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("name-confirm")
                            .primary()
                            .label(self.confirm)
                            .disabled(empty)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}
