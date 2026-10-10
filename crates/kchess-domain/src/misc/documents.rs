//! Library documents (`core/src/domain/library.ts`): the session decoders, the study command shape
//! and the small checks a frontend's library calls pass through. Each decoder accepts and rejects
//! what the TypeScript decoder does, and returns the same normalized value.

use std::cell::RefCell;
use std::sync::LazyLock;

use super::constants::*;
use super::engine::{
    self, Action, J, Schema, array, literal, object, optional, parse, picklist_str, pipe,
    strict_object,
};
use crate::js;
use crate::replay;
use crate::tree::{self, TreeNode};

type Handler = fn(Vec<J>) -> Result<J, String>;

const STANDARD_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

fn arg(args: &[J], i: usize) -> &J {
    args.get(i).unwrap_or(&J::Undef)
}

fn object_or_none(v: &J) -> Option<&J> {
    v.is_object().then_some(v)
}

/// `Number.isInteger(v)` for a number.
fn integer_of(v: Option<&J>) -> Option<f64> {
    match v {
        Some(J::Num(n)) if n.is_finite() && n.fract() == 0.0 => Some(*n),
        _ => None,
    }
}

/// `text(v, max)`: a string no longer than `max` UTF-16 units.
fn text_of(v: Option<&J>, max: usize) -> Option<&str> {
    match v {
        Some(J::Str(s)) if js::utf16_len(s) <= max => Some(s),
        _ => None,
    }
}

/// `Number.isFinite(v)` for a number.
fn finite_of(v: Option<&J>) -> Option<f64> {
    match v {
        Some(J::Num(n)) if n.is_finite() => Some(*n),
        _ => None,
    }
}

/// `typeof n === 'number' && Number.isFinite(n) ? Math.max(0, n) : 0`
fn ms(v: Option<&J>) -> f64 {
    finite_of(v).map_or(0.0, |n| n.max(0.0))
}

/// The JavaScript `Math.min` and `Math.max` for numbers, which propagate NaN.
fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

fn variant_of(setup: Option<&J>) -> Option<&str> {
    let variant = setup?.get("variant")?.str()?;
    VARIANTS.contains(&variant).then_some(variant)
}

/// `{ variant, fen }` from a decoded setup.
fn setup_object(variant: &str, fen: &str) -> J {
    J::Obj(vec![
        ("variant".into(), J::Str(variant.into())),
        ("fen".into(), J::Str(fen.into())),
    ])
}

