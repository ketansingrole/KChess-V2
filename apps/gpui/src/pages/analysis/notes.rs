//! Notes on the move shown (`MoveNotes.vue`): a comment and an annotation glyph, written right
//! under the move list (and shown in it), so annotating never means leaving the board. A game's
//! clock times (`[%clk …]`) ride along in the comment unseen.

use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use super::AnalysisPage;
use crate::assets::icon;
use crate::game::rules;
use crate::game::tree::{comment_text, move_number};
use crate::theme::{Palette, tint};
use crate::widgets::KeyboardFocus as _;

/// `MOVE_GLYPHS`: the annotations a move can carry, by NAG.
pub const MOVE_GLYPHS: [(i64, &str, &str); 6] = [
    (1, "!", "Good move"),
    (2, "?", "Mistake"),
    (3, "!!", "Brilliant move"),
    (4, "??", "Blunder"),
    (5, "!?", "Interesting move"),
    (6, "?!", "Dubious move"),
];

fn text(method: &str, args: &[Value]) -> String {
    rules(method, args)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// The NAGs with `nag` as the move's annotation (or none); `None` when nothing is left. The rules
/// answer numbers as floats.
pub(super) fn with_glyph(nags: Option<&[i64]>, nag: Option<i64>) -> Option<Vec<i64>> {
    let nags = rules("setMoveGlyph", &[json!(nags), json!(nag)]).ok()?;
    let nags: Vec<i64> = nags
        .as_array()?
        .iter()
        .map(|nag| crate::game::whole(nag) as i64)
        .collect();
    (!nags.is_empty()).then_some(nags)
}

/// The notes field and what it was last filled for.
pub struct Notes {
    pub field: Entity<TextareaState>,
    /// Shown while being written, or while the move has a comment.
    pub editing: bool,
    /// The move the field was filled from.
    pub filled_for: Option<String>,
}

impl AnalysisPage {
    /// The move's comments as written.
    fn raw_comment(&self) -> String {
        self.node()
            .comments
            .clone()
            .unwrap_or_default()
            .join("\n\n")
    }

    /// The comments as a reader sees them.
    fn shown_comment(&self) -> String {
        self.node()
            .comments
            .iter()
            .flatten()
            .map(|comment| comment_text(comment))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Refill the field when another move is shown.
    pub(super) fn sync_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.notes.filled_for.as_deref() == Some(self.path.as_str()) {
            return;
        }
        self.notes.filled_for = Some(self.path.clone());
        self.notes.editing = false;
        let shown = self.shown_comment();
        self.notes
            .field
            .update(cx, |field, cx| field.set_value(shown, window, cx));
    }

    /// The field changed: the move's comment follows, keeping its clock and other commands.
    pub(super) fn write_comment(&mut self, cx: &mut Context<Self>) {
        let typed = self.notes.field.read(cx).value().to_string();
        if typed == self.shown_comment() {
            return;
        }
        let next = text(
            "withCommentText",
            &[json!(self.raw_comment()), json!(typed)],
        );
        let path = self.path.clone();
        self.root.node_at_mut(&path).comments = (!next.is_empty()).then(|| vec![next]);
        self.changed(cx);
    }

    /// Comment on the move shown (`C`, or the move list's menu).
    pub(super) fn focus_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::prefs::set(cx, super::TOOL, "moves");
        self.notes.editing = true;
        self.notes
            .field
            .update(cx, |field, cx| field.focus(window, cx));
        cx.notify();
    }

    /// Set the move's annotation, or take it off when it is already that one.
    pub(super) fn toggle_glyph(&mut self, path: &str, nag: i64, cx: &mut Context<Self>) {
        let chosen = (self.glyph_of(path) != Some(nag)).then_some(nag);
        let node = self.root.node_at_mut(path);
        node.nags = with_glyph(node.nags.as_deref(), chosen);
        self.changed(cx);
    }

    /// The annotation of a move, by NAG.
    pub(super) fn glyph_of(&self, path: &str) -> Option<i64> {
        self.root
            .node_at(path)
            .nags
            .iter()
            .flatten()
            .copied()
            .find(|nag| (1..=6).contains(nag))
    }

    pub(super) fn render_notes(&self, cx: &mut Context<Self>, palette: &Palette) -> AnyElement {
        let node = self.node();
        let subject = if self.path.is_empty() {
            "the starting position".to_string()
        } else {
            format!("{} {}", move_number(node.ply - 1., true), node.san)
        };
        let clock = rules("commentClock", &[json!(self.raw_comment())])
            .ok()
            .and_then(|seconds| seconds.as_f64())
            .map(|seconds| text("formatClock", &[json!(seconds * 1000.)]));
        let current = self.glyph_of(&self.path);
        let open = self.notes.editing || !self.shown_comment().is_empty();
        let glyphs = (!self.path.is_empty()).then(|| {
            h_flex()
                .gap(px(2.))
                .children(MOVE_GLYPHS.map(|(nag, glyph, label)| {
                    let on = current == Some(nag);
                    div()
                        .id(SharedString::from(format!("glyph-{nag}")))
                        .role(Role::Button)
                        .keyboard_focus(palette.primary)
                        .aria_label(label)
                        .aria_toggled(if on { Toggled::True } else { Toggled::False })
                        .min_w(px(26.))
                        .h(px(24.))
                        .px(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .border_1()
                        .rounded(px(6.))
                        .text_size(px(12.))
                        .font_weight(FontWeight::BOLD)
                        .cursor_pointer()
                        .when(on, |button| {
                            button
                                .border_color(tint(palette.primary, 60.))
                                .bg(tint(palette.primary, 22.))
                                .text_color(palette.highlighted)
                        })
                        .when(!on, |button| {
                            button
                                .border_color(gpui_kit::transparent_black())
                                .text_color(palette.text_muted)
                                .hover(|button| {
                                    button.bg(palette.accented).text_color(palette.text)
                                })
                        })
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(label).build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let path = this.path.clone();
                            this.toggle_glyph(&path, nag, cx)
                        }))
                        .child(glyph)
                }))
        });
        let head = h_flex()
            .gap(px(8.))
            .min_h(px(26.))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette.text_muted)
                    .child(div().truncate().child(subject.clone()))
                    .children(clock.filter(|clock| !clock.is_empty()).map(|clock| {
                        div()
                            .ml(px(4.))
                            .font_weight(FontWeight::NORMAL)
                            .child(format!("· {clock}"))
                    })),
            )
            .children(glyphs);
        let body = if open {
            div()
                .child(Textarea::new(&self.notes.field).aria_label(format!("Comment on {subject}")))
                .into_any_element()
        } else {
            h_flex()
                .id("add-comment")
                .role(Role::Button)
                .keyboard_focus(palette.primary)
                .aria_label("Add a comment")
                .w_full()
                .gap(px(6.))
                .px(px(10.))
                .py(px(7.))
                .border_1()
                .border_dashed()
                .border_color(palette.border_accented)
                .rounded(px(8.))
                .text_size(px(13.))
                .text_color(palette.text_muted)
                .cursor_text()
                .hover(|button| {
                    button
                        .border_color(palette.primary)
                        .text_color(palette.text)
                })
                .on_click(cx.listener(|this, _, window, cx| this.focus_notes(window, cx)))
                .child(icon("message-square-plus").size(px(14.)))
                .child("Add a comment")
                .child(div().flex_1())
                .child(
                    div()
                        .px(px(5.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(palette.border_accented)
                        .text_size(px(11.))
                        .child("C"),
                )
                .into_any_element()
        };
        v_flex()
            .gap(px(6.))
            .child(head)
            .child(body)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::with_glyph;

    #[test]
    fn a_glyph_replaces_the_last_and_comes_off_again() {
        assert_eq!(with_glyph(None, Some(5)), Some(vec![5]));
        assert_eq!(with_glyph(Some(&[5]), Some(2)), Some(vec![2]));
        assert_eq!(with_glyph(Some(&[2]), None), None);
    }
}
