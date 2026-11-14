//! A game's place in Game history as every frontend keeps it: the archive rules
//! (`kchess_domain::games::archive`) decide when the game is saved, replaced or dropped, and the
//! frontend carries out the writes they ask for, in order.

use kchess_domain::games::archive::{Effect, Host, Identity, Snapshot, State};
use kchess_domain::library::PlayedGame;
use kchess_domain::misc::constants::{HistoryKind, SessionKind};

/// A write to the profile's library a Game history place asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Write {
    /// Save (or replace) a played game.
    SaveGame(Box<PlayedGame>),
    /// Remove the played game with this id.
    RemoveGame(String),
    /// Keep the place's identity as the `kind` session.
    SaveSession {
        kind: SessionKind,
        identity: Identity,
    },
}

/// The place in Game history of a `kind` game.
pub struct HistoryPlace {
    kind: HistoryKind,
    archive: State,
    saved_identity: Option<Identity>,
}

/// What the archive is told of the game now.
fn host<'a>(source: Option<&'a Snapshot>, has_play: bool, games: &'a [PlayedGame]) -> Host<'a> {
    Host {
        has_play,
        source,
        games,
        now: crate::now_ms(),
        new_id: new_id(),
    }
}

fn new_id() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // A clock-based id is still unique enough for one profile's history.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        bytes = nanos.to_le_bytes();
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

impl HistoryPlace {
    /// The place kept by `identity` (its kind's `archive:` session, none when there is none),
    /// among the played `games`; `resume` continues the saved game's entry instead of starting a
    /// new one.
    pub fn new(
        kind: HistoryKind,
        identity: Option<Identity>,
        resume: bool,
        source: Option<Snapshot>,
        has_play: bool,
        games: &[PlayedGame],
    ) -> (HistoryPlace, Vec<Write>) {
        let host = host(source.as_ref(), has_play, games);
        let mut place = HistoryPlace {
            kind,
            archive: State::new(identity.clone(), &host),
            saved_identity: identity,
        };
        let effects = place.archive.init(resume, &host);
        let writes = place.writes(effects);
        (place, writes)
    }

    /// The game changed; `finished` saves it as it is now.
    pub fn save(
        &mut self,
        finished: bool,
        source: Option<Snapshot>,
        has_play: bool,
        games: &[PlayedGame],
    ) -> Vec<Write> {
        self.run(
            |archive, host| archive.save(finished, host),
            source.as_ref(),
            has_play,
            games,
        )
    }

    /// A new game begins: the one before keeps its place and this one gets its own.
    pub fn reset(
        &mut self,
        source: Option<Snapshot>,
        has_play: bool,
        games: &[PlayedGame],
    ) -> Vec<Write> {
        self.run(
            |archive, host| archive.reset(host),
            source.as_ref(),
            has_play,
            games,
        )
    }

