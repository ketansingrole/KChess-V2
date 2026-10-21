//! Game review (`crates/kchess-wasm/js/review.ts`): Lichess's winning chances, move labels and
//! accuracy, plus turning synced Lichess games (SAN) into UCI lines.
//!
//! Results must be bit-for-bit what V8 computes, so `exp` is fdlibm's (which V8 uses for
//! `Math.exp`) and rounding and ±0 follow JavaScript's `Math.round`, `Math.max` and `Math.min`.

use std::ops::Range;

use serde_json::{Map, Value, json};
use shakmaty::{Bitboard, Color, Position, Rank};

use crate::replay;
use crate::rules;

/* ── JavaScript arithmetic ── */

/// Whether `exp` fuses its multiply-adds. Native builds know their CPU; WebAssembly runs on
/// either, so its host measures `Math.exp` and sets this (see `set_fused_exp`).
#[cfg(target_arch = "wasm32")]
static FUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Make `exp` fuse multiply-adds (WebAssembly only; native builds follow their CPU).
pub fn set_fused_exp(fused: bool) {
    #[cfg(target_arch = "wasm32")]
    FUSED.store(fused, std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = fused;
}

/// `a * b + c` as V8's C++ computes it: clang contracts it into one fused multiply-add on
/// arm64 builds, while x86-64 builds (no FMA in the baseline instruction set) round twice.
#[inline]
fn madd(a: f64, b: f64, c: f64) -> f64 {
    #[cfg(target_arch = "aarch64")]
    {
        a.mul_add(b, c)
    }
    #[cfg(target_arch = "wasm32")]
    {
        if FUSED.load(std::sync::atomic::Ordering::Relaxed) {
            a.mul_add(b, c)
        } else {
            a * b + c
        }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "wasm32")))]
    {
        a * b + c
    }
}

/// V8's `Math.exp` (fdlibm `__ieee754_exp`, as in V8's `base/ieee754.cc`), including the
/// multiply-adds its compiler fuses on arm64, so results match `Math.exp` to the bit
/// (`tests/core/native-rules.test.ts` checks this on every platform). The constants keep
/// fdlibm's digits so they can be checked against the source.
#[allow(
    clippy::excessive_precision,
    clippy::approx_constant,
    clippy::collapsible_if
)]
pub fn js_exp(x: f64) -> f64 {
    const HALF: [f64; 2] = [0.5, -0.5];
    const O_THRESHOLD: f64 = 7.097_827_128_933_839_730_96e2;
    const U_THRESHOLD: f64 = -7.451_332_191_019_411_084_20e2;
    const LN2_HI: [f64; 2] = [
        6.931_471_803_691_238_164_90e-1,
        -6.931_471_803_691_238_164_90e-1,
    ];
    const LN2_LO: [f64; 2] = [
        1.908_214_929_270_587_700_02e-10,
        -1.908_214_929_270_587_700_02e-10,
    ];
    const INV_LN2: f64 = 1.442_695_040_888_963_387_00;
    const P1: f64 = 1.666_666_666_666_660_190_37e-1;
    const P2: f64 = -2.777_777_777_701_559_338_42e-3;
    const P3: f64 = 6.613_756_321_437_934_361_17e-5;
    const P4: f64 = -1.653_390_220_546_525_153_90e-6;
    const P5: f64 = 4.138_136_797_057_238_460_39e-8;
    const E: f64 = std::f64::consts::E;
    const HUGE: f64 = 1.0e300;
    const TWO_M1000: f64 = 9.332_636_185_032_188_789_90e-302;
    const TWO_1023: f64 = 8.988_465_674_311_579_538_65e307;

    let bits = x.to_bits();
    let mut hx = (bits >> 32) as u32;
    let xsb = ((hx >> 31) & 1) as usize;
    hx &= 0x7fff_ffff;
    let (mut hi, mut lo, mut k) = (0.0, 0.0, 0i32);
    let mut x = x;
    if hx >= 0x4086_2E42 {
        if hx >= 0x7ff0_0000 {
            let lx = bits as u32;
            if ((hx & 0xf_ffff) | lx) != 0 {
                return x + x;
            }
            return if xsb == 0 { x } else { 0.0 };
        }
        if x > O_THRESHOLD {
            return HUGE * HUGE;
        }
        if x < U_THRESHOLD {
            return TWO_M1000 * TWO_M1000;
        }
    }
    if hx > 0x3fd6_2e42 {
        if hx < 0x3FF0_A2B2 {
            if x == 1.0 {
                return E;
            }
            hi = x - LN2_HI[xsb];
            lo = LN2_LO[xsb];
            k = 1 - xsb as i32 - xsb as i32;
        } else {
            k = madd(INV_LN2, x, HALF[xsb]) as i32;
            let t = f64::from(k);
            hi = madd(-t, LN2_HI[0], x);
            lo = t * LN2_LO[0];
        }
        x = hi - lo;
    } else if hx < 0x3e30_0000 {
        if HUGE + x > 1.0 {
            return 1.0 + x;
        }
    }
    let t = x * x;
    let twopk = if k >= -1021 {
        f64::from_bits(u64::from((0x3ff0_0000 + (k << 20)) as u32) << 32)
    } else {
        f64::from_bits(u64::from((0x3ff0_0000 + ((k + 1000) << 20)) as u32) << 32)
    };
    let c = madd(
        -t,
        madd(t, madd(t, madd(t, madd(t, P5, P4), P3), P2), P1),
        x,
    );
    if k == 0 {
        return 1.0 - ((x * c) / (c - 2.0) - x);
    }
    let y = 1.0 - ((lo - (x * c) / (2.0 - c)) - hi);
    if k >= -1021 {
        if k == 1024 {
            return y * 2.0 * TWO_1023;
        }
        y * twopk
    } else {
        y * twopk * TWO_M1000
    }
}

