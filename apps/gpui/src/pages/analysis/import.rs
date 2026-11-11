//! Importing onto the analysis board: pasted or dropped PGN and FEN, files, and Lichess games
//! and studies fetched by link.

use super::Opening;
use super::*;
use crate::game::played::GameOrigin;
use kchess_contracts::calls;

impl AnalysisPage {
    pub(crate) fn open_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.import_error = None;
        self.import
            .update(cx, |text, cx| text.set_value("", window, cx));
        let page = cx.entity().downgrade();
        let import = self.import.clone();
        crate::widgets::open_dialog(window, cx, move |dialog, _, cx| {
            let (error, importing) = page.upgrade().map_or((None, false), |page| {
                let page = page.read(cx);
                (page.import_error.clone(), page.importing.is_some())
            });
            let (run, text) = (page.clone(), import.clone());
            dialog.title("Import a position or game").w(px(512.)).child(
                v_flex()
                    .gap(px(12.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(cx.global::<Palette>().text_muted)
                            .child("Paste a FEN, a PGN, or a Lichess game or study link"),
                    )
                    .child(Textarea::new(&import).aria_label("FEN, PGN or Lichess link"))
                    .children(error.map(|error| {
                        // Spoken when it appears: a link's fetch fails after Import was pressed.
                        div()
                            .id("import-error")
                            .test_support()
                            .aria_label(error.clone())
                            .relative()
                            .text_size(px(14.))
                            .text_color(cx.global::<Palette>().error)
                            .child(crate::widgets::live_region(
                                "import-error-spoken",
                                error.clone(),
                            ))
                            .child(error)
                    }))
                    .child(
                        h_flex()
                            .justify_end()
                            .gap(px(8.))
                            .child({
                                let page = run.clone();
                                Button::new("import-file")
                                    .ghost()
                                    .icon(icon("folder-open"))
                                    .label("Open file…")
                                    .on_click(move |_, window, cx| {
                                        let _ = page
                                            .update(cx, |page, cx| page.choose_file(window, cx));
                                    })
                            })
                            .child(div().flex_1())
                            .child({
                                let page = run.clone();
                                Button::new("import-cancel")
                                    .ghost()
                                    .label("Cancel")
                                    .on_click(move |_, window, cx| {
                                        // A link still being fetched is dropped with it.
                                        let _ = page.update(cx, |page, _| page.importing = None);
                                        window.close_dialog(cx);
                                    })
                            })
                            .child(
                                Button::new("import-run")
                                    .primary()
                                    .label("Import")
                                    .loading(importing)
                                    .disabled(text.read(cx).value().trim().is_empty())
                                    .on_click(move |_, window, cx| {
                                        let _ =
                                            run.update(cx, |page, cx| page.run_import(window, cx));
                                    }),
                            ),
                    ),
            )
        });
    }

