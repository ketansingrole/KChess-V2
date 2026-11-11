//! The analysis board as the shell drives it: its name, its life as it is shown and left, and
//! its actions with the study bar's.

use gpui_kit::*;

use super::AnalysisPage;
use crate::pages::study_bar::StudyAction;
use crate::pages::{ActionInfo, ActionRole, NavGroup, Page, PageAction, PageInfo, PageView};

/// The board's own actions, as its menu and keys offer them, and the study bar's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Action {
    Flip,
    CopyFen,
    CopyPgn,
    CopyLine,
    Import,
    /// The study bar's, and whether the study is a Lichess study's offline copy.
    Study(StudyAction, bool),
}

impl PageAction for Action {
    fn info(&self) -> ActionInfo {
        match *self {
            Action::Flip => ActionInfo::flip("analysis:flip"),
            Action::CopyFen => ActionInfo::new(
                "analysis:copy-fen",
                "Copy FEN",
                "copy",
                "position clipboard",
            ),
            Action::CopyPgn => ActionInfo::new(
                "analysis:copy-pgn",
                "Copy PGN",
                "file-text",
                "game moves clipboard",
            )
            .role(ActionRole::Copy),
            Action::CopyLine => ActionInfo::new(
                "analysis:copy-line",
                "Copy line",
                "list-tree",
                "variation moves pgn clipboard",
            ),
            Action::Import => ActionInfo::new(
                "analysis:import",
                "Import a position or game…",
                "import",
                "paste pgn fen link",
            ),
            Action::Study(action, cloud) => study_info(action, cloud),
        }
    }
}

fn study_info(action: StudyAction, cloud: bool) -> ActionInfo {
    let (id, label, glyph, keywords) = match action {
        StudyAction::SaveAsStudy => ("save", "Save as study", "book-plus", "keep store"),
        StudyAction::Details => (
            "details",
            "Game details",
            "file-pen-line",
            "headers pgn tags",
        ),
        StudyAction::SaveCopy => ("copy", "Save a copy", "copy", "duplicate study"),
        StudyAction::Close => ("close", "Close study", "book-x", "unsaved analysis"),
        StudyAction::Remove if cloud => {
            ("remove", "Remove offline copy", "trash-2", "lichess study")
        }
        StudyAction::Remove => ("remove", "Delete study", "trash-2", "remove"),
        StudyAction::AddChapter => ("add-chapter", "Add chapter", "plus", "new"),
        StudyAction::RenameChapter => ("rename-chapter", "Rename chapter", "pencil", "name"),
        StudyAction::DuplicateChapter => ("duplicate-chapter", "Duplicate chapter", "copy", "copy"),
        StudyAction::MoveChapterUp => (
            "move-chapter-up",
            "Move chapter up",
            "chevron-up",
            "reorder",
        ),
        StudyAction::MoveChapterDown => (
            "move-chapter-down",
            "Move chapter down",
            "chevron-down",
            "reorder",
        ),
        StudyAction::RemoveChapter => ("delete-chapter", "Delete chapter", "trash-2", "remove"),
        StudyAction::PreviousChapter => (
            "previous-chapter",
            "Previous chapter",
            "chevron-left",
            "back",
        ),
        StudyAction::NextChapter => ("next-chapter", "Next chapter", "chevron-right", "forward"),
    };
    ActionInfo::new(format!("study:{id}"), label, glyph, keywords)
}

impl PageView for AnalysisPage {
    const INFO: PageInfo = PageInfo {
        page: Page::Analysis,
        label: "Analysis board",
        short: "Analysis board",
        icon: "microscope",
        group: Some(NavGroup::Review),
        keywords: "engine evaluate position fen pgn",
    };

    type Action = Action;

    fn create(window: &mut Window, cx: &mut Context<Self>) -> Self {
        AnalysisPage::new(window, cx)
    }

    /// The board's engine follows the board only while it is shown.
    fn shown(this: &Entity<Self>, _again: bool, _window: &mut Window, cx: &mut App) {
        this.update(cx, |analysis, cx| analysis.attach(cx));
    }

    fn hidden(this: &Entity<Self>, _window: &mut Window, cx: &mut App) {
        this.update(cx, |analysis, cx| analysis.detach(cx));
    }

    fn actions(&self, cx: &App) -> Vec<Action> {
        let bar = self.study_bar();
        let bar = bar.read(cx);
        let cloud = bar.cloud(cx);
        [
            Action::Flip,
            Action::CopyFen,
            Action::CopyPgn,
            Action::CopyLine,
            Action::Import,
        ]
        .into_iter()
        .chain(
            bar.actions(cx)
                .into_iter()
                .map(|action| Action::Study(action, cloud)),
        )
        .collect()
    }

    fn act(this: &Entity<Self>, action: Action, window: &mut Window, cx: &mut App) {
        if let Action::Study(action, _) = action {
            let bar = this.read(cx).study_bar();
            bar.update(cx, |bar, cx| bar.act(action, window, cx));
            return;
        }
        this.update(cx, |board, cx| match action {
            Action::Flip => board.flip(cx),
            Action::CopyFen => board.copy_fen(window, cx),
            Action::CopyPgn => board.copy_pgn(window, cx),
            Action::CopyLine => board.copy_shown_line(window, cx),
            Action::Import => board.open_import(window, cx),
            Action::Study(..) => {}
        });
    }

    /// New on the analysis board is a fresh board, asking first when that would lose
    /// unsaved analysis.
    fn new_here(_this: &Entity<Self>, window: &mut Window, cx: &mut App) {
        super::new_board(window, cx);
    }
}
