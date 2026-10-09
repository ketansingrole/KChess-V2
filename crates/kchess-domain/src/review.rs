//! Game review (`core/src/domain/review.ts`): Lichess's winning chances, move labels and
//! accuracy, plus turning synced Lichess games (SAN) into UCI lines.
//!
//! Results must be bit-for-bit what V8 computes, so `exp` is fdlibm's (which V8 uses for
//! `Math.exp`) and rounding and ±0 follow JavaScript's `Math.round`, `Math.max` and `Math.min`.

use serde_json::{Map, Value, json};
use shakmaty::Position;

use crate::replay;
use crate::rules;

/* ── JavaScript arithmetic ── */

/// `a * b + c` as V8's C++ computes it: clang contracts it into one fused multiply-add on
/// arm64 builds, while x86-64 builds (no FMA in the baseline instruction set) round twice.
#[inline]
fn madd(a: f64, b: f64, c: f64) -> f64 {
    #[cfg(target_arch = "aarch64")]
    {
        a.mul_add(b, c)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        a * b + c
    }
}

/// V8's `Math.exp` (fdlibm `__ieee754_exp`, as in V8's `base/ieee754.cc`), including the
/// multiply-adds its compiler fuses on arm64, so results match `Math.exp` to the bit
/// (`core/tests/unit/native-rules.test.ts` checks this on every platform). The constants keep
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
    let squares: Vec<f64> = values
        .iter()
        .map(|v| (v - average) * (v - average))
        .collect();
    mean(&squares).sqrt()
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
}

/// `analyseReview`, or None when the input needs the TypeScript rules (see `Review::read`).
fn analyse(review: &Review) -> Option<Analysis> {
    // Only each position's side to move and end matter here, not its FEN.
    let positions = replay::replay_turns(review.fen, &review.moves);
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
    let computed = game_accuracy(&colors, &accuracies, count, &percent);
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
        for label in ["inaccuracy", "mistake", "blunder"] {
            let n = (0..count)
                .filter(|&i| colors[i] == white && judgments[i] == Some(label))
                .count();
            side.insert(label.into(), n.into());
        }
        Value::Object(side)
    };
    Some(Analysis {
        moves,
        sides: [side(true), side(false)],
        start_chances: eval_at(0).map(|s| win_chances(s, white_at(0))),
    })
}

fn game_accuracy(
    colors: &[bool],
    accuracies: &[Option<f64>],
    count: usize,
    percent: &dyn Fn(usize) -> Option<f64>,
) -> [Option<f64>; 2] {
    let mut wins = Vec::with_capacity(count + 1);
    for i in 0..=count {
        match percent(i) {
            Some(value) => wins.push(value),
            None => return [None, None],
        }
    }
    if wins.len() < 2 {
        return [None, None];
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
    let mut result = [None, None];
    for (index, white) in [true, false].into_iter().enumerate() {
        let mut scored: Vec<(f64, f64)> = Vec::new();
        for i in 0..count {
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
}