/// `Math.round`: halves round up, toward +∞.
pub fn js_round(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let floor = x.floor();
    let rounded = if x - floor >= 0.5 { floor + 1.0 } else { floor };
    // Math.round keeps the sign of zero for -0.5 ≤ x ≤ -0.
    if rounded == 0.0 && x.is_sign_negative() {
        -0.0
    } else {
        rounded
    }
}

/// `Math.max(a, b)` for numbers: +0 is larger than −0.
fn js_max(a: f64, b: f64) -> f64 {
    if a > b || (a == b && a.is_sign_positive()) {
        a
    } else {
        b
    }
}

/// `Math.min(a, b)` for numbers: −0 is smaller than +0.
fn js_min(a: f64, b: f64) -> f64 {
    if a < b || (a == b && a.is_sign_negative()) {
        a
    } else {
        b
    }
}

/// `Math.sign` for a finite non-zero-or-zero number.
fn js_sign(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        x
    }
}

/* ── Scores ── */

#[derive(Clone, Copy, Debug, Default)]
struct Score {
    cp: Option<f64>,
    mate: Option<f64>,
}

const MAX_CP: f64 = 1000.0;

fn clamp_cp(cp: f64) -> f64 {
    js_max(-MAX_CP, js_min(MAX_CP, cp))
}

fn raw_chances(cp: f64) -> f64 {
    2.0 / (1.0 + js_exp(-0.003_682_08 * cp)) - 1.0
}

fn win_chances(score: Score, white_to_move: bool) -> f64 {
    if score.mate == Some(0.0) {
        return if white_to_move { -1.0 } else { 1.0 };
    }
    if let Some(mate) = score.mate {
        let cp = (21.0 - js_min(10.0, mate.abs())) * 100.0;
        return raw_chances(js_sign(mate) * cp);
    }
    raw_chances(clamp_cp(score.cp.unwrap_or(0.0)))
}

fn centipawns(score: Score, white_to_move: bool) -> f64 {
    if score.mate == Some(0.0) {
        return if white_to_move { -MAX_CP } else { MAX_CP };
    }
    if let Some(mate) = score.mate {
        return js_sign(mate) * MAX_CP;
    }
    clamp_cp(score.cp.unwrap_or(0.0))
}

fn pov(white: bool, value: f64) -> f64 {
    if white { value } else { -value }
}

fn judge(before: Score, after: Score, mover_white: bool) -> Option<&'static str> {
    let mate_before = before.mate.map(|m| pov(mover_white, m));
    let mate_after = after
        .mate
        .filter(|&m| m != 0.0)
        .map(|m| pov(mover_white, m));
    let delivered = after.mate == Some(0.0);
    let cp_before = before.cp.map_or(0.0, |cp| pov(mover_white, clamp_cp(cp)));
    let cp_after = after.cp.map_or(0.0, |cp| pov(mover_white, clamp_cp(cp)));
    if !delivered {
        if mate_before.is_none() && mate_after.is_some_and(|m| m < 0.0) {
            return Some(if cp_before < -999.0 {
                "inaccuracy"
            } else if cp_before < -700.0 {
                "mistake"
            } else {
                "blunder"
            });
        }
        if mate_before.is_some_and(|m| m > 0.0) && mate_after.is_none_or(|m| m < 0.0) {
            return Some(if cp_after > 999.0 {
                "inaccuracy"
            } else if cp_after > 700.0 {
                "mistake"
            } else {
                "blunder"
            });
        }
    }
    let (Some(b), Some(a)) = (before.cp, after.cp) else {
        return None;
    };
    let drop = pov(
        mover_white,
        raw_chances(clamp_cp(b)) - raw_chances(clamp_cp(a)),
    );
    [(0.3, "blunder"), (0.2, "mistake"), (0.1, "inaccuracy")]
        .into_iter()
        .find(|(threshold, _)| drop >= *threshold)
        .map(|(_, label)| label)
}

