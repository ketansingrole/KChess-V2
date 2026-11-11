use serde_json::Value;

use super::{Out, arg};

const METHODS: &[&str] = &[
    "treeFromPgn",
    "treeToPgn",
    "replay",
    "setupReplay",
    "reviewClassifications",
    "reviewPhases",
    "reviewMoment",
    "puzzleClue",
    "analyseReview",
    "studyDocumentPgn",
    "pvSan",
    "playMove",
    "startNode",
    "sanHistory",
    "pgnFromSan",
    "pgnFromUci",
    "pgnFromMoves",
    "setupPgn",
    "drawReason",
    "setupDrawReason",
    "position",
    "dests",
    "legalMoves",
    "play",
    "playSan",
    "line",
    "chess960Fen",
    "defaultFen",
    "fenSetup",
    "fenProblem",
    "pgnGames",
    "pgnMainline",
];

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    let _ = arg(args, 0);
    METHODS
        .contains(&method)
        .then(|| kchess_domain::api::call_value(method, args))
}
