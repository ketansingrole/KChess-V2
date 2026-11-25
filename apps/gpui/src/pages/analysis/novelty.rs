//! The game's novelty: its first main-line move that no game of the game databases on this
//! computer played (Your games aside), marked N in the move list as ChessBase marks one, with
//! the games that reached the position before it and what they played. It is looked up again
//! when the main line, the databases or the help allowed during a live game change.

use super::*;
use crate::log::ResultExt as _;
use kchess_contracts::ErrorKind;
use kchess_contracts::check::{DatabaseId, Fen, GameDbGameId, Moves, PlayerName};
use kchess_contracts::types::DatabaseSource;
use kchess_contracts::types::GameDbNovelty;

/// The line settles before it is looked up: a game played out move by move asks once.
const NOVELTY_DELAY: Duration = Duration::from_millis(500);

#[derive(Default)]
pub(super) struct Novelty {
    /// The line, databases and origin the answer is for.
    key: String,
    found: Option<GameDbNovelty>,
    task: Option<Task<()>>,
}

/// What the N's tooltip says of the games before it.
pub(super) fn describe(novelty: &GameDbNovelty) -> String {
    if novelty.games == 0 {
        return "Novelty: the games in your databases stopped here".to_string();
    }
    let played: Vec<String> = novelty
        .played
        .iter()
        .map(|played| format!("{} ({})", played.san, played.games))
        .collect();
    format!(
        "Novelty: new to your databases. {} reached this position and played {}",
        crate::widgets::format_counted(novelty.games as f64, "game", "games"),
        played.join(", ")
    )
}

impl AnalysisPage {
    /// The main line's novelty looked up again, once the line, the databases or the help
    /// allowed changed since the last answer.
    pub(super) fn find_novelty(&mut self, cx: &mut Context<Self>) {
        let databases: Vec<(String, u64, i64)> = cx
            .global::<GameDbHandle>()
            .0
            .read(cx)
            .databases()
            .iter()
            .filter(|info| info.source != DatabaseSource::Own)
            .map(|info| (info.id.clone(), info.games, info.updated_at))
            .collect();
        let moves = self.mainline();
        let from = self.database_game().cloned();
        if databases.is_empty()
            || moves.is_empty()
            || !self.standard_rules()
            || !Self::assistance_allowed(cx)
        {
            if self.novelty.found.is_some() || self.novelty.task.is_some() {
                self.novelty = Novelty::default();
                cx.notify();
            }
            return;
        }
        let key = format!("{} {databases:?} {from:?}", self.mainline_key());
        if key == self.novelty.key {
            return;
        }
        self.novelty.key = key;
        self.novelty.found = None;
        let name = |header: String| {
            let header = header.trim().to_string();
            (!header.is_empty() && header != "?").then_some(PlayerName(header))
        };
        let call = calls::GameDbNovelty {
            fen: Fen(self.root.fen.clone()),
            moves: Moves(moves),
            white: name(self.header("White")),
            black: name(self.header("Black")),
            database: from.as_ref().map(|from| DatabaseId(from.database.clone())),
            game: from.as_ref().map(|from| GameDbGameId(from.game)),
        };
        let core = cx.global::<AppCore>().clone();
        self.novelty.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NOVELTY_DELAY).await;
            let answer = core.lookup(call).await;
            this.update(cx, |this, cx| {
                this.novelty.task = None;
                // Withheld while a live game runs: asked again once the help is lent.
                let withheld =
                    matches!(&answer, Err(cause) if cause.kind == Some(ErrorKind::Withheld));
                if withheld {
                    this.novelty.key.clear();
                } else {
                    this.novelty.found = answer.warn_on_err();
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// The main-line move marked N, by its path, with what its tooltip says.
    pub(super) fn novelty_mark(&self) -> Option<(String, SharedString)> {
        let found = self.novelty.found.as_ref()?;
        let ply = usize::try_from(found.ply?).ok()?;
        let moves = self.mainline();
        let line = moves.get(..=ply)?;
        Some((path_of(line), describe(found).into()))
    }
}

#[cfg(test)]
mod tests {
    use super::describe;
    use kchess_contracts::types::{GameDbNovelty, NoveltyMove};

    #[test]
    fn the_tooltip_names_the_games_before_and_their_moves() {
        let found = GameDbNovelty {
            ply: Some(5),
            games: 12,
            played: vec![
                NoveltyMove {
                    san: "a6".into(),
                    uci: "a7a6".into(),
                    games: 9,
                },
                NoveltyMove {
                    san: "Nf6".into(),
                    uci: "g8f6".into(),
                    games: 3,
                },
            ],
        };
        assert_eq!(
            describe(&found),
            "Novelty: new to your databases. 12 games reached this position and played a6 (9), Nf6 (3)"
        );
        let ended = GameDbNovelty {
            ply: Some(5),
            ..GameDbNovelty::default()
        };
        assert_eq!(
            describe(&ended),
            "Novelty: the games in your databases stopped here"
        );
    }
}
