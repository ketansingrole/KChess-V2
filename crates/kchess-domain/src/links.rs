//! Lichess links pasted into a frontend: which game, study, broadcast, puzzle or player a
//! lichess.org address names. The app's import, Quick Pick and the command line share them.

/// A Lichess game link pasted in place of a PGN: the game's id, the side the link looks from
/// (`/black`), and the move it points at (`#23`).
#[derive(Debug, PartialEq)]
pub struct GameLink {
    pub id: String,
    pub orientation: shakmaty::Color,
    pub ply: Option<usize>,
}

/// What follows `lichess.org/` in a link, with or without its scheme and `www.`.
pub fn lichess_path(text: &str) -> Option<&str> {
    let text = text.trim();
    let rest = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))
        .unwrap_or(text);
    rest.strip_prefix("www.")
        .unwrap_or(rest)
        .strip_prefix("lichess.org/")
}

/// `USERNAME`: what Lichess accepts as a name.
pub fn is_username(name: &str) -> bool {
    (2..=30).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The player a lichess.org profile link names (`lichess.org/@/DrNykterstein`, with `/all`,
/// `/perf/blitz` and the like after it).
pub fn profile_link(text: &str) -> Option<String> {
    let path = lichess_path(text)?.strip_prefix("@/")?;
    let name = path.split(['/', '?', '#']).next().unwrap_or_default();
    is_username(name).then(|| name.to_string())
}

/// A Lichess study link (`lichess.org/study/STUDYID`, with `/CHAPTERID` after it): the study's
/// id and the chapter's, empty for its first.
pub fn study_link(text: &str) -> Option<(String, String)> {
    let id = |part: &str| part.len() == 8 && part.bytes().all(|byte| byte.is_ascii_alphanumeric());
    let path = lichess_path(text)?.strip_prefix("study/")?;
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    match parts.as_slice() {
        [study] if id(study) => Some(((*study).to_string(), String::new())),
        [study, chapter] if id(study) && id(chapter) => {
            Some(((*study).to_string(), (*chapter).to_string()))
        }
        _ => None,
    }
}

/// The side a Lichess export says to show (`[Orientation "black"]`).
pub fn pgn_orientation(pgn: &str) -> Option<shakmaty::Color> {
    match pgn
        .lines()
        .find_map(|line| line.trim().strip_prefix("[Orientation "))?
    {
        tag if tag.contains("black") => Some(shakmaty::Color::Black),
        tag if tag.contains("white") => Some(shakmaty::Color::White),
        _ => None,
    }
}

/// A Lichess broadcast link: a tournament's (`broadcast/slug/TOURID`), or a round's
/// (`broadcast/slug/round-slug/ROUNDID`, with a board's game id after it).
#[derive(Debug, PartialEq)]
pub enum BroadcastLink {
    Tour(String),
    Round { round: String, game: String },
}

pub fn broadcast_link(text: &str) -> Option<BroadcastLink> {
    let id = |part: &str| part.len() == 8 && part.bytes().all(|byte| byte.is_ascii_alphanumeric());
    let path = lichess_path(text)?.strip_prefix("broadcast/")?;
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    match parts.as_slice() {
        // `broadcast/by/someone` lists a user's broadcasts.
        ["by", ..] => None,
        [_, tour] if id(tour) => Some(BroadcastLink::Tour((*tour).to_string())),
        [_, _, round] if id(round) => Some(BroadcastLink::Round {
            round: (*round).to_string(),
            game: String::new(),
        }),
        [_, _, round, game] if id(round) && id(game) => Some(BroadcastLink::Round {
            round: (*round).to_string(),
            game: (*game).to_string(),
        }),
        _ => None,
    }
}

/// The puzzle a lichess.org/training link names (five letters and digits). Theme pages share the
/// shape (`training/fork`), so a frontend that knows the themes rules them out.
pub fn training_link(text: &str) -> Option<String> {
    let path = lichess_path(text)?.strip_prefix("training/")?;
    let id = path.split(['?', '#']).next().unwrap_or_default();
    let id = id.strip_suffix('/').unwrap_or(id);
    (id.len() == 5 && id.bytes().all(|byte| byte.is_ascii_alphanumeric())).then(|| id.to_string())
}

/// A Lichess tournament link: an arena's (`tournament/ID`) or a Swiss event's (`swiss/ID`), as
/// the system (`arena` or `swiss`) and the id.
pub fn tournament_link(text: &str) -> Option<(&'static str, String)> {
    let path = lichess_path(text)?;
    let (system, rest) = if let Some(rest) = path.strip_prefix("tournament/") {
        ("arena", rest)
    } else {
        ("swiss", path.strip_prefix("swiss/")?)
    };
    let id = rest.split(['?', '#']).next().unwrap_or_default();
    let id = id.strip_suffix('/').unwrap_or(id);
    // Lists share the shape: `tournament/calendar`, `tournament/featured`.
    (id.len() == 8
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
        && !["calendar", "featured"].contains(&id))
    .then(|| (system, id.to_string()))
}

/// Reads `lichess.org/abcdefgh`, `https://lichess.org/abcdefgh1234/black#12` and the like; `None`
/// for anything else, a study or a puzzle included.
pub fn game_link(text: &str) -> Option<GameLink> {
    const NOT_GAMES: [&str; 9] = [
        "analysis", "training", "practice", "insights", "streamer", "learning", "tutorial",
        "settings", "features",
    ];
    let rest = lichess_path(text)?;
    let (path, anchor) = rest.split_once('#').unwrap_or((rest, ""));
    let path = path.split('?').next().unwrap_or_default();
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let first = parts.next()?;
    let side = parts.next();
    if parts.next().is_some()
        || !matches!(first.len(), 8 | 12)
        || !first.bytes().all(|byte| byte.is_ascii_alphanumeric())
        || NOT_GAMES.contains(&first)
    {
        return None;
    }
    let orientation = match side {
        None | Some("white") => shakmaty::Color::White,
        Some("black") => shakmaty::Color::Black,
        Some(_) => return None,
    };
    Some(GameLink {
        id: first[..8].to_string(),
        orientation,
        ply: anchor.parse().ok(),
    })
}

#[cfg(test)]
mod link_tests {
    use super::{GameLink, game_link, profile_link};

    #[test]
    fn a_broadcast_link_names_its_tournament_or_round() {
        use super::{BroadcastLink, broadcast_link};
        let round = |round: &str, game: &str| {
            Some(BroadcastLink::Round {
                round: round.into(),
                game: game.into(),
            })
        };
        assert_eq!(
            broadcast_link("https://lichess.org/broadcast/european-youth-2026/uyRfZyjV"),
            Some(BroadcastLink::Tour("uyRfZyjV".into()))
        );
        assert_eq!(
            broadcast_link("lichess.org/broadcast/european-youth-2026/round-2/BeCQF8BD"),
            round("BeCQF8BD", "")
        );
        assert_eq!(
            broadcast_link("https://lichess.org/broadcast/e/round-2/BeCQF8BD/AbCd1234#12"),
            round("BeCQF8BD", "AbCd1234")
        );
        assert_eq!(
            broadcast_link("https://lichess.org/broadcast/by/someone1"),
            None
        );
        assert_eq!(broadcast_link("https://lichess.org/broadcast"), None);
        assert_eq!(
            broadcast_link("https://lichess.org/broadcast/calendar"),
            None
        );
        assert_eq!(broadcast_link("https://lichess.org/abcdefgh"), None);
    }

    #[test]
    fn a_study_link_names_its_study_and_chapter() {
        use super::study_link;
        let both = |study: &str, chapter: &str| Some((study.to_string(), chapter.to_string()));
        assert_eq!(
            study_link("https://lichess.org/study/1wzY0YmD/oOEEUJjV"),
            both("1wzY0YmD", "oOEEUJjV")
        );
        assert_eq!(
            study_link("lichess.org/study/1wzY0YmD#3"),
            both("1wzY0YmD", "")
        );
        assert_eq!(study_link("https://lichess.org/study/by/thibault"), None);
        assert_eq!(study_link("https://lichess.org/study/all/popular"), None);
        assert_eq!(study_link("https://lichess.org/study"), None);
        assert_eq!(
            super::pgn_orientation("[Event \"x\"]\n[Orientation \"black\"]\n\n*"),
            Some(shakmaty::Color::Black)
        );
        assert_eq!(super::pgn_orientation("1. e4 *"), None);
    }

    #[test]
    fn a_training_link_names_its_puzzle() {
        use super::training_link;
        assert_eq!(
            training_link("https://lichess.org/training/FPFg0").as_deref(),
            Some("FPFg0")
        );
        assert_eq!(
            training_link("lichess.org/training/FPFg0/?a#b").as_deref(),
            Some("FPFg0")
        );
        assert_eq!(training_link("https://lichess.org/training"), None);
        assert_eq!(training_link("https://lichess.org/training/mateIn2"), None);
        assert_eq!(training_link("FPFg0"), None);
    }

    #[test]
    fn a_tournament_link_names_its_system_and_event() {
        use super::tournament_link;
        let event = |system, id: &str| Some((system, id.to_string()));
        assert_eq!(
            tournament_link("https://lichess.org/tournament/Hb3sLw0T"),
            event("arena", "Hb3sLw0T")
        );
        assert_eq!(
            tournament_link("lichess.org/swiss/j8rtJ5GL/?page=2#x"),
            event("swiss", "j8rtJ5GL")
        );
        for other in [
            "https://lichess.org/tournament/calendar",
            "https://lichess.org/tournament",
            "https://lichess.org/tournament/Hb3sLw0T/extra",
            "https://lichess.org/swiss",
            "https://lichess.org/Hb3sLw0T",
        ] {
            assert_eq!(tournament_link(other), None, "{other}");
        }
    }

    #[test]
    fn a_profile_link_names_its_player() {
        for (text, name) in [
            ("https://lichess.org/@/DrNykterstein", Some("DrNykterstein")),
            ("lichess.org/@/Magnus_C-1/perf/blitz", Some("Magnus_C-1")),
            ("www.lichess.org/@/someone/all?page=2", Some("someone")),
            ("https://lichess.org/@/a", None),
            ("https://lichess.org/@/", None),
            ("https://lichess.org/abcdefgh", None),
            ("@DrNykterstein", None),
        ] {
            assert_eq!(profile_link(text).as_deref(), name, "{text}");
        }
    }
    use shakmaty::Color;

    #[test]
    fn a_lichess_game_link_names_the_game_side_and_move() {
        let link = |id: &str, orientation, ply| {
            Some(GameLink {
                id: id.to_string(),
                orientation,
                ply,
            })
        };
        assert_eq!(
            game_link("https://lichess.org/q7ZvsdUF"),
            link("q7ZvsdUF", Color::White, None)
        );
        assert_eq!(
            game_link(" lichess.org/q7ZvsdUFab12/black#23 "),
            link("q7ZvsdUF", Color::Black, Some(23))
        );
        assert_eq!(
            game_link("http://www.lichess.org/q7ZvsdUF/white?x=1"),
            link("q7ZvsdUF", Color::White, None)
        );
        for other in [
            "q7ZvsdUF",
            "https://lichess.org/study/abcdefgh",
            "https://lichess.org/training/abcde",
            "https://lichess.org/analysis",
            "https://lichess.org/q7ZvsdUF/red",
            "https://example.org/q7ZvsdUF",
            "https://lichess.org/q7Zv-dUF",
        ] {
            assert_eq!(game_link(other), None, "{other}");
        }
    }
}
