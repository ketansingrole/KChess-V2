//! The board kept in a study: saving to its chapter, opening, saving as and closing a study.

use super::*;
use crate::game::tree::TreePgn as _;
use kchess_contracts::types::{StudyCommand, StudyCommandResult};

impl AnalysisPage {
    /// Write the board to its study when it changed there (`persistStudy`); the answer, for a
    /// quit to wait on.
    pub(super) fn persist_study(&mut self, cx: &mut Context<Self>) -> Option<StudySave> {
        self.study_save = None;
        if self.study.is_empty() {
            return None;
        }
        let library = cx.global::<StudyHandle>().0.clone();
        let name = library.read(cx).study(&self.study)?["name"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let text = self.root.pgn_text();
        if text == self.study_pgn {
            return None;
        }
        // Counted as saved while the core writes it; a failure makes it unsaved again.
        let previous = std::mem::replace(&mut self.study_pgn, text.clone());
        let (study, chapter) = (self.study.clone(), self.chapter.clone());
        let command = library.read(cx).command(StudyCommand::Save {
            name,
            pgn: text.clone(),
            id: Some(self.study.clone()),
            chapter_id: Some(self.chapter.clone()),
        });
        let answer = (Box::pin(command) as std::pin::Pin<Box<dyn Future<Output = _>>>).shared();
        let outcome = answer.clone();
        self.study_writes += 1;
        let write = self.study_writes;
        self.study_writing = Some((write, answer.clone()));
        cx.spawn(async move |this, cx| {
            let result: Result<StudyCommandResult, String> = outcome.await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .study_writing
                    .as_ref()
                    .is_some_and(|(known, _)| *known == write)
                {
                    this.study_writing = None;
                }
                match result {
                    Ok(_) => this.study_error = None,
                    // Another window changed the chapter: the board keeps its moves as an unsaved
                    // copy rather than overwriting that window's, or blocking every later save.
                    Err(cause)
                        if crate::studies::is_stale(cause.as_str())
                            && this.study == study
                            && this.chapter == chapter =>
                    {
                        this.study.clear();
                        this.chapter.clear();
                        this.study_pgn.clear();
                        this.study_error = Some(STALE_COPY.to_string());
                    }
                    // The failure is the study's: shown while the board still holds it, and not on a
                    // game or study opened since.
                    Err(cause) if this.study == study && this.chapter == chapter => {
                        if this.study_pgn == text {
                            this.study_pgn = previous;
                        }
                        this.study_error = Some(cause);
                    }
                    Err(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
        Some(answer)
    }

    /// Every edit so far written to the study, or why it could not be.
    pub fn flush_study(
        &mut self,
        cx: &mut Context<Self>,
    ) -> impl Future<Output = Result<(), String>> + 'static + use<> {
        // Nothing new to write: the write still on its way (an autosave) is what to wait for.
        let save = self
            .persist_study(cx)
            .or_else(|| self.study_writing.as_ref().map(|(_, save)| save.clone()));
        async move {
            match save {
                Some(save) => save.await.map(|_| ()),
                None => Ok(()),
            }
        }
    }

    /// Show a study's chapter (its first by default); its edits are saved to it from now on.
    pub fn open_study(
        &mut self,
        id: &str,
        chapter: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let library = cx.global::<StudyHandle>().0.clone();
        let found = library.read(cx).study(id).cloned();
        let chapters = found
            .as_ref()
            .and_then(|study| study["chapters"].as_array().cloned())
            .unwrap_or_default();
        let opened = chapters
            .iter()
            .find(|entry| Some(entry["id"].as_str().unwrap_or_default()) == chapter)
            .or_else(|| chapters.first())
            .cloned();
        let tree = opened
            .as_ref()
            .and_then(|chapter| chapter["pgn"].as_str())
            .and_then(TreeNode::from_pgn);
        let (Some(opened), Some(tree)) = (opened, tree) else {
            return Err("This study could not be opened".into());
        };
        self.take_board(tree, cx);
        self.study = id.to_string();
        self.chapter = opened["id"].as_str().unwrap_or_default().to_string();
        library.read(cx).track(&self.study, &self.chapter);
        self.study_pgn = self.root.pgn_text();
        self.study_error = None;
        self.changed(cx);
        Ok(())
    }

    /// Keep the board as a new study (named), and save its edits to it from now on.
    pub fn save_as_study(
        &mut self,
        name: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<String, String>> {
        self.persist_study(cx);
        self.navigation += 1;
        let navigation = self.navigation;
        let text = self.root.pgn_text();
        let library = cx.global::<StudyHandle>().0.clone();
        let answer = library.read(cx).command(StudyCommand::Save {
            name,
            pgn: text.clone(),
            id: None,
            chapter_id: None,
        });
        cx.spawn(async move |this, cx| {
            let result = answer.await?;
            let id = result.id.clone().ok_or("The study could not be saved")?;
            this.update(cx, |this, cx| {
                library.update(cx, |library, cx| {
                    library.apply(&result);
                    cx.notify();
                });
                // The new study stays in the library either way; only the board it was made
                // from links to it.
                if this.navigation != navigation {
                    return Ok(id.clone());
                }
                let chapter = library
                    .read(cx)
                    .study(&id)
                    .and_then(|study| study["chapters"][0]["id"].as_str().map(str::to_string));
                let Some(chapter) = chapter else {
                    return Err("The saved study has no chapter".to_string());
                };
                this.study = id.clone();
                this.chapter = chapter;
                this.study_pgn = text;
                this.study_error = None;
                this.schedule_save(cx);
                cx.notify();
                Ok(id)
            })
            .map_err(|_| "The analysis board closed".to_string())?
        })
    }

    /// Save the open study, then return to an empty board; the study stays open when it cannot
    /// be saved.
    pub fn close_study(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        let flush = self.flush_study(cx);
        let navigation = self.navigation;
        cx.spawn(async move |this, cx| {
            let saved = flush.await;
            this.update(cx, |this, cx| match saved {
                // A game or study opened while it saved has the board now: it stays.
                _ if this.navigation != navigation => true,
                Ok(()) => {
                    this.study.clear();
                    this.chapter.clear();
                    this.study_pgn.clear();
                    this.load(None, cx);
                    true
                }
                Err(cause) => {
                    this.study_error = Some(cause);
                    cx.notify();
                    false
                }
            })
            .unwrap_or(false)
        })
    }

    /// Link the board to a study again (a deleted one restored), while the board still holds it.
    pub fn relink(&mut self, id: String, chapter: String, navigation: u64, cx: &mut Context<Self>) {
        if self.navigation == navigation && self.study.is_empty() {
            cx.global::<StudyHandle>().0.read(cx).track(&id, &chapter);
            self.study = id;
            self.chapter = chapter;
            self.schedule_save(cx);
            cx.notify();
        }
    }
}