    /// Pick a PGN or FEN file and import it, as if its text were pasted; one that cannot be
    /// read says so under the field.
    pub(super) fn choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            let Some(path) = paths.first() else {
                return;
            };
            let text = crate::links::read_game_file(path);
            let _ = this.update_in(cx, |this, window, cx| match text {
                Some(text) => {
                    this.import
                        .update(cx, |field, cx| field.set_value(text, window, cx));
                    this.run_import(window, cx);
                }
                None => {
                    this.import_error = Some("Choose a PGN or FEN file under 2 MB".into());
                    window.refresh();
                }
            });
        })
        .detach();
    }

    /// Import `text` as the import dialog would, from a file dropped on the window: the dialog
    /// closes once the board has it, or stays open with the text and why it was refused.
    pub(crate) fn import_text(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_import(window, cx);
        self.import
            .update(cx, |field, cx| field.set_value(text, window, cx));
        self.run_import(window, cx);
    }

    /// `runImport`: a FEN starts a new analysis there; a Lichess game link fetches that game;
    /// otherwise the text is read as one PGN game.
    pub(super) fn run_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.import.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        // A Lichess game or study link is read as Paste and Quick Pick read it.
        match crate::links::classify(&text) {
            Some(crate::links::Link::Game(link)) => return self.import_game(link, window, cx),
            Some(crate::links::Link::StudyChapter { study, chapter }) => {
                return self.import_study(study, chapter, window, cx);
            }
            _ => {}
        }
        self.importing = None;
        if let Some(fen) =
            crate::game::editor_setup(&text).and_then(|setup| crate::game::setup_fen(&setup))
        {
            if let Some(problem) = crate::game::position_problem(&fen) {
                self.import_error = Some(problem.into());
                window.refresh();
                return;
            }
            self.load(Some(&fen), cx);
        } else if !crate::game::pgn_analysable(&text) {
            self.import_error = Some(UNPLAYABLE_VARIANT.into());
            window.refresh();
            return;
        } else if !self.load_pgn(&text, cx) {
            self.import_error =
                Some("Use a valid FEN or one PGN game with legal moves, under 2 MB".into());
            window.refresh();
            return;
        } else if self.root.children.is_empty() {
            self.import_error = Some("No moves could be read from that PGN".into());
            window.refresh();
            return;
        }
        self.import_error = None;
        window.close_dialog(cx);
    }

    /// Fetch a Lichess game by its link and open it facing as linked, at the move linked.
    pub(super) fn import_game(
        &mut self,
        link: crate::game::GameLink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let export = cx
            .global::<AppCore>()
            .request(calls::ExportGame { id: json!(link.id) });
        let pgn = async move {
            match export.await {
                Ok(Value::String(pgn)) => Ok(Some(pgn)),
                Ok(_) => Ok(None),
                Err(cause) => Err(format!("Could not fetch that game: {}", cause.message)),
            }
        };
        let opening = Opening {
            // A link without a move opens at the start, as Lichess does.
            ply: link.ply.unwrap_or(0),
            orientation: Some(link.orientation),
            origin: Some(GameOrigin::lichess(link.id)),
            review: false,
        };
        self.import_fetched(pgn, opening, window, cx);
    }

    /// Fetch a public Lichess study's chapter by its link and open it as the study shows it.
    pub(super) fn import_study(
        &mut self,
        study: String,
        chapter: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fetch = cx.global::<AppCore>().request(calls::LichessStudyChapter {
            id: json!(study),
            chapter: json!(chapter),
        });
        let pgn = async move {
            match fetch.await {
                Ok(found) => Ok(found["pgn"].as_str().map(str::to_string)),
                Err(cause) => Err(format!("Could not fetch that study: {}", cause.message)),
            }
        };
        self.import_fetched(pgn, Opening::default(), window, cx);
    }

    /// Open a game being fetched for the import (`None` when Lichess sent nothing readable).
    pub(super) fn import_fetched(
        &mut self,
        pgn: impl std::future::Future<Output = Result<Option<String>, String>> + 'static,
        mut opening: Opening,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.import_turn += 1;
        let turn = self.import_turn;
        self.importing = Some(turn);
        self.import_error = None;
        window.refresh();
        cx.spawn_in(window, async move |this, cx| {
            let pgn = pgn.await;
            let _ = this.update_in(cx, |this, window, cx| {
                // Dismissed meanwhile (Cancel, Escape or a click outside): nobody waits for it.
                if this.importing != Some(turn) || !window.has_active_dialog(cx) {
                    this.importing = this.importing.filter(|pending| *pending != turn);
                    return;
                }
                this.importing = None;
                let unplayable = matches!(&pgn, Ok(Some(pgn)) if !crate::game::pgn_analysable(pgn));
                let opened = match pgn {
                    Ok(Some(pgn)) => {
                        opening.orientation = opening
                            .orientation
                            .or_else(|| crate::game::pgn_orientation(&pgn));
                        this.open_pgn(&pgn, opening, cx)
                    }
                    Ok(None) => false,
                    Err(message) => {
                        this.import_error = Some(message.into());
                        window.refresh();
                        return;
                    }
                };
                if opened {
                    this.import_error = None;
                    window.close_dialog(cx);
                } else {
                    this.import_error = Some(if unplayable {
                        UNPLAYABLE_VARIANT.into()
                    } else {
                        "That game cannot be opened on the analysis board".into()
                    });
                    window.refresh();
                }
            });
        })
        .detach();
    }
}

/// Why a game in another variant is not imported.
pub(super) const UNPLAYABLE_VARIANT: &str = "KChess doesn't play this variant";