fn move_accuracy(win_before: f64, win_after: f64) -> f64 {
    if win_after >= win_before {
        return 100.0;
    }
    let raw = 103.166_810_071_164_9 * js_exp(-0.043_544_153_867_539_51 * (win_before - win_after));
    js_max(0.0, js_min(100.0, raw - 3.166_924_740_191_411 + 1.0))
}

fn mean(values: &[f64]) -> f64 {
    values.iter().fold(0.0, |sum, v| sum + v) / values.len() as f64
}

fn standard_deviation(values: &[f64]) -> f64 {
    let average = mean(values);
    // Same order and arithmetic as collecting the squares into a Vec and taking their mean,
    // without the allocation: this runs once per accuracy window of a review.
    let squares = values
        .iter()
        .fold(0.0, |sum, v| sum + (v - average) * (v - average));
    (squares / values.len() as f64).sqrt()
}

fn harmonic_mean(values: &[f64]) -> f64 {
    let sum = values
        .iter()
        .fold(0.0, |sum, v| sum + 1.0 / js_max(*v, 0.001));
    values.len() as f64 / sum
}

/* ── A stored review ── */

/// The parts of a `StoredReview` the analysis reads. Values outside the stored shape (a score
/// that is not a number, judgments that are not strings) make the input unsupported, and the
/// caller uses the TypeScript rules, whose JavaScript coercions they depend on.
struct Review<'a> {
    fen: &'a str,
    moves: Vec<&'a str>,
    lichess: bool,
    evals: Vec<Option<Score>>,
    judgments: Option<Vec<Option<&'a Value>>>,
    accuracy: [Option<f64>; 2],
}

fn number_field(map: &Map<String, Value>, key: &str) -> Result<Option<f64>, ()> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::Number(n)) => n.as_f64().map(Some).ok_or(()),
        Some(_) => Err(()),
    }
}