/// `playable(setup, moves)`: a list of legal UCI moves from the setup's position.
fn playable(variant: &str, fen: &str, moves: Option<&J>) -> Option<Vec<String>> {
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

/// `clockSide(raw)`: `{ minutes, increment }` when both are whole minutes from 0 to 180.
fn clock_side(raw: Option<&J>) -> Option<(f64, f64)> {
    let raw = object_or_none(raw?)?;
    let minutes = integer_of(raw.get("minutes"))?;
    let increment = integer_of(raw.get("increment"))?;
    ((0.0..=180.0).contains(&minutes) && (0.0..=180.0).contains(&increment))
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

/// `decodeAnalysisSession`
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

/// `decodeLocalSession`
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

/// `value.slice(0, max)` in UTF-16 units; a character straddling the limit is left out.
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

/// `decodeComputerSession`
fn decode_computer(raw: &J) -> Option<J> {
    let raw = object_or_none(raw)?;
    let version = integer_of(raw.get("version"));
    let setup_source = raw.get("setup").filter(|s| s.truthy());
    // Version 1 games began at the standard start and had no clock.
    let (variant, fen) = match (
        version,
        variant_of(setup_source),
        setup_source.and_then(|s| text_of(s.get("fen"), 100)),
    ) {
        (Some(2.0), Some(variant), Some(fen)) => (variant, fen),
        _ => ("standard", STANDARD_FEN),
    };
    if !matches!(version, Some(v) if v == 1.0 || v == 2.0) {
        return None;
    }
    if !(variant == "standard" || variant == "chess960") {
        return None;
    }
    let moves = playable(variant, fen, raw.get("moves"))?;
    let level = raw
        .get("level")
        .and_then(J::str)
        .filter(|l| ENGINE_LEVELS.contains(l))?;
    let color = raw.get("color")?;
    if !matches!(engine::js_string(color).as_str(), "white" | "black") {
        return None;
    }
    let clock = clock_side(raw.get("clock"));
    let ply = match raw.get("ply") {
        Some(J::Num(p)) => js_max(0.0, js_min(moves.len() as f64, p.floor())),
        _ => moves.len() as f64,
    };
    let flagged = match raw.get("flagged").and_then(J::str) {
        Some(f) if f == "white" || f == "black" => J::Str(f.into()),
        _ => J::Null,
    };
    Some(J::Obj(vec![
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
    ]))
}

/* ── Archive identities ── */

/// `decodeArchiveIdentity`
fn decode_archive_identity(raw: &J) -> Option<J> {
    let raw = object_or_none(raw)?;
    let id = text_of(raw.get("id"), 80).filter(|id| !id.is_empty())?;
    let started_at = finite_of(raw.get("startedAt"))?;
    Some(J::Obj(vec![
        ("id".into(), J::Str(id.into())),
        ("startedAt".into(), J::Num(started_at)),
    ]))
}

/// `SESSION_VERSION`: the on-disk version of each kind; archives have none.
fn session_version(kind: &str) -> Option<Option<f64>> {
    Some(match kind {
        "analysis" | "local" => Some(1.0),
        "computer" => Some(2.0),
        "archive:computer" | "archive:board" | "archive:clock" => None,
        _ => return None,
    })
}

/// `decodeStoredSession(kind, raw)`; an unknown kind fails as the TypeScript table lookup did.
fn decode_stored(kind: &str, raw: &J) -> Result<Option<J>, String> {
    Ok(match kind {
        "analysis" => decode_analysis(raw),
        "local" => decode_local(raw),
        "computer" => decode_computer(raw),
        "archive:computer" | "archive:board" | "archive:clock" => decode_archive_identity(raw),
        _ => return Err("SESSION_DECODERS[kind] is not a function".into()),
    })
}

/// `encodeSession(kind, session)`: `{ version, ...session }` for kinds with a version.
fn encode_session(kind: &str, session: &J) -> J {
    let Some(Some(version)) = session_version(kind) else {
        return session.clone();
    };
    let mut fields = vec![("version".to_string(), J::Num(version))];
    // `{ version, ...session }` copies the session's own entries (none for non-objects).
    let spread: Vec<(String, J)> = match session {
        J::Obj(_) | J::Arr(_) | J::Str(_) | J::Bytes { .. } => engine::own_entries(session.clone()),
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

/// `decodeJoinedTournaments`
fn decode_joined_tournaments(raw: &J) -> J {
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
                if !TOURNAMENT_SYSTEMS.contains(&system) {
                    return None;
                }
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
fn js_key_order(fields: Vec<(String, J)>) -> Vec<(String, J)> {
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

/// `decodeRepertoireMisses`: `studyId:color` → FEN → missed count.
fn decode_repertoire_misses(raw: &J) -> J {
    let J::Obj(_) = raw else {
        return J::Obj(Vec::new());
    };
    let mut result: Vec<(String, J)> = Vec::new();
    for (key, table) in entries_of(raw).into_iter().take(200) {
        if js::utf16_len(&key) > 200 || !table.truthy() || !table.is_object() {
            continue;
        }
        let mut counts: Vec<(String, J)> = Vec::new();
        for (fen, count) in entries_of(table).into_iter().take(2000) {
            let whole = integer_of(Some(count)).filter(|n| *n > 0.0);
            if let Some(n) = whole
                && js::utf16_len(&fen) <= 100
            {
                counts.push((fen, J::Num(n)));
            }
        }
        result.push((key, J::Obj(js_key_order(counts))));
    }
    J::Obj(js_key_order(result))
}

/* ── Schemas: study commands, legacy documents and the small checks ── */

const MAX_STUDY_NAME: usize = 200;

fn study_name() -> Schema {
    pipe(
        Schema::Str(None),
        vec![Action::MaxLength(MAX_STUDY_NAME, None)],
    )
}

fn local_id() -> Schema {
    pipe(
        Schema::Str(None),
        vec![Action::MinLength(1, None), Action::MaxLength(80, None)],
    )
}

fn study_pgn() -> Schema {
    pipe(
        Schema::Str(None),
        vec![Action::MaxLength(MAX_DOCUMENT, None)],
    )
}

fn lichess_id() -> Schema {
    pipe(
        Schema::Str(None),
        vec![Action::Regex {
            source: "/^[a-zA-Z0-9]{8}$/",
            test: super::patterns::lichess_id,
            msg: None,
        }],
    )
}

fn account() -> Schema {
    pipe(Schema::Str(None), vec![Action::MaxLength(40, None)])
}

fn chapter_input() -> Schema {
    pipe(
        array(
            object(vec![("name", study_name()), ("pgn", study_pgn())], None),
            None,
        ),
        vec![
            Action::MinLength(1, None),
            Action::MaxLength(MAX_CHAPTERS, None),
        ],
    )
}

/// The `op` discriminator of one study command.
fn op(name: &str) -> (&'static str, Schema) {
    ("op", literal(name))
}

/// `studyCommandSchema`: a variant over `op`, one object per study command.
fn study_command_schema() -> Schema {
    let opt = |s: Schema| optional(s);
    Schema::Variant {
        key: "op",
        options: vec![
            object(
                vec![
                    op("save"),
                    ("name", study_name()),
                    ("pgn", study_pgn()),
                    ("id", opt(local_id())),
                    ("chapterId", opt(local_id())),
                ],
                None,
            ),
            object(
                vec![
                    op("saveChapters"),
                    ("name", study_name()),
                    ("chapters", chapter_input()),
                    ("id", opt(local_id())),
                ],
                None,
            ),
            object(
                vec![op("addChapter"), ("id", local_id()), ("name", study_name())],
                None,
            ),
            object(
                vec![
                    op("renameChapter"),
                    ("id", local_id()),
                    ("chapterId", local_id()),
                    ("name", study_name()),
                ],
                None,
            ),
            object(
                vec![
                    op("duplicateChapter"),
                    ("id", local_id()),
                    ("chapterId", local_id()),
                ],
                None,
            ),
            object(
                vec![
                    op("removeChapter"),
                    ("id", local_id()),
                    ("chapterId", local_id()),
                ],
                None,
            ),
            object(
                vec![
                    op("markCloud"),
                    ("id", local_id()),
                    ("account", account()),
                    ("remoteId", lichess_id()),
                    ("baseline", study_pgn()),
                ],
                None,
            ),
            object(
                vec![
                    op("offline"),
                    ("account", account()),
                    (
                        "remote",
                        object(vec![("id", lichess_id()), ("name", study_name())], None),
                    ),
                    ("chapters", chapter_input()),
                ],
                None,
            ),
            object(vec![op("remove"), ("id", local_id())], None),
            object(vec![op("restore"), ("study", Schema::Unknown)], None),
            object(
                vec![op("rename"), ("id", local_id()), ("name", study_name())],
                None,
            ),
            object(vec![op("duplicate"), ("id", local_id())], None),
        ],
        msg: None,
    }
}

fn library_id() -> Schema {
    pipe(
        Schema::Str(None),
        vec![Action::MinLength(1, None), Action::MaxLength(80, None)],
    )
}

fn repertoire_key() -> Schema {
    pipe(
        Schema::Str(None),
        vec![Action::MinLength(1, None), Action::MaxLength(200, None)],
    )
}

fn legacy_documents_schema() -> Schema {
    let entries = LEGACY_DOCUMENT_KEYS
        .iter()
        .map(|key| {
            (
                *key,
                optional(pipe(
                    Schema::Str(None),
                    vec![Action::MaxLength(MAX_DOCUMENT, None)],
                )),
            )
        })
        .collect();
    strict_object(entries, None)
}

static STUDY_COMMAND: LazyLock<Schema> = LazyLock::new(study_command_schema);
static LIBRARY_ID: LazyLock<Schema> = LazyLock::new(library_id);
static REPERTOIRE_KEY: LazyLock<Schema> = LazyLock::new(repertoire_key);
static SESSION_KIND: LazyLock<Schema> = LazyLock::new(|| picklist_str(SESSION_KINDS, None));
static SIDE: LazyLock<Schema> = LazyLock::new(|| picklist_str(&["white", "black"], None));
static LEGACY: LazyLock<Schema> = LazyLock::new(legacy_documents_schema);

/// `assertSession(kind, value)`: the value, decoded as its kind's document.
fn assert_session(args: Vec<J>) -> Result<J, String> {
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

/// `assertArchivedGameShape`: an object; the core service checks its fields.
fn assert_archived_game_shape(args: Vec<J>) -> Result<J, String> {
    let value = arg(&args, 0);
    if !value.truthy() || !value.is_object() {
        return Err("This game cannot be saved.".into());
    }
    Ok(value.clone())
}

fn handler_of(method: &str) -> Option<Handler> {
    Some(match method {
        "decodeAnalysisSession" => |a| Ok(decode_analysis(arg(&a, 0)).unwrap_or(J::Null)),
        "decodeLocalSession" => |a| Ok(decode_local(arg(&a, 0)).unwrap_or(J::Null)),
        "decodeComputerSession" => |a| Ok(decode_computer(arg(&a, 0)).unwrap_or(J::Null)),
        "decodeArchiveIdentity" => |a| Ok(decode_archive_identity(arg(&a, 0)).unwrap_or(J::Null)),
        "decodeStoredSession" => |a| {
            let kind = arg(&a, 0).str().unwrap_or_default().to_string();
            Ok(decode_stored(&kind, arg(&a, 1))?.unwrap_or(J::Null))
        },
        "encodeSession" => |a| {
            let kind = arg(&a, 0).str().unwrap_or_default().to_string();
            Ok(encode_session(&kind, arg(&a, 1)))
        },
        "isSessionKind" => |a| {
            Ok(J::Bool(
                arg(&a, 0).str().is_some_and(|k| SESSION_KINDS.contains(&k)),
            ))
        },
        "assertSession" => assert_session,
        "decodeJoinedTournaments" => |a| Ok(decode_joined_tournaments(arg(&a, 0))),
        "decodeRepertoireMisses" => |a| Ok(decode_repertoire_misses(arg(&a, 0))),
        "assertStudyCommandShape" => |a| parse(&STUDY_COMMAND, arg_owned(a, 0)),
        "assertArchivedGameShape" => assert_archived_game_shape,
        "assertLibraryId" => |a| parse(&LIBRARY_ID, arg_owned(a, 0)),
        "assertRepertoireKey" => |a| parse(&REPERTOIRE_KEY, arg_owned(a, 0)),
        "assertSessionKind" => |a| parse(&SESSION_KIND, arg_owned(a, 0)),
        "assertSide" => |a| parse(&SIDE, arg_owned(a, 0)),
        "assertLegacyDocuments" => |a| parse(&LEGACY, arg_owned(a, 0)),
        _ => return None,
    })
}

fn arg_owned(mut args: Vec<J>, i: usize) -> J {
    args.get_mut(i)
        .map_or(J::Undef, |slot| std::mem::replace(slot, J::Undef))
}

/// The document validators and decoders by their TypeScript names.
pub fn handler(method: &str) -> Option<Handler> {
    handler_of(method)
}

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
            decode_repertoire_misses(&raw).into_value(),
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