    fn run(
        &mut self,
        step: impl FnOnce(&mut State, &Host<'_>) -> Vec<Effect>,
        source: Option<&Snapshot>,
        has_play: bool,
        games: &[PlayedGame],
    ) -> Vec<Write> {
        let effects = step(&mut self.archive, &host(source, has_play, games));
        self.writes(effects)
    }

    /// The archive's effects as writes, then the identity when it changed.
    fn writes(&mut self, effects: Vec<Effect>) -> Vec<Write> {
        let mut writes: Vec<Write> = effects
            .into_iter()
            .map(|effect| match effect {
                Effect::Save(game) => Write::SaveGame(game),
                Effect::Remove(id) => Write::RemoveGame(id),
            })
            .collect();
        let identity = &self.archive.identity;
        if self.saved_identity.as_ref() != Some(identity) {
            self.saved_identity = Some(identity.clone());
            writes.push(Write::SaveSession {
                kind: self.kind.session(),
                identity: identity.clone(),
            });
        }
        writes
    }
}

/// Keep `games` (the played games a frontend holds) as the writes leave them.
pub fn apply_to(games: &mut Vec<PlayedGame>, writes: &[Write]) {
    for write in writes {
        match write {
            Write::SaveGame(game) => {
                games.retain(|kept| kept.id != game.id);
                games.push((**game).clone());
            }
            Write::RemoveGame(id) => games.retain(|kept| kept.id != *id),
            Write::SaveSession { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_version_4_uuids() {
        let ids: Vec<String> = (0..64).map(|_| new_id()).collect();
        for id in &ids {
            let chars: Vec<char> = id.chars().collect();
            assert_eq!(chars.len(), 36, "{id}");
            assert_eq!(chars[14], '4', "{id}");
            assert!("89ab".contains(chars[19]), "{id}");
        }
        assert!(
            ids.iter()
                .any(|id| id.as_bytes()[15] != ids[0].as_bytes()[15])
        );
    }

    /// Restored after a restart, a game whose only move is taken back leaves no entry behind.
    #[test]
    fn a_resumed_game_taken_back_to_its_start_leaves_no_entry() {
        let identity = Some(Identity {
            id: "resumed-game".into(),
            started_at: 1.0e12,
        });
        let saved = vec![played("resumed-game")];
        let source = Some(Snapshot {
            source: HistoryKind::Computer,
            setup: kchess_domain::variant::Setup::standard(),
            moves: Vec::new(),
            white: "You".into(),
            black: "Stockfish".into(),
            result: "*".into(),
            reason: "In progress".into(),
            time_control: "-".into(),
            clock_summary: None,
        });
        let (mut place, writes) = HistoryPlace::new(
            HistoryKind::Computer,
            identity,
            true,
            source.clone(),
            false,
            &saved,
        );
        assert_eq!(writes, Vec::new());
        let writes = place.save(false, source, false, &saved);
        assert_eq!(writes, vec![Write::RemoveGame("resumed-game".into())]);
    }

    /// A new place keeps its identity once; a game with a move is saved under it, and the
    /// identity is not written again until a new game gets its own.
    #[test]
    fn the_identity_is_written_only_when_it_changes() {
        let mut games = Vec::new();
        let mut game = kchess_domain::games::local::LocalGame::resume(None, 0.).unwrap();
        let (mut place, writes) = HistoryPlace::new(
            HistoryKind::Board,
            None,
            true,
            Some(game.archive_snapshot()),
            false,
            &games,
        );
        let [Write::SaveSession { kind, identity }] = writes.as_slice() else {
            panic!("a new place keeps its identity: {writes:?}");
        };
        assert_eq!(*kind, SessionKind::ArchiveBoard);
        game.play("e2e4", 1.);
        let source = Some(game.archive_snapshot());
        let writes = place.save(false, source.clone(), true, &games);
        assert!(
            matches!(writes.first(), Some(Write::SaveGame(game)) if game.id == identity.id),
            "{writes:?}"
        );
        apply_to(&mut games, &writes);
        assert_eq!(games.len(), 1);
        let writes = place.save(false, source.clone(), true, &games);
        assert!(
            writes
                .iter()
                .all(|write| !matches!(write, Write::SaveSession { .. })),
            "{writes:?}"
        );
        let writes = place.reset(source, true, &games);
        assert!(
            writes.iter().any(|write| matches!(
                write,
                Write::SaveSession { identity: next, .. } if next.id != identity.id
            )),
            "{writes:?}"
        );
    }

    #[test]
    fn removed_games_leave_the_list() {
        let mut games = vec![played("a"), played("b")];
        apply_to(&mut games, &[Write::RemoveGame("a".into())]);
        assert_eq!(games, [played("b")]);
    }

    /// Saving a game replaces the entry with its id, moved to the end, and keeps the others.
    #[test]
    fn a_saved_game_replaces_its_own_entry_only() {
        let mut games = vec![played("a"), played("b")];
        let mut longer = played("a");
        longer.moves.push("e7e5".into());
        apply_to(&mut games, &[Write::SaveGame(Box::new(longer.clone()))]);
        assert_eq!(games, [played("b"), longer]);
    }

    /// An unfinished game with one move, saved as `id`.
    fn played(id: &str) -> PlayedGame {
        PlayedGame {
            id: id.into(),
            source: HistoryKind::Computer,
            started_at: 1.0e12,
            updated_at: 1.0e12,
            white: "You".into(),
            black: "Stockfish".into(),
            result: "*".into(),
            reason: "In progress".into(),
            finished: false,
            setup: kchess_domain::variant::Setup::standard(),
            moves: vec!["e2e4".into()],
            time_control: "-".into(),
            clock_summary: None,
        }
    }
}