impl<'a> Review<'a> {
    fn read(raw: &'a Value) -> Option<Review<'a>> {
        let map = raw.as_object()?;
        let fen = map.get("fen")?.as_str()?;
        let moves = map
            .get("moves")?
            .as_array()?
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()?;
        let evals = match map.get("evals")? {
            Value::Array(items) => items
                .iter()
                .map(|item| match item {
                    Value::Null => Ok(None),
                    Value::Object(e) => {
                        let score = Score {
                            cp: number_field(e, "cp")?,
                            mate: number_field(e, "mate")?,
                        };
                        Ok((score.cp.is_some() || score.mate.is_some()).then_some(score))
                    }
                    _ => Err(()),
                })
                .collect::<Result<Vec<_>, ()>>()
                .ok()?,
            _ => return None,
        };
        let judgments = match map.get("judgments") {
            None | Some(Value::Null) => None,
            Some(Value::Array(items)) => Some(
                items
                    .iter()
                    .map(|j| match j {
                        Value::Null => Ok(None),
                        Value::String(_) => Ok(Some(j)),
                        _ => Err(()),
                    })
                    .collect::<Result<Vec<_>, ()>>()
                    .ok()?,
            ),
            Some(_) => return None,
        };
        let accuracy = match map.get("accuracy") {
            None | Some(Value::Null) => [None, None],
            Some(Value::Object(a)) => {
                let side = |key| match a.get(key) {
                    None | Some(Value::Null) => Ok(None),
                    Some(Value::Number(n)) => n.as_f64().map(Some).ok_or(()),
                    Some(_) => Err(()),
                };
                [side("white").ok()?, side("black").ok()?]
            }
            Some(_) => return None,
        };
        Some(Review {
            fen,
            moves,
            lichess: map.get("source").and_then(Value::as_str) == Some("lichess"),
            evals,
            judgments,
            accuracy,
        })
    }
}

struct Analysis {
    moves: Vec<Value>,
    sides: [Value; 2],
    start_chances: Option<f64>,
    /// Per move: whether White played it, and its accuracy.
    colors: Vec<bool>,
    accuracies: Vec<Option<f64>>,
    /// Per move, its weight in an accuracy figure; None unless every position has a score.
    weights: Option<Vec<f64>>,
}

/// `analyseReview`, or None when the input needs the TypeScript rules (see `Review::read`).
fn analyse(review: &Review) -> Option<Analysis> {
    // Only each position's side to move and end matter here, not its FEN. The end check is
    // skipped when every position already has a score: `eval_at` then never reads it, and the
    // check would generate every legal move at every ply.
    let needs_end =
        review.evals.len() < review.moves.len() + 1 || review.evals.iter().any(Option::is_none);
    let positions = replay::replay_turns_with_end(review.fen, &review.moves, needs_end);
    let eval_at = |i: usize| -> Option<Score> {
        if let Some(Some(score)) = review.evals.get(i) {
            return Some(*score);
        }
        match positions.get(i).and_then(|p| p.1) {
            Some("checkmate") => Some(Score {
                cp: None,
                mate: Some(0.0),
            }),
            Some(_) => Some(Score {
                cp: Some(0.0),
                mate: None,
            }),
            None => None,
        }
    };
    // With no legal start, the TypeScript rules throw as soon as a score needs a side to move.
    if positions.is_empty() && eval_at(0).is_some() {
        return None;
    }
    let white_at = |i: usize| -> bool {
        match positions.get(i) {
            Some(p) => p.0,
            None => positions[0].0 == i.is_multiple_of(2),
        }
    };
    let percent = |i: usize| eval_at(i).map(|s| 50.0 + 50.0 * win_chances(s, white_at(i)));
    let count = review.moves.len().min(positions.len().saturating_sub(1));
    let mut moves = Vec::with_capacity(count);
    let mut colors = Vec::with_capacity(count);
    let mut accuracies: Vec<Option<f64>> = Vec::with_capacity(count);
    let mut judgments: Vec<Option<&str>> = Vec::with_capacity(count);
    let mut losses: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
    for i in 0..count {
        let white = white_at(i);
        let (before, after) = (eval_at(i), eval_at(i + 1));
        let mut entry = Map::new();
        entry.insert(
            "color".into(),
            (if white { "white" } else { "black" }).into(),
        );
        if let Some(after) = after {
            entry.insert("chances".into(), json!(win_chances(after, white_at(i + 1))));
        }
        let judgment: Option<&str> = match (&review.judgments, review.lichess) {
            (Some(list), true) => list.get(i).copied().flatten().and_then(Value::as_str),
            _ => match (before, after) {
                (Some(b), Some(a)) => judge(b, a, white),
                _ => None,
            },
        };
        if let Some(label) = judgment {
            entry.insert("judgment".into(), label.into());
        }
        let accuracy = match (percent(i), percent(i + 1)) {
            (Some(wb), Some(wa)) => Some(move_accuracy(
                pov(white, wb - 50.0) + 50.0,
                pov(white, wa - 50.0) + 50.0,
            )),
            _ => None,
        };
        if let Some(value) = accuracy {
            entry.insert("accuracy".into(), json!(value));
        }
        if let (Some(b), Some(a)) = (before, after) {
            let loss =
                pov(white, centipawns(b, white)) - pov(white, centipawns(a, white_at(i + 1)));
            losses[usize::from(!white)].push(js_max(0.0, loss));
        }
        colors.push(white);
        accuracies.push(accuracy);
        judgments.push(judgment);
        moves.push(Value::Object(entry));
    }
    let weights = volatility_weights(count, &percent);
    let computed = weights.as_deref().map_or([None, None], |weights| {
        weighted_accuracy(&colors, &accuracies, weights, 0..count)
    });
    // One pass counts each label for both sides; the previous shape scanned every move once per
    // label and side (six passes over the game).
    let mut label_counts = [[0usize; 3]; 2];
    for i in 0..count {
        let Some(label) = judgments[i] else { continue };
        let lane = usize::from(!colors[i]);
        match label {
            "inaccuracy" => label_counts[lane][0] += 1,
            "mistake" => label_counts[lane][1] += 1,
            "blunder" => label_counts[lane][2] += 1,
            _ => {}
        }
    }
    let side = |white: bool| -> Value {
        let index = usize::from(!white);
        let lichess = if review.lichess {
            review.accuracy[index]
        } else {
            None
        };
        let mut side = Map::new();
        if let Some(figure) = lichess.or(computed[index]) {
            side.insert("accuracy".into(), json!(js_round(figure)));
        }
        if !losses[index].is_empty() {
            side.insert("acpl".into(), json!(js_round(mean(&losses[index]))));
        }
        for (slot, label) in ["inaccuracy", "mistake", "blunder"].into_iter().enumerate() {
            side.insert(label.into(), label_counts[index][slot].into());
        }
        Value::Object(side)
    };
    Some(Analysis {
        moves,
        sides: [side(true), side(false)],
        start_chances: eval_at(0).map(|s| win_chances(s, white_at(0))),
        colors,
        accuracies,
        weights,
    })
}

/// Each move's weight in an accuracy figure: how much the winning chances swing around it.
/// None unless every position has a score.
fn volatility_weights(count: usize, percent: &dyn Fn(usize) -> Option<f64>) -> Option<Vec<f64>> {
    let mut wins = Vec::with_capacity(count + 1);
    for i in 0..=count {
        wins.push(percent(i)?);
    }
    if wins.len() < 2 {
        return None;
    }
    let size = 2.max(8.min(wins.len() / 10));
    let mut weights = Vec::new();
    let window_weight = |window: &[f64]| js_max(0.5, js_min(12.0, standard_deviation(window)));
    for _ in 0..size.min(wins.len()).saturating_sub(2) {
        weights.push(window_weight(&wins[..size.min(wins.len())]));
    }
    for i in 0..=(wins.len() - size) {
        weights.push(window_weight(&wins[i..i + size]));
    }
    Some(weights)
}

/// Both sides' accuracy over the moves in `range`: the mean of the weighted and the harmonic
/// mean of their move accuracies.
fn weighted_accuracy(
    colors: &[bool],
    accuracies: &[Option<f64>],
    weights: &[f64],
    range: Range<usize>,
) -> [Option<f64>; 2] {
    let mut result = [None, None];
    for (index, white) in [true, false].into_iter().enumerate() {
        let mut scored: Vec<(f64, f64)> = Vec::new();
        for i in range.clone() {
            if colors[i] != white {
                continue;
            }
            if let (Some(accuracy), Some(&weight)) = (accuracies[i], weights.get(i)) {
                scored.push((accuracy, weight));
            }
        }
        if scored.is_empty() {
            continue;
        }
        let weighted = scored.iter().fold(0.0, |sum, (a, w)| sum + a * w)
            / scored.iter().fold(0.0, |sum, (_, w)| sum + w);
        let plain: Vec<f64> = scored.iter().map(|(a, _)| *a).collect();
        result[index] = Some((weighted + harmonic_mean(&plain)) / 2.0);
    }
    result
}

/// `analyseReview`: per-move labels, accuracy and chances, and both sides' figures. None when
/// the stored review needs the TypeScript rules.
pub fn analyse_review(raw: &Value) -> Option<Value> {
    let analysis = analyse(&Review::read(raw)?)?;
    let [white, black] = analysis.sides;
    let mut out = Map::new();
    out.insert("moves".into(), Value::Array(analysis.moves));
    out.insert("white".into(), white);
    out.insert("black".into(), black);
    if let Some(chances) = analysis.start_chances {
        out.insert("startChances".into(), json!(chances));
    }
    Some(Value::Object(out))
}

/// `reviewPhases`: each side's accuracy in the opening, middlegame and endgame, worked out like
/// the game's. The division is a simpler form of Lichess's: the middlegame begins once at most 10
/// queens, rooks, bishops and knights remain or either side has fewer than 4 pieces left on its
/// back rank, and the endgame once at most 6 remain. None when the review cannot be analysed.
pub fn review_phases(raw: &Value) -> Option<Value> {
    let review = Review::read(raw)?;
    let analysis = analyse(&review)?;
    let count = analysis.colors.len();
    let mut pos = rules::standard_from_fen(review.fen)?;
    let (mut middle, mut end) = (count, count);
    for i in 0..=count {
        let board = pos.board();
        let pieces = (board.occupied() & !board.kings() & !board.pawns()).count();
        let sparse = |color: Color, rank: Rank| {
            (board.by_color(color) & Bitboard::from_rank(rank)).count() < 4
        };
        if middle == count
            && (pieces <= 10
                || sparse(Color::White, Rank::First)
                || sparse(Color::Black, Rank::Eighth))
        {
            middle = i;
        }
        if pieces <= 6 {
            end = i;
            break;
        }
        if i == count {
            break;
        }
        let m = rules::parse_uci(review.moves[i]).and_then(|u| rules::legal_move(&pos, u))?;
        pos.play_unchecked(m);
    }
    let mut sides = [Map::new(), Map::new()];
    if let Some(weights) = analysis.weights.as_deref() {
        let phases = [
            ("opening", 0..middle),
            ("middlegame", middle..end),
            ("endgame", end..count),
        ];
        for (name, range) in phases {
            let figures = weighted_accuracy(&analysis.colors, &analysis.accuracies, weights, range);
            for (side, figure) in sides.iter_mut().zip(figures) {
                if let Some(figure) = figure {
                    side.insert(name.into(), json!(js_round(figure)));
                }
            }
        }
    }
    let [white, black] = sides;
    Some(json!({ "white": white, "black": black }))
}

/// Display classifications enrich review labels without changing Lichess judgments or practice.
pub fn review_classifications(raw: &Value) -> Option<Value> {
    let review = Review::read(raw)?;
    let analysis = analyse(&review)?;
    let mut pos = rules::standard_from_fen(review.fen)?;
    let evals = raw.get("evals")?.as_array()?;
    let mut labels = Vec::with_capacity(analysis.moves.len());
    for (i, entry) in analysis.moves.iter().enumerate() {
        let text = review.moves[i];
        let m = rules::parse_uci(text).and_then(|u| rules::legal_move(&pos, u))?;
        let before = evals.get(i).filter(|v| v.is_object());
        let after = evals.get(i + 1).filter(|v| v.is_object());
        let label = if let Some(label) = entry.get("judgment").and_then(Value::as_str) {
            Some(label)
        } else if before.is_some() && after.is_some() {
            let white = pos.turn() == shakmaty::Color::White;
            let scores = review
                .evals
                .get(i)
                .copied()
                .flatten()
                .zip(review.evals.get(i + 1).copied().flatten());
            scores.and_then(|(b, a)| {
                let loss = 50.0 * pov(white, win_chances(b, white) - win_chances(a, !white));
                let best = before.and_then(|v| v.get("best")).and_then(Value::as_str);
                if loss <= 2.0 && best == Some(text) {
                    Some(if sound_sacrifice(&pos, m, before, after) {
                        "brilliant"
                    } else {
                        "best"
                    })
                } else if loss <= 2.0 {
                    Some("excellent")
                } else if loss < 5.0 {
                    Some("good")
                } else {
                    None
                }
            })
        } else {
            None
        };
        labels.push(label.map_or(Value::Null, |v| Value::String(v.into())));
        pos.play_unchecked(m);
    }
    Some(Value::Array(labels))
}

/// A deliberately conservative brilliant: a non-forced best move whose deep engine line
/// accepts a piece sacrifice, retaining at least equality without an already won position.
/// Require the reply AND our next move so ordinary defended-piece exchanges do not qualify.
fn sound_sacrifice(
    pos: &shakmaty::variant::VariantPosition,
    m: shakmaty::Move,
    before: Option<&Value>,
    after: Option<&Value>,
) -> bool {
    use shakmaty::{Color, Role};
    let (Some(before), Some(after)) = (before, after) else {
        return false;
    };
    if before.get("depth").and_then(Value::as_u64).unwrap_or(0) < 18
        || after.get("depth").and_then(Value::as_u64).unwrap_or(0) < 18
        || pos.legal_moves().len() < 2
        || matches!(m.role(), Role::Pawn | Role::King)
    {
        return false;
    }
    let white = pos.turn() == Color::White;
    let Some(b) = before.get("cp").and_then(Value::as_f64) else {
        return false;
    };
    let viable = after
        .get("cp")
        .and_then(Value::as_f64)
        .is_some_and(|v| pov(white, v) >= -50.0)
        || after
            .get("mate")
            .and_then(Value::as_f64)
            .is_some_and(|v| pov(white, v) > 0.0);
    if !(-100.0..=200.0).contains(&pov(white, b)) || !viable {
        return false;
    }
    let Some(pv) = after
        .get("pv")
        .and_then(Value::as_array)
        .filter(|v| v.len() >= 2)
    else {
        return false;
    };
    let material = |p: &shakmaty::variant::VariantPosition| -> i32 {
        p.board()
            .iter()
            .map(|(_, piece)| {
                let value = match piece.role {
                    Role::Pawn => 1,
                    Role::Knight | Role::Bishop => 3,
                    Role::Rook => 5,
                    Role::Queen => 9,
                    Role::King => 0,
                };
                if piece.color == pos.turn() {
                    value
                } else {
                    -value
                }
            })
            .sum()
    };
    let baseline = material(pos);
    let mut line = pos.clone();
    line.play_unchecked(m);
    for (j, text) in pv.iter().take(2).enumerate() {
        let Some(reply) = text
            .as_str()
            .and_then(rules::parse_uci)
            .and_then(|u| rules::legal_move(&line, u))
        else {
            return false;
        };
        if j == 0 && (reply.to() != m.to() || reply.capture().is_none()) {
            return false;
        }
        line.play_unchecked(reply);
    }
    material(&line) <= baseline - 2
}

/// `summarize`: what the game list shows for a review.
pub fn summarize(raw: &Value) -> Option<Value> {
    let analysis = analyse(&Review::read(raw)?)?;
    let [white, black] = analysis.sides;
    let field = |key: &str| raw.get(key).cloned();
    let mut out = Map::new();
    for key in ["key", "source", "complete"] {
        if let Some(value) = field(key) {
            out.insert(key.into(), value);
        }
    }
    out.insert("white".into(), white);
    out.insert("black".into(), black);
    Some(Value::Object(out))
}

/* ── Synced Lichess games ── */

/// `sanToUci`: SAN moves played from `fen`, as UCI, as far as they are legal (standard rules).
pub fn san_to_uci<S: AsRef<str>>(fen: &str, sans: &[S]) -> Vec<String> {
    let Some(mut pos) = rules::standard_from_fen(fen) else {
        return Vec::new();
    };
    let mut moves = Vec::with_capacity(sans.len());
    for san in sans {
        let Some(m) = rules::parse_san(&pos, san.as_ref()) else {
            break;
        };
        moves.push(rules::make_uci(&m));
        pos.play_unchecked(m);
    }
    moves
}

const INITIAL_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// `/\[FEN "([^"]+)"\]/.exec(pgn)?.[1]`
fn fen_tag(pgn: &str) -> Option<&str> {
    let mut from = 0;
    while let Some(at) = pgn[from..].find("[FEN \"") {
        let start = from + at + 6;
        let value_end = pgn[start..].find('"').map(|i| start + i);
        if let Some(end) = value_end
            && end > start
            && pgn[end..].starts_with("\"]")
        {
            return Some(&pgn[start..end]);
        }
        from = from + at + 1;
    }
    None
}

/// `lichessLine`: a synced game's start position (initial FEN, else its PGN's FEN tag) and its
/// SAN moves as UCI.
pub fn lichess_line(
    moves: &str,
    pgn: Option<&str>,
    initial_fen: Option<&str>,
) -> (String, Vec<String>) {
    let tagged = pgn.filter(|p| !p.is_empty()).and_then(fen_tag);
    let start = initial_fen.or(tagged).unwrap_or(INITIAL_FEN);
    let fen = rules::parse_fen(start)
        .map_or_else(|| INITIAL_FEN.to_string(), |s| rules::make_setup_fen(&s));
    let sans: Vec<&str> = moves
        .split(crate::js::is_space)
        .filter(|s| !s.is_empty())
        .collect();
    let uci = san_to_uci(&fen, &sans);
    (fen, uci)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sacrifice_review() -> Value {
        json!({
            "fen": "rnbq1rk1/ppp1bppp/4pn2/3pP3/3P4/3B1N2/PPP2PPP/RNBQ1RK1 w - - 0 1",
            "moves": ["d3h7"], "source": "local",
            "evals": [{"cp": 20, "best": "d3h7", "depth": 18},
                {"cp": 20, "depth": 18, "pv": ["g8h7", "f3g5"]}]
        })
    }

    #[test]
    fn brilliant_requires_a_deep_sound_accepted_sacrifice() {
        let raw = sacrifice_review();
        assert_eq!(review_classifications(&raw), Some(json!(["brilliant"])));
        let mut shallow = raw.clone();
        shallow["evals"][1]["depth"] = json!(10);
        assert_eq!(review_classifications(&shallow), Some(json!(["best"])));
        let mut winning = raw.clone();
        winning["evals"][0]["cp"] = json!(500);
        winning["evals"][1]["cp"] = json!(500);
        assert_eq!(review_classifications(&winning), Some(json!(["best"])));
        let mut incomplete = raw.clone();
        incomplete["evals"][1]["pv"] = json!(["g8h7"]);
        assert_eq!(review_classifications(&incomplete), Some(json!(["best"])));
        let mut illegal = raw.clone();
        illegal["evals"][1]["pv"] = json!(["a8h7", "f3g5"]);
        assert_eq!(review_classifications(&illegal), Some(json!(["best"])));
        let mut unsound = raw.clone();
        unsound["evals"][1]["cp"] = json!(-400);
        assert_eq!(review_classifications(&unsound), Some(json!(["blunder"])));
        let mut missing = raw;
        missing["evals"][1] = Value::Null;
        assert_eq!(review_classifications(&missing), Some(json!([null])));
    }

    #[test]
    fn brilliant_uses_the_movers_perspective_for_black_too() {
        let raw = json!({
            "fen": "rnbq1rk1/ppp2ppp/3b1n2/3p4/3Pp3/4PN2/PPP1BPPP/RNBQ1RK1 b - - 0 1",
            "moves": ["d6h2"], "source": "local",
            "evals": [{"cp": -20, "best": "d6h2", "depth": 18},
                {"cp": -20, "depth": 18, "pv": ["g1h2", "f6g4"]}]
        });
        assert_eq!(review_classifications(&raw), Some(json!(["brilliant"])));
    }

    #[test]
    fn classifications_preserve_lichess_errors_and_exclude_ordinary_trades() {
        let mut raw = sacrifice_review();
        raw["source"] = json!("lichess");
        raw["judgments"] = json!(["mistake"]);
        assert_eq!(review_classifications(&raw), Some(json!(["mistake"])));
        let trade = json!({
            "fen": "3qk3/8/8/8/3n4/2B5/8/4K3 w - - 0 1",
            "moves": ["c3d4"], "source": "local",
            "evals": [{"cp": 0, "best": "c3d4", "depth": 18},
                {"cp": 0, "depth": 18, "pv": ["d8d4", "e1f1"]}]
        });
        assert_eq!(review_classifications(&trade), Some(json!(["best"])));
    }

    #[test]
    fn exp_matches_known_values() {
        assert_eq!(js_exp(0.0), 1.0);
        assert_eq!(js_exp(1.0), std::f64::consts::E);
        assert!((js_exp(-3.68208) - (-3.68208f64).exp()).abs() < 1e-15);
        assert_eq!(js_exp(f64::NEG_INFINITY), 0.0);
    }

    #[test]
    fn round_matches_javascript() {
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(0.499_999_999_999_999_94), 0.0);
        assert!(js_round(-0.4).is_sign_negative());
    }

