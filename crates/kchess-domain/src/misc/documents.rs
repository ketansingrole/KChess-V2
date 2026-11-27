//! Library documents: the session decoders and the session checks a frontend's library calls
//! pass through; a method's arguments are checked by their types (`kchess_contracts::check`).

use std::cell::RefCell;

use super::constants::*;
use super::value::{J, own_entries};
use crate::js;
use crate::keyed::Keyed;
use crate::replay;
use crate::training::repetition::RepertoireMisses;
use crate::tree::{self, TreeNode};
use crate::variant::Variant;

/// A document check: the arguments, normalized, or the message it rejected them with.
pub type Check = fn(Vec<J>) -> Result<J, String>;

fn arg(args: &[J], i: usize) -> &J {
    args.get(i).unwrap_or(&J::Undef)
}

fn object_or_none(v: &J) -> Option<&J> {
    v.is_object().then_some(v)
}

fn integer_of(v: Option<&J>) -> Option<f64> {
    match v {
        Some(J::Num(n)) if n.is_finite() && n.fract() == 0.0 => Some(*n),
        _ => None,
    }
}

/// A string no longer than `max` UTF-16 units.
fn text_of(v: Option<&J>, max: usize) -> Option<&str> {
    match v {
        Some(J::Str(s)) if js::utf16_len(s) <= max => Some(s),
        _ => None,
    }
}

fn finite_of(v: Option<&J>) -> Option<f64> {
    match v {
        Some(J::Num(n)) if n.is_finite() => Some(*n),
        _ => None,
    }
}

/// A finite number, at least 0; anything else is 0.
fn ms(v: Option<&J>) -> f64 {
    finite_of(v).map_or(0.0, |n| n.max(0.0))
}

fn variant_of(setup: Option<&J>) -> Option<Variant> {
    Variant::from_key(setup?.get("variant")?.str()?)
}

fn setup_object(variant: Variant, fen: &str) -> J {
    J::Obj(vec![
        ("variant".into(), J::Str(variant.as_str().into())),
        ("fen".into(), J::Str(fen.into())),
    ])
}

/// A list of legal UCI moves from the setup's position.
fn playable(variant: Variant, fen: &str, moves: Option<&J>) -> Option<Vec<String>> {
    let Some(J::Arr(items)) = moves else {
        return None;
    };
    if items.len() > MAX_MOVES {
        return None;
    }
    let mut list = Vec::with_capacity(items.len());
    for item in items {
        let J::Str(uci) = item else {
            return None;
        };
        if !super::patterns::uci_move(uci) {
            return None;
        }
        list.push(uci.clone());
    }
    (replay::replay_setup_count(variant, fen, &list) == Some(list.len())).then_some(list)
}

fn moves_value(moves: &[String]) -> J {
    J::Arr(moves.iter().map(|m| J::Str(m.clone())).collect())
}

/// `{ minutes, increment }` when both are whole minutes from 0 to 180.
fn clock_side(raw: Option<&J>) -> Option<(f64, f64)> {
    let raw = object_or_none(raw?)?;
    let minutes = integer_of(raw.get("minutes"))?;
    let increment = integer_of(raw.get("increment"))?;
    crate::games::Clock { minutes, increment }
        .fits()
        .then_some((minutes, increment))
}

fn clock_value(clock: (f64, f64)) -> J {
    J::Obj(vec![
        ("minutes".into(), J::Num(clock.0)),
        ("increment".into(), J::Num(clock.1)),
    ])
}

/// `{ white, black }` times, each a non-negative finite number of milliseconds.
fn times_value(times: Option<&J>) -> J {
    J::Obj(vec![
        (
            "white".into(),
            J::Num(ms(times.and_then(|t| t.get("white")))),
        ),
        (
            "black".into(),
            J::Num(ms(times.and_then(|t| t.get("black")))),
        ),
    ])
}

/* ── Analysis sessions ── */

thread_local! {
    /// The last PGN parsed: sessions are saved after every move, and moving changes only the path.
    static PARSED: RefCell<Option<(String, Option<TreeNode>)>> = const { RefCell::new(None) };
}

