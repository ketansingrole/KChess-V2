//! One thing KChess downloads, as Settings → Storage lists it: its icon, its name and what it
//! holds (or how far its download has come), and its buttons at the row's end. Every download
//! reads the same way there, whichever page also offers it.

use super::*;

#[derive(IntoElement)]
pub struct DownloadRow {
    id: ElementId,
    glyph: Icon,
    title: SharedString,
    detail: SharedString,
    /// Under way: how far (0 to 1), when that is known.
    progress: Option<Option<f32>>,
    error: Option<(&'static str, SharedString)>,
    actions: Vec<AnyElement>,
}

impl DownloadRow {
    /// `detail` is what it holds and its size, or what a download would take.
    pub fn new(
        id: impl Into<ElementId>,
        glyph: Icon,
        title: impl Into<SharedString>,
        detail: impl Into<SharedString>,
    ) -> Self {
        DownloadRow {
            id: id.into(),
            glyph,
            title: title.into(),
            detail: detail.into(),
            progress: None,
            error: None,
            actions: Vec::new(),
        }
    }

    /// A download runs: a bar shows `share` of it, and the detail says how far in words.
    pub fn progress(mut self, share: Option<f32>) -> Self {
        self.progress = Some(share);
        self
    }

    /// What went wrong the last time, as `id`, announced to a screen reader.
    pub fn error(mut self, id: &'static str, error: Option<SharedString>) -> Self {
        self.error = error.map(|error| (id, error));
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }
}

impl RenderOnce for DownloadRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let label = format!("{}, {}", self.title, self.detail);
        h_flex()
            .id(self.id)
            .test_support()
            .aria_label(label)
            .w_full()
            .gap(px(12.))
            .py(px(10.))
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .size(px(32.))
                    .items_center()
                    .justify_center()
                    .rounded(radii::BOX)
                    .bg(palette.accented)
                    .child(self.glyph.size(px(16.)).text_color(palette.text_muted)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.title),
                    )
                    .when_some(self.progress, |column, share| {
                        column.child(progress_bar(share, &palette).my(px(4.)))
                    })
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(self.detail),
                    )
                    .children(self.error.map(|(id, error)| {
                        div()
                            .id(id)
                            .test_support()
                            .role(Role::Status)
                            .aria_label(error.clone())
                            .text_size(px(12.))
                            .text_color(palette.error)
                            .child(error)
                    })),
            )
            .child(h_flex().flex_shrink_0().gap(px(4.)).children(self.actions))
    }
}