    #[test]
    fn reads_lichess_lines() {
        let (fen, moves) = lichess_line(
            "e4 e5  Nf3",
            Some("[FEN \"8/8/8/8/8/8/8/8 w - - 0 1\"]"),
            None,
        );
        assert_eq!(fen, "8/8/8/8/8/8/8/8 w - - 0 1");
        assert!(moves.is_empty());
        let (fen, moves) = lichess_line("e4 e5 Nf3", None, None);
        assert_eq!(fen, INITIAL_FEN);
        assert_eq!(moves, ["e2e4", "e7e5", "g1f3"]);
    }

    fn cp(value: f64) -> Score {
        Score {
            cp: Some(value),
            mate: None,
        }
    }

    fn mate(value: f64) -> Score {
        Score {
            cp: None,
            mate: Some(value),
        }
    }

    #[test]
    fn winning_chances_follow_lichess_curve_and_treat_checkmate_as_lost_for_side_to_move() {
        assert_eq!(win_chances(cp(0.0), true), 0.0);
        assert!((win_chances(cp(5000.0), true) - win_chances(cp(1000.0), true)).abs() < 0.005);
        assert!(win_chances(mate(3.0), false) > 0.99);
        assert_eq!(win_chances(mate(0.0), true), -1.0);
        assert_eq!(win_chances(mate(0.0), false), 1.0);
    }