fn decode_analysis(raw: &J) -> Option<J> {
    let raw = object_or_none(raw)?;
    if raw.get("version") != Some(&J::Num(1.0)) {
        return None;
    }
    let pgn = raw.get("pgn")?.str()?;
    let path = PARSED.with(|cell| {
        let mut parsed = cell.borrow_mut();
        if parsed.as_ref().is_none_or(|(cached, _)| cached != pgn) {
            *parsed = Some((pgn.to_string(), tree::tree_from_pgn(pgn)));
        }
        let root = parsed.as_ref()?.1.as_ref()?;
        // A path the document no longer contains ends where the document does.
        let mut moves: Vec<&str> = Vec::new();
        let mut node = root;
        if let Some(path) = raw.get("path").and_then(J::str)
            && !path.is_empty()
        {
            for uci in path.split(' ') {
                let Some(next) = node.children.iter().find(|child| child.uci == uci) else {
                    break;
                };
                moves.push(uci);
                node = next;
            }
        }
        Some(moves.join(" "))
    })?;
    let text_or_empty = |key: &str| {
        raw.get(key)
            .and_then(J::str)
            .unwrap_or_default()
            .to_string()
    };
    Some(J::Obj(vec![
        ("pgn".into(), J::Str(pgn.to_string())),
        ("path".into(), J::Str(path)),
        (
            "orientation".into(),
            J::Str(
                if raw.get("orientation") == Some(&J::Str("black".into())) {
                    "black"
                } else {
                    "white"
                }
                .into(),
            ),
        ),
        ("study".into(), J::Str(text_or_empty("study"))),
        ("chapter".into(), J::Str(text_or_empty("chapter"))),
    ]))
}

/* ── Local sessions ── */

fn decode_local(raw: &J) -> Option<J> {
    if raw.get("version") != Some(&J::Num(1.0)) {
        return None;
    }
    let setup = raw.get("setup").filter(|s| s.truthy())?;
    let variant = variant_of(Some(setup))?;
    let fen = text_of(setup.get("fen"), 120)?;
    let moves = playable(variant, fen, raw.get("moves"))?;
    let clock = raw.get("clock").filter(|c| c.truthy());
    let white = clock_side(clock.and_then(|c| c.get("white")));
    let black = clock_side(clock.and_then(|c| c.get("black")));
    let times = raw
        .get("times")
        .filter(|t| t.truthy())
        .map(|t| times_value(Some(t)));
    let result = match raw.get("result").filter(|r| r.truthy()) {
        Some(result) => match result.get("reason").and_then(J::str) {
            Some(reason) => {
                let mut fields = Vec::new();
                if let Some(winner) = result.get("winner").and_then(J::str)
                    && (winner == "white" || winner == "black")
                {
                    fields.push(("winner".to_string(), J::Str(winner.to_string())));
                }
                fields.push(("reason".to_string(), J::Str(utf16_prefix(reason, 80))));
                J::Obj(fields)
            }
            None => J::Null,
        },
        None => J::Null,
    };
    Some(J::Obj(vec![
        ("setup".into(), setup_object(variant, fen)),
        ("moves".into(), moves_value(&moves)),
        (
            "clock".into(),
            match (white, black) {
                (Some(w), Some(b)) => J::Obj(vec![
                    ("white".into(), clock_value(w)),
                    ("black".into(), clock_value(b)),
                ]),
                _ => J::Null,
            },
        ),
        ("times".into(), times.unwrap_or(J::Null)),
        ("result".into(), result),
    ]))
}

/// The first `max` UTF-16 units; a character straddling the limit is left out.
fn utf16_prefix(s: &str, max: usize) -> String {
    let mut units = 0;
    let mut out = String::new();
    for c in s.chars() {
        units += c.len_utf16();
        if units > max {
            break;
        }
        out.push(c);
    }
    out
}

/* ── Computer sessions ── */

