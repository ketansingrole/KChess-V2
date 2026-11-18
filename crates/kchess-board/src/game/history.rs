//! A game's place in Game history, as a page keeps it: `kchess_games::history` decides, and the
//! played games (`played.rs`) carry out its writes.

use gpui_kit::App;
use kchess_contracts::types::{HistoryKind, SessionDocument};
use kchess_domain::games::archive::{Identity, Snapshot};
use kchess_games::history::Write;

use crate::played::PlayedHandle;

pub struct HistoryPlace(kchess_games::history::HistoryPlace);

impl HistoryPlace {
    /// The place of a `kind` game kept by `identity` (its kind's `archive:` session, none when
    /// there is none); `resume` continues the saved game's entry instead of starting a new one.
    pub fn new(
        kind: HistoryKind,
        identity: Option<Identity>,
        resume: bool,
        source: Snapshot,
        has_play: bool,
        cx: &mut App,
    ) -> HistoryPlace {
        let (place, writes) = kchess_games::history::HistoryPlace::new(
            kind,
            identity,
            resume,
            Some(source),
            has_play,
            cx.global::<PlayedHandle>().0.read(cx).games(),
        );
        write(writes, cx);
        HistoryPlace(place)
    }

    /// The game changed; `finished` saves it as it is now.
    pub fn save(&mut self, finished: bool, source: Snapshot, has_play: bool, cx: &mut App) {
        let games = cx.global::<PlayedHandle>().0.read(cx).games();
        let writes = self.0.save(finished, Some(source), has_play, games);
        write(writes, cx);
    }

    /// A new game begins: the one before keeps its place and this one gets its own.
    pub fn reset(&mut self, source: Snapshot, has_play: bool, cx: &mut App) {
        let games = cx.global::<PlayedHandle>().0.read(cx).games();
        let writes = self.0.reset(Some(source), has_play, games);
        write(writes, cx);
    }
}

/// Carry out the writes in order, through the played games' ordered writer.
fn write(writes: Vec<Write>, cx: &mut App) {
    let played = cx.global::<PlayedHandle>().0.clone();
    played.update(cx, |played, cx| {
        for write in writes {
            match write {
                Write::SaveGame(game) => played.save(*game, cx),
                Write::RemoveGame(id) => {
                    played.remove(&id, cx);
                }
                Write::SaveSession { kind, identity } => {
                    played.save_session(kind, SessionDocument::Archive(identity));
                }
            }
        }
    });
}