    #[test]
    fn labels_by_the_drop_in_movers_winning_chances() {
        assert_eq!(judge(cp(0.0), cp(-20.0), true), None);
        assert_eq!(judge(cp(0.0), cp(-60.0), true), Some("inaccuracy"));
        assert_eq!(judge(cp(0.0), cp(-120.0), true), Some("mistake"));
        assert_eq!(judge(cp(0.0), cp(-300.0), true), Some("blunder"));
        // The same for Black, whose losses are White's gains.
        assert_eq!(judge(cp(0.0), cp(60.0), false), Some("inaccuracy"));
        assert_eq!(judge(cp(0.0), cp(-300.0), false), None);
    }

    #[test]
    fn hardly_minds_a_lost_position_getting_more_lost() {
        assert_eq!(judge(cp(-900.0), cp(-1100.0), true), None);
    }

    #[test]
    fn judges_walking_into_mate_by_how_good_the_position_still_was() {
        assert_eq!(judge(cp(50.0), mate(-3.0), true), Some("blunder"));
        assert_eq!(judge(cp(-800.0), mate(-3.0), true), Some("mistake"));
        assert_eq!(judge(cp(-1200.0), mate(-3.0), true), Some("inaccuracy"));
        assert_eq!(judge(cp(-50.0), mate(3.0), false), Some("blunder"));
    }

    #[test]
    fn judges_missing_a_forced_mate_by_what_is_left() {
        assert_eq!(judge(mate(2.0), cp(1500.0), true), Some("inaccuracy"));
        assert_eq!(judge(mate(2.0), cp(800.0), true), Some("mistake"));
        assert_eq!(judge(mate(2.0), cp(300.0), true), Some("blunder"));
        assert_eq!(judge(mate(2.0), mate(-4.0), true), Some("blunder"));
        // A slower mate is not a mistake.
        assert_eq!(judge(mate(2.0), mate(4.0), true), None);
    }

    #[test]
    fn never_judges_the_mating_move() {
        assert_eq!(judge(mate(-1.0), mate(0.0), false), None);
    }

    #[test]
    fn is_full_for_a_move_that_keeps_the_winning_chances_and_falls_with_the_loss() {
        assert_eq!(move_accuracy(50.0, 50.0), 100.0);
        assert_eq!(move_accuracy(50.0, 60.0), 100.0);
        assert!((move_accuracy(50.0, 30.0) - 41.0).abs() < 0.5);
        assert_eq!(move_accuracy(100.0, 0.0), 0.0);
    }
}