fn decode_computer(raw: &J) -> Option<J> {
    let raw = object_or_none(raw)?;
    let version = integer_of(raw.get("version"));
    let setup_source = raw.get("setup").filter(|s| s.truthy());
    // Another variant's FEN (Crazyhouse's pockets) may run as long as an over-the-board game's.
    let limit = match variant_of(setup_source) {
        Some(variant) if !variant.standard_rules() => 120,
        _ => 100,
    };
    // Version 1 games began at the standard start and had no clock.
    let (variant, fen) = match (
        version,
        variant_of(setup_source),
        setup_source.and_then(|s| text_of(s.get("fen"), limit)),
    ) {
        (Some(2.0), Some(variant), Some(fen)) => (variant, fen),
        _ => (Variant::Standard, crate::rules::INITIAL_FEN),
    };
    if !matches!(version, Some(v) if v == 1.0 || v == 2.0) {
        return None;
    }
    // Every variant: Fairy-Stockfish plays the others.
    let moves = playable(variant, fen, raw.get("moves"))?;
    let level = raw
        .get("level")
        .and_then(J::str)
        .filter(|level| EngineLevel::from_key(level).is_some())?;
    let color = raw.get("color")?;
    if !matches!(js_string(color).as_str(), "white" | "black") {
        return None;
    }
    let clock = clock_side(raw.get("clock"));
    let ply = match raw.get("ply") {
        Some(J::Num(p)) => js::max(0.0, js::min(moves.len() as f64, p.floor())),
        _ => moves.len() as f64,
    };
    let flagged = match raw.get("flagged").and_then(J::str) {
        Some(f) if f == "white" || f == "black" => J::Str(f.into()),
        _ => J::Null,
    };
    let mut session = vec![
        ("moves".into(), moves_value(&moves)),
        ("ply".into(), J::Num(ply)),
        ("level".into(), J::Str(level.into())),
        ("color".into(), color.clone()),
        (
            "resigned".into(),
            J::Bool(raw.get("resigned") == Some(&J::Bool(true))),
        ),
        ("setup".into(), setup_object(variant, fen)),
        ("clock".into(), clock.map_or(J::Null, clock_value)),
        (
            "times".into(),
            clock.map_or(J::Null, |_| times_value(raw.get("times"))),
        ),
        ("flagged".into(), flagged),
    ];
    // Who the computer plays as, kept when it is one it can play as.
    if let Some(sparring) = computer_sparring(raw.get("sparring")) {
        session.push(("sparring".into(), sparring));
    }
    if let Some(band) = integer_of(raw.get("people")).filter(|band| {
        crate::games::human::BANDS
            .iter()
            .any(|one| f64::from(*one) == *band)
    }) {
        session.push(("people".into(), J::Num(band)));
    }
    Some(J::Obj(session))
}

/// The player a computer session imitates (`computer::Sparring`): a game database's id and a
/// name in its games.
fn computer_sparring(raw: Option<&J>) -> Option<J> {
    let raw = object_or_none(raw?)?;
    let database = text_of(raw.get("database"), 40).filter(|id| {
        !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })?;
    let player = text_of(raw.get("player"), 200).filter(|name| !name.trim().is_empty())?;
    Some(J::Obj(vec![
        ("database".into(), J::Str(database.into())),
        ("player".into(), J::Str(player.into())),
    ]))
}

