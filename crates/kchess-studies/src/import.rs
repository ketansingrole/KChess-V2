//! `Import a study`: its dialog, and the chapters and name read from the PGN pasted in it.

use super::*;

/// Why an import was refused.
pub(super) const INVALID_IMPORT: &str = "Paste valid PGN with up to 64 chapters";
pub(super) const VARIANT_IMPORT: &str = "A chapter is in a variant KChess doesn't play";

/// An import's chapters: each game named by its `ChapterName`, or numbered.
pub(super) fn import_chapters(text: &str) -> Result<Vec<(String, String)>, &'static str> {
    let chapters: Vec<(String, String)> = kchess_domain::position::pgn_games(text)
        .into_iter()
        .enumerate()
        .map(|(index, game)| {
            let name = game
                .headers
                .iter()
                .find(|(name, _)| name == "ChapterName")
                .map_or_else(
                    || format!("Chapter {}", index + 1),
                    |(_, value)| value.clone(),
                );
            (name, game.pgn)
        })
        .collect();
    // A game read under rules other than its own would show positions it never had.
    if chapters
        .iter()
        .any(|(_, pgn)| !crate::game::pgn_analysable(pgn))
    {
        return Err(VARIANT_IMPORT);
    }
    let valid = !chapters.is_empty()
        && chapters.len() <= MAX_CHAPTERS
        && chapters
            .iter()
            .all(|(_, pgn)| TreeNode::from_pgn(pgn).is_some());
    if valid {
        Ok(chapters)
    } else {
        Err(INVALID_IMPORT)
    }
}

/// An imported study's name when none is given: its players, else its event.
pub(super) fn import_name(first: &str) -> Option<String> {
    let headers = TreeNode::from_pgn(first)?
        .headers
        .take()
        .unwrap_or_default();
    let header = |key: &str| {
        headers
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && *value != "?")
            .map(str::to_string)
    };
    match (header("White"), header("Black")) {
        (Some(white), Some(black)) => Some(format!("{white} – {black}")),
        _ => header("Event"),
    }
}

impl StudiesPage {
    pub(crate) fn open_import(window: &mut Window, cx: &mut App) {
        Self::open_import_with(None, window, cx);
    }

    /// The import dialog holding `pgn` already, as a file of several games dropped on the window
    /// offers it.
    pub fn open_import_with(pgn: Option<String>, window: &mut Window, cx: &mut App) {
        let import = cx.new(|cx| ImportStudy::new(window, cx));
        if let Some(pgn) = pgn {
            import.update(cx, |import, cx| {
                import
                    .text
                    .update(cx, |text, cx| text.set_value(pgn, window, cx));
            });
        }
        crate::widgets::open_dialog(window, cx, move |dialog, _, _| {
            dialog
                .title("Import a study")
                .w(px(512.))
                .child(import.clone())
        });
    }
}

/// `Import a study`: a name (taken from the PGN when left empty) and PGN chapters.
pub(super) struct ImportStudy {
    name: Entity<InputState>,
    text: Entity<TextareaState>,
    error: Option<SharedString>,
    _subscription: Subscription,
}

impl ImportStudy {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> ImportStudy {
        let name = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Name (optional: taken from the PGN)")
        });
        let text = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(8)
                .placeholder("[Event \"…\"]\n1. e4 e5 2. Nf3 { The main idea } Nc6 (2... d6) *")
        });
        let subscription = cx.observe(&text, |_, _, cx| cx.notify());
        ImportStudy {
            name,
            text,
            error: None,
            _subscription: subscription,
        }
    }

    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text.read(cx).value().trim().to_string();
        let chapters = match import_chapters(&text) {
            Ok(chapters) => chapters,
            Err(problem) => {
                self.error = Some(problem.into());
                cx.notify();
                return;
            }
        };
        let library = StudiesPage::library(cx);
        let given = self.name.read(cx).value().trim().to_string();
        let name = if given.is_empty() {
            import_name(&chapters[0].1)
                .unwrap_or_else(|| library.read(cx).fresh_name("Imported study"))
        } else {
            given
        };
        let answer = library.read(cx).command(StudyCommand::SaveChapters {
            name,
            chapters: chapters
                .iter()
                .map(|(name, pgn)| ChapterInput {
                    name: name.clone(),
                    pgn: pgn.clone(),
                })
                .collect(),
            id: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = answer.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(result) => {
                    library.update(cx, |library, cx| {
                        library.apply(&result);
                        cx.notify();
                    });
                    window.close_dialog(cx);
                    let id = result.id.unwrap_or_default();
                    window.push_notification(
                        widgets::action_notification(
                            Notification::success("Study imported"),
                            "open-imported",
                            "Open",
                            move |window, cx| StudiesPage::open(id.clone(), window, cx),
                        ),
                        cx,
                    );
                }
                Err(cause) => {
                    crate::log::write(Level::Warn, "studies", &format!("Import failed: {cause}"));
                    this.error = Some(cause.into());
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl Render for ImportStudy {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let empty = self.text.read(cx).value().trim().is_empty();
        v_flex()
            .gap(px(12.))
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(palette.text_muted)
                    .child("Paste PGN chapters"),
            )
            .child(Input::new(&self.name).aria_label("Study name"))
            .child(Textarea::new(&self.text).aria_label("PGN"))
            .child(widgets::dialog_footer(
                self.error
                    .clone()
                    .map(|error| ("import-study-error", error)),
                None,
                [
                    Button::new("import-cancel")
                        .ghost()
                        .label("Cancel")
                        .on_click(|_, window, cx| window.close_dialog(cx))
                        .into_any_element(),
                    Button::new("import-run")
                        .primary()
                        .label("Import")
                        .disabled(empty)
                        .on_click(cx.listener(|this, _, window, cx| this.run(window, cx)))
                        .into_any_element(),
                ],
                &palette,
            ))
    }
}
