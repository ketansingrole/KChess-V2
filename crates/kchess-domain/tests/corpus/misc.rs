use serde_json::Value;

use kchess_domain::misc::constants::SESSION_KINDS;
use kchess_domain::misc::value::J;
use kchess_domain::misc::{self, documents};

use super::Out;

const DOCUMENTS: &[&str] = &[
    "validateCoreArguments",
    "decodeAnalysisSession",
    "decodeLocalSession",
    "decodeComputerSession",
    "decodeArchiveIdentity",
    "decodeStoredSession",
    "encodeSession",
    "isSessionKind",
    "decodeJoinedTournaments",
    "decodeRepertoireMisses",
];

/// Each method's first argument is the sidecar of the values JSON cannot carry.
pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    if !DOCUMENTS.contains(&method) && !misc::is_validator(method) {
        return None;
    }
    let Some((sidecar, user)) = args.split_first() else {
        return Some(Err(format!(
            "{method} was called without its lossless arguments"
        )));
    };
    Some(misc::inputs(user, sidecar).and_then(|values| answer(method, values)))
}

fn answer(method: &str, values: Vec<J>) -> Out {
    let arg = |i: usize| values.get(i).unwrap_or(&J::Undef);
    let kind = || arg(0).str().unwrap_or_default().to_string();
    let decoded = |kind: &str, raw: &J| -> Out {
        Ok(documents::decode_stored(kind, raw)?.map_or(Value::Null, J::into_value))
    };
    match method {
        "validateCoreArguments" => match values.as_slice() {
            [J::Str(name), J::Arr(list)] => {
                misc::contracts::check(name, list).map(|()| Value::Null)
            }
            _ => Err("The arguments of an argument check are a method name and a list.".into()),
        },
        "decodeAnalysisSession" => decoded("analysis", arg(0)),
        "decodeLocalSession" => decoded("local", arg(0)),
        "decodeComputerSession" => decoded("computer", arg(0)),
        "decodeArchiveIdentity" => decoded("archive:board", arg(0)),
        "decodeStoredSession" => decoded(&kind(), arg(1)),
        "encodeSession" => Ok(documents::encode_session(&kind(), arg(1)).into_value()),
        "isSessionKind" => Ok(Value::Bool(
            arg(0).str().is_some_and(|k| SESSION_KINDS.contains(&k)),
        )),
        "decodeJoinedTournaments" => Ok(documents::decode_joined_tournaments(arg(0)).into_value()),
        "decodeRepertoireMisses" => Ok(documents::decode_repertoire_misses(arg(0)).into_value()),
        _ => misc::validate(method, values),
    }
}