/// The text a value takes in string coercion.
fn js_string(v: &J) -> String {
    match v {
        J::Undef => "undefined".into(),
        J::Null => "null".into(),
        J::Bool(b) => b.to_string(),
        J::Num(n) => js::number_to_string(*n),
        J::Str(s) => s.clone(),
        J::Arr(items) => items
            .iter()
            .map(|item| match item {
                J::Undef | J::Null => String::new(),
                other => js_string(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        J::Bytes { .. } | J::Obj(_) => "[object Object]".into(),
    }
}

/* ── Archive identities ── */

fn decode_archive_identity(raw: &J) -> Option<J> {
    let raw = object_or_none(raw)?;
    let id = text_of(raw.get("id"), 80).filter(|id| !id.is_empty())?;
    let started_at = finite_of(raw.get("startedAt"))?;
    Some(J::Obj(vec![
        ("id".into(), J::Str(id.into())),
        ("startedAt".into(), J::Num(started_at)),
    ]))
}

/// The on-disk version of each kind; archives have none.
fn session_version(kind: SessionKind) -> Option<f64> {
    match kind {
        SessionKind::Analysis | SessionKind::Local => Some(1.0),
        SessionKind::Computer => Some(2.0),
        SessionKind::ArchiveComputer | SessionKind::ArchiveBoard | SessionKind::ArchiveClock => {
            None
        }
    }
}

/// A stored session of `kind`, decoded; none when it is not one. An unknown kind is an error.
pub fn decode_stored(kind: &str, raw: &J) -> Result<Option<J>, String> {
    let Some(kind) = SessionKind::from_key(kind) else {
        return Err("SESSION_DECODERS[kind] is not a function".into());
    };
    Ok(match kind {
        SessionKind::Analysis => decode_analysis(raw),
        SessionKind::Local => decode_local(raw),
        SessionKind::Computer => decode_computer(raw),
        SessionKind::ArchiveComputer | SessionKind::ArchiveBoard | SessionKind::ArchiveClock => {
            decode_archive_identity(raw)
        }
    })
}

/// A session as stored: `{ version, ...session }` for kinds with a version.
pub fn encode_session(kind: &str, session: &J) -> J {
    let Some(version) = SessionKind::from_key(kind).and_then(session_version) else {
        return session.clone();
    };
    let mut fields = vec![("version".to_string(), J::Num(version))];
    // `{ version, ...session }` copies the session's own entries (none for non-objects).
    let spread: Vec<(String, J)> = match session {
        J::Obj(_) | J::Arr(_) | J::Str(_) | J::Bytes { .. } => own_entries(session.clone()),
        _ => Vec::new(),
    };
    for (key, value) in spread {
        match fields.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => fields.push((key, value)),
        }
    }
    J::Obj(fields)
}

/* ── Joined tournaments and repertoire misses ── */

/// The tournaments joined, as stored: at most 100, each one complete.
pub fn decode_joined_tournaments(raw: &J) -> J {
    let J::Arr(entries) = raw else {
        return J::Arr(Vec::new());
    };
    J::Arr(
        entries
            .iter()
            .take(100)
            .filter_map(|entry| {
                if !entry.truthy() {
                    return None;
                }
                let system = entry.get("system").and_then(J::str)?;
                TournamentSystem::from_key(system)?;
                let id = text_of(entry.get("id"), 20)?;
                let account = text_of(entry.get("account"), 40)?;
                let name = text_of(entry.get("name"), 200)?;
                let until = finite_of(entry.get("until"))?;
                Some(J::Obj(vec![
                    ("system".into(), J::Str(system.into())),
                    ("id".into(), J::Str(id.into())),
                    ("account".into(), J::Str(account.into())),
                    ("name".into(), J::Str(name.into())),
                    ("until".into(), J::Num(until)),
                ]))
            })
            .collect(),
    )
}

/// The own properties of an object (or the indices of an array), in order.
fn entries_of(v: &J) -> Vec<(String, &J)> {
    match v {
        J::Obj(fields) => fields.iter().map(|(k, v)| (k.clone(), v)).collect(),
        J::Arr(items) => items
            .iter()
            .enumerate()
            .map(|(i, item)| (i.to_string(), item))
            .collect(),
        _ => Vec::new(),
    }
}

/// `Object.keys` order for a built object: whole-number keys ascending first, then the rest in
/// insertion order.
fn js_key_order<V>(fields: Vec<(String, V)>) -> Vec<(String, V)> {
    let index = |k: &str| {
        let canonical = k == "0"
            || (!k.starts_with('0') && !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()));
        canonical
            .then(|| k.parse::<u64>().ok())
            .flatten()
            .filter(|n| *n < u64::from(u32::MAX))
    };
    let (mut numbered, named): (Vec<_>, Vec<_>) =
        fields.into_iter().partition(|(k, _)| index(k).is_some());
    numbered.sort_by_key(|(k, _)| index(k));
    numbered.into_iter().chain(named).collect()
}

/// The repertoire misses, as stored: `studyId:color` → FEN → missed count.
pub fn decode_repertoire_misses(raw: &J) -> RepertoireMisses {
    let J::Obj(_) = raw else {
        return Keyed::default();
    };
    let mut result = Vec::new();
    for (key, table) in entries_of(raw).into_iter().take(200) {
        if js::utf16_len(&key) > 200 || !table.truthy() || !table.is_object() {
            continue;
        }
        let mut counts = Vec::new();
        for (fen, count) in entries_of(table).into_iter().take(2000) {
            let whole = integer_of(Some(count)).filter(|n| *n > 0.0);
            if let Some(n) = whole
                && js::utf16_len(&fen) <= 100
            {
                counts.push((fen, n));
            }
        }
        let counts = js_key_order(counts)
            .into_iter()
            .filter_map(|(fen, n)| match super::value::number_value(n) {
                serde_json::Value::Number(n) => Some((fen, n)),
                _ => None,
            })
            .collect();
        result.push((key, Keyed(counts)));
    }
    Keyed(js_key_order(result))
}

/* ── Session checks ── */

/// `[kind, value]`: the value, decoded as its kind's document.
pub fn assert_session(args: Vec<J>) -> Result<J, String> {
    let kind = arg(&args, 0).str().unwrap_or_default().to_string();
    let value = arg(&args, 1);
    let decoded = if value.truthy() && value.is_object() {
        let encoded = encode_session(&kind, value);
        decode_stored(&kind, &encoded)?
    } else {
        None
    };
    decoded.ok_or_else(|| "This session cannot be saved.".to_string())
}

/// A document check by the name the core's contracts give it.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repertoire_misses_keep_whole_positive_counts() {
        let raw = J::Obj(vec![(
            "study:white".into(),
            J::Obj(vec![
                ("fen-a".into(), J::Num(2.0)),
                ("fen-b".into(), J::Num(0.5)),
                ("fen-c".into(), J::Undef),
            ]),
        )]);
        assert_eq!(
            serde_json::to_value(decode_repertoire_misses(&raw)).unwrap(),
            serde_json::json!({ "study:white": { "fen-a": 2 } })
        );
    }

    #[test]
    fn sessions_need_their_version() {
        let local = J::Obj(vec![("version".into(), J::Num(2.0))]);
        assert!(decode_local(&local).is_none());
        assert!(matches!(
            decode_stored("archive:board", &J::Obj(vec![])),
            Ok(None)
        ));
        let identity = J::Obj(vec![
            ("id".into(), J::Str("g1".into())),
            ("startedAt".into(), J::Num(5.0)),
        ]);
        assert_eq!(
            encode_session("archive:board", &identity).into_value(),
            identity.clone().into_value()
        );
    }

    /// A computer game in another variant is kept (Fairy-Stockfish plays it), its moves checked
    /// by that variant's rules: a Crazyhouse drop is a move, and the same drop in standard chess
    /// is not. Such a game once could not be saved at all.
    #[test]
    fn a_computer_game_in_a_variant_is_kept_by_its_rules() {
        let session = |variant: &str, fen: &str| {
            serde_json::json!({
                "setup": { "variant": variant, "fen": fen },
                "moves": ["e2e4", "d7d5", "e4d5", "d8d5", "P@e3"],
                "ply": 5,
                "level": "club",
                "color": "white",
            })
        };
        let zh = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1";
        let check = |value: serde_json::Value| {
            assert_session(vec![
                J::Str("computer".into()),
                crate::misc::from_json(&value),
            ])
            .map(J::into_value)
        };
        let kept = check(session("crazyhouse", zh)).expect("a Crazyhouse game is kept");
        assert_eq!(kept["setup"]["variant"], "crazyhouse");
        assert_eq!(kept["moves"].as_array().map(Vec::len), Some(5));
        assert_eq!(
            check(session("standard", crate::rules::INITIAL_FEN)),
            Err("This session cannot be saved.".into())
        );
    }

    /// Who the computer plays as is saved with its game: the player it imitates, or the band of
    /// people it plays like. Both were once dropped on saving, so a resumed game, or one read
    /// again after a save, played as Stockfish. One it cannot play as is left out.
    #[test]
    fn a_computer_game_keeps_its_opponent() {
        let check = |extra: serde_json::Value| {
            let mut session = serde_json::json!({
                "setup": { "variant": "standard", "fen": crate::rules::INITIAL_FEN },
                "moves": ["d2d4"],
                "level": "club",
                "color": "white",
            });
            for (key, value) in extra.as_object().expect("an object") {
                session[key] = value.clone();
            }
            assert_session(vec![
                J::Str("computer".into()),
                crate::misc::from_json(&session),
            ])
            .map(J::into_value)
            .expect("a computer game")
        };
        let sparring =
            serde_json::json!({ "database": "player-lichess-carlsen", "player": "Carlsen" });
        assert_eq!(
            check(serde_json::json!({ "sparring": sparring }))["sparring"],
            sparring
        );
        assert_eq!(check(serde_json::json!({ "people": 1600 }))["people"], 1600);
        // Read back as the game's session, it plays as them.
        let kept: crate::games::session::ComputerSession =
            serde_json::from_value(check(serde_json::json!({ "people": 1600 }))).unwrap();
        assert_eq!(kept.people, Some(1600));
        for odd in [
            serde_json::json!({ "people": 1500 }),
            serde_json::json!({ "people": "1600" }),
            serde_json::json!({ "sparring": { "database": "Not An Id", "player": "Carlsen" } }),
            serde_json::json!({ "sparring": { "database": "player-x", "player": " " } }),
        ] {
            let kept = check(odd.clone());
            assert!(
                kept.get("people").is_none() && kept.get("sparring").is_none(),
                "{odd}: {kept}"
            );
        }
        let plain = check(serde_json::json!({}));
        assert!(plain.get("people").is_none() && plain.get("sparring").is_none());
    }

    /// Another variant's FEN may run past a standard one's 100 characters, up to 120: a full
    /// Crazyhouse board with full pockets is kept, and a standard FEN that long is not read.
    #[test]
    fn a_variant_fen_may_run_longer_than_a_standard_one() {
        let decoded = |variant: &str, fen: &str| {
            let raw = serde_json::json!({
                "version": 2,
                "setup": { "variant": variant, "fen": fen },
                "moves": [],
                "level": "club",
                "color": "white",
            });
            decode_computer(&crate::misc::from_json(&raw)).map(J::into_value)
        };
        let zh = "r1b1k1n1/p1p1p1p1/1p1p1p1p/n1b1q1r1/N1B1Q1R1/1P1P1P1P/P1P1P1P1/R1B1K1N1\
                  [PPPPPPPPNNBBRRQQppppppppnnbbrrqq] w - - 0 1";
        assert!((101..=120).contains(&zh.len()), "{}", zh.len());
        let kept = decoded("crazyhouse", zh).expect("a long Crazyhouse FEN is kept");
        assert_eq!(kept["setup"]["fen"], zh);
        let long = "x".repeat(110);
        let kept = decoded("standard", &long).expect("read from the usual start");
        assert_eq!(kept["setup"]["fen"], crate::rules::INITIAL_FEN);
    }

    /// The prefix keeps whole characters: one that straddles the limit is left out.
    #[test]
    fn a_prefix_stops_at_the_limit_in_utf16_units() {
        assert_eq!(utf16_prefix("abcdef", 3), "abc");
        assert_eq!(utf16_prefix("abc", 3), "abc");
        assert_eq!(utf16_prefix("abc", 0), "");
        // The pile of poo is two units: it fits in two, not in the one left after `a`.
        assert_eq!(utf16_prefix("a\u{1F4A9}b", 3), "a\u{1F4A9}");
        assert_eq!(utf16_prefix("a\u{1F4A9}b", 2), "a");
    }

    #[test]
    fn a_time_is_a_finite_number_or_nothing() {
        assert_eq!(ms(Some(&J::Num(f64::INFINITY))), 0.0);
        assert_eq!(ms(Some(&J::Num(f64::NEG_INFINITY))), 0.0);
        assert_eq!(ms(Some(&J::Num(1500.0))), 1500.0);
        assert_eq!(finite_of(Some(&J::Num(f64::INFINITY))), None);
        assert_eq!(finite_of(Some(&J::Num(2.5))), Some(2.5));
    }

    /// A list of moves holds 1024 at most.
    #[test]
    fn a_game_holds_1024_moves_at_most() {
        let shuffle = |count: usize| {
            let moves: Vec<J> = ["g1f3", "g8f6", "f3g1", "f6g8"]
                .iter()
                .cycle()
                .take(count)
                .map(|m| J::Str((*m).into()))
                .collect();
            J::Arr(moves)
        };
        let fen = crate::rules::INITIAL_FEN;
        let kept = playable(Variant::Standard, fen, Some(&shuffle(1024)));
        assert_eq!(kept.map(|moves| moves.len()), Some(1024));
        assert_eq!(playable(Variant::Standard, fen, Some(&shuffle(1025))), None);
    }

    /// A list's entries are its indices, so a list of counts reads as one keyed by them.
    #[test]
    fn a_list_of_misses_is_read_by_its_indices() {
        let raw = J::Obj(vec![(
            "study:white".into(),
            J::Arr(vec![J::Num(2.0), J::Num(0.0), J::Num(3.0)]),
        )]);
        assert_eq!(
            serde_json::to_value(decode_repertoire_misses(&raw)).unwrap(),
            serde_json::json!({ "study:white": { "0": 2, "2": 3 } })
        );
    }

    /// A key of 200 UTF-16 units stays; one of 201, and one with no table to hold, go.
    #[test]
    fn repertoire_keys_are_bounded_and_need_a_table() {
        let table = || J::Obj(vec![("fen".into(), J::Num(1.0))]);
        let raw = J::Obj(vec![
            ("k".repeat(200), table()),
            ("k".repeat(201), table()),
            ("string".into(), J::Str("not a table".into())),
            ("null".into(), J::Null),
        ]);
        let kept = serde_json::to_value(decode_repertoire_misses(&raw)).unwrap();
        let keys: Vec<&String> = kept.as_object().unwrap().keys().collect();
        assert_eq!(keys, [&"k".repeat(200)]);
    }

    /// Whole-number keys come first, ascending; anything that is not a canonical index below
    /// 2^32 - 1 keeps its place among the rest.
    #[test]
    fn only_canonical_indices_sort_first() {
        let order = js_key_order(
            [
                "b",
                "0",
                "05",
                "+5",
                "",
                "4294967295",
                "4294967294",
                "7",
                "-1",
                "a",
            ]
            .iter()
            .map(|k| ((*k).to_string(), J::Null))
            .collect(),
        );
        let keys: Vec<&str> = order.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            [
                "0",
                "7",
                "4294967294",
                "b",
                "05",
                "+5",
                "",
                "4294967295",
                "-1",
                "a"
            ]
        );
    }

    /// A value that is not an object is no session, whatever its kind.
    #[test]
    fn a_session_must_be_an_object() {
        let cannot = Err("This session cannot be saved.".to_string());
        for value in [J::Str("x".into()), J::Num(1.0), J::Bool(true), J::Null] {
            let checked = assert_session(vec![J::Str("no such kind".into()), value]);
            assert_eq!(checked.map(J::into_value), cannot);
        }
        let checked = assert_session(vec![J::Str("no such kind".into()), J::Obj(vec![])]);
        assert_eq!(
            checked.map(J::into_value),
            Err("SESSION_DECODERS[kind] is not a function".to_string())
        );
    }

    #[test]
    fn js_order_puts_index_keys_first() {
        let order = js_key_order(vec![
            ("b".into(), J::Null),
            ("10".into(), J::Null),
            ("2".into(), J::Null),
            ("a".into(), J::Null),
        ]);
        let keys: Vec<&str> = order.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["2", "10", "b", "a"]);
    }
}
