//! Game review: Lichess's winning chances, move labels and
//! accuracy, plus turning synced Lichess games (SAN) into UCI lines.
//!
//! Results must be bit-for-bit what V8 computes, so `exp` is fdlibm's (which V8 uses for
//! `Math.exp`) and rounding and ±0 follow JavaScript's `Math.round`, `Math.max` and `Math.min`.

use std::ops::Range;

use serde::Serialize;
use serde_json::Number;
use shakmaty::variant::VariantPosition;
use shakmaty::{Bitboard, Color, Move, Position, Rank};

use crate::pgn::Game;
use crate::records::review::{
    Judgment, ReviewEval, ReviewSide, ReviewSource, ReviewSummary, StoredReview, review_key,
};
use crate::variant::Variant;
use crate::{js, replay, rules};

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
/// (checked on every platform). The constants keep
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
    /// The tablebases' verdict: a win or a loss as sure as a mate, its distance unknown.
    tablebase: bool,
}

const MAX_CP: f64 = 1000.0;

fn clamp_cp(cp: f64) -> f64 {
    js::max(-MAX_CP, js::min(MAX_CP, cp))
}

fn raw_chances(cp: f64) -> f64 {
    2.0 / (1.0 + js_exp(-0.003_682_08 * cp)) - 1.0
}

fn win_chances(score: Score, white_to_move: bool) -> f64 {
    if score.mate == Some(0.0) {
        return if white_to_move { -1.0 } else { 1.0 };
    }
    if let Some(mate) = score.mate {
        let cp = (21.0 - js::min(10.0, mate.abs())) * 100.0;
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

fn judge(before: Score, after: Score, mover_white: bool) -> Option<Judgment> {
    let mate_before = before.mate.map(|m| pov(mover_white, m));
    let mate_after = after
        .mate
        .filter(|&m| m != 0.0)
        .map(|m| pov(mover_white, m));
    // A mate after the move is never 0 (that one is delivered), so its sign says who mates.
    let mated = |m: f64| m.is_sign_negative();
    let delivered = after.mate == Some(0.0);
    let cp_before = before.cp.map_or(0.0, |cp| pov(mover_white, clamp_cp(cp)));
    let cp_after = after.cp.map_or(0.0, |cp| pov(mover_white, clamp_cp(cp)));
    // A tablebase win or loss is as sure as a mate: a mate that becomes one is not lost, and one
    // that comes after it was already there.
    let sure_win = |score: Score, cp: f64| score.tablebase && cp >= MAX_CP;
    let sure_loss = |score: Score, cp: f64| score.tablebase && cp <= -MAX_CP;
    if !delivered {
        if mate_before.is_none() && mate_after.is_some_and(mated) && !sure_loss(before, cp_before) {
            return Some(if cp_before < -999.0 {
                Judgment::Inaccuracy
            } else if cp_before < -700.0 {
                Judgment::Mistake
            } else {
                Judgment::Blunder
            });
        }
        if mate_before.is_some_and(|m| m > 0.0)
            && mate_after.is_none_or(mated)
            && !sure_win(after, cp_after)
        {
            return Some(if cp_after > 999.0 {
                Judgment::Inaccuracy
            } else if cp_after > 700.0 {
                Judgment::Mistake
            } else {
                Judgment::Blunder
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
    [
        (0.3, Judgment::Blunder),
        (0.2, Judgment::Mistake),
        (0.1, Judgment::Inaccuracy),
    ]
    .into_iter()
    .find(|(threshold, _)| drop >= *threshold)
    .map(|(_, label)| label)
}

fn move_accuracy(win_before: f64, win_after: f64) -> f64 {
    if win_after >= win_before {
        return 100.0;
    }
    let raw = 103.166_810_071_164_9 * js_exp(-0.043_544_153_867_539_51 * (win_before - win_after));
    js::max(0.0, js::min(100.0, raw - 3.166_924_740_191_411 + 1.0))
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
        .fold(0.0, |sum, v| sum + 1.0 / js::max(*v, 0.001));
    values.len() as f64 / sum
}

/* ── A stored review ── */

/// One scored position of a review, as the analysis reads it.
struct Eval<'a> {
    score: Score,
    /// The engine's move there.
    best: Option<&'a str>,
    /// The line the engine expects after it.
    pv: Vec<&'a str>,
    depth: f64,
}

/// The parts of a `StoredReview` the analysis reads.
struct Review<'a> {
    fen: &'a str,
    /// The game's variant; none for one KChess does not play.
    variant: Option<Variant>,
    moves: Vec<&'a str>,
    lichess: bool,
    /// Per position; none for one without a score.
    evals: Vec<Option<Eval<'a>>>,
    judgments: Option<&'a [Option<Judgment>]>,
    accuracy: [Option<f64>; 2],
}

impl<'a> Review<'a> {
    fn of(review: &'a StoredReview) -> Review<'a> {
        let evals = review
            .evals
            .iter()
            .map(|eval| {
                let eval = eval.as_ref()?;
                (eval.cp.is_some() || eval.mate.is_some()).then(|| Eval {
                    score: Score {
                        cp: eval.cp,
                        mate: eval.mate,
                        tablebase: eval.tablebase,
                    },
                    best: eval.best.as_deref(),
                    pv: eval.pv.iter().flatten().map(String::as_str).collect(),
                    depth: eval.depth.unwrap_or(0.0),
                })
            })
            .collect();
        let accuracy = review.accuracy.as_ref().map_or([None, None], |figures| {
            [&figures.white, &figures.black].map(|figure| figure.as_ref().and_then(Number::as_f64))
        });
        Review {
            fen: &review.fen,
            variant: match review.variant.as_deref() {
                None | Some("") => Some(Variant::Standard),
                Some(key) => Variant::from_key(key),
            },
            moves: review.moves.iter().map(String::as_str).collect(),
            lichess: review.source == ReviewSource::Lichess,
            evals,
            judgments: review.judgments.as_deref(),
            accuracy,
        }
    }

    fn eval(&self, i: usize) -> Option<&Eval<'a>> {
        self.evals.get(i)?.as_ref()
    }
}

/// A move's verdict in a game's analysis.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MoveVerdict {
    #[serde(with = "crate::color::by_name")]
    pub color: Color,
    /// White's winning chances after the move, from -1 to 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chances: Option<f64>,
    /// Lichess's label in a Lichess review, else one by the drop in the mover's winning chances.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judgment: Option<Judgment>,
    /// From 0 to 100.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accuracy: Option<f64>,
}

/// A reviewed game's verdict on each move, and both sides' figures.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameAnalysis {
    pub moves: Vec<MoveVerdict>,
    pub white: ReviewSide,
    pub black: ReviewSide,
    /// White's winning chances at the start, from -1 to 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_chances: Option<f64>,
}

impl GameAnalysis {
    pub fn side(&self, color: Color) -> &ReviewSide {
        match color {
            Color::White => &self.white,
            Color::Black => &self.black,
        }
    }
}

struct Analysis {
    moves: Vec<MoveVerdict>,
    sides: [ReviewSide; 2],
    start_chances: Option<f64>,
    /// Per move: whether White played it, and its accuracy.
    colors: Vec<bool>,
    accuracies: Vec<Option<f64>>,
    /// Per move, its weight in an accuracy figure; None unless every position has a score.
    weights: Option<Vec<f64>>,
}

/// A figure rounded as JavaScript rounds it.
fn rounded(value: f64) -> Option<Number> {
    Number::from_f64(js::round(value))
}

/// A start that is not a legal position is refused once a score needs a side to move.
fn analyse_moves(review: &Review) -> Result<Analysis, String> {
    // Only each position's side to move and end matter here, not its FEN. The end check is
    // skipped when every position already has a score: `eval_at` then never reads it, and the
    // check would generate every legal move at every ply.
    let needs_end =
        review.evals.len() < review.moves.len() + 1 || review.evals.iter().any(Option::is_none);
    let positions = review.variant.map_or_else(Vec::new, |variant| {
        replay::replay_turns_in(variant, review.fen, &review.moves, needs_end)
    });
    let eval_at = |i: usize| -> Option<Score> {
        if let Some(eval) = review.eval(i) {
            return Some(eval.score);
        }
        match positions.get(i).and_then(|p| p.1) {
            Some("checkmate") => Some(Score {
                cp: None,
                mate: Some(0.0),
                tablebase: false,
            }),
            Some("won") => Some(Score {
                cp: None,
                mate: Some(if positions.get(i).is_some_and(|p| p.0) {
                    1.0
                } else {
                    -1.0
                }),
                tablebase: false,
            }),
            Some(_) => Some(Score {
                cp: Some(0.0),
                mate: None,
                tablebase: false,
            }),
            None => None,
        }
    };
    if positions.is_empty() && eval_at(0).is_some() {
        return Err("the review's start is not a legal position".into());
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
    let mut losses: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
    for i in 0..count {
        let white = white_at(i);
        let (before, after) = (eval_at(i), eval_at(i + 1));
        let judgment = match (review.judgments, review.lichess) {
            (Some(list), true) => list.get(i).copied().flatten(),
            _ => match (before, after) {
                (Some(b), Some(a)) => judge(b, a, white),
                _ => None,
            },
        };
        let accuracy = match (percent(i), percent(i + 1)) {
            (Some(wb), Some(wa)) => Some(move_accuracy(
                pov(white, wb - 50.0) + 50.0,
                pov(white, wa - 50.0) + 50.0,
            )),
            _ => None,
        };
        if let (Some(b), Some(a)) = (before, after) {
            let loss =
                pov(white, centipawns(b, white)) - pov(white, centipawns(a, white_at(i + 1)));
            losses[usize::from(!white)].push(js::max(0.0, loss));
        }
        colors.push(white);
        accuracies.push(accuracy);
        moves.push(MoveVerdict {
            color: if white { Color::White } else { Color::Black },
            chances: after.map(|after| win_chances(after, white_at(i + 1))),
            judgment,
            accuracy,
        });
    }
    let weights = volatility_weights(count, &percent);
    let computed = weights.as_deref().map_or([None, None], |weights| {
        weighted_accuracy(&colors, &accuracies, weights, 0..count)
    });
    let mut sides = [ReviewSide::default(), ReviewSide::default()];
    for verdict in &moves {
        let side = &mut sides[usize::from(verdict.color == Color::Black)];
        match verdict.judgment {
            Some(Judgment::Inaccuracy) => side.inaccuracy += 1,
            Some(Judgment::Mistake) => side.mistake += 1,
            Some(Judgment::Blunder) => side.blunder += 1,
            None => {}
        }
    }
    for (index, side) in sides.iter_mut().enumerate() {
        let lichess = if review.lichess {
            review.accuracy[index]
        } else {
            None
        };
        side.accuracy = lichess.or(computed[index]).and_then(rounded);
        if !losses[index].is_empty() {
            side.acpl = rounded(mean(&losses[index]));
        }
    }
    Ok(Analysis {
        moves,
        sides,
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
    let window_weight = |window: &[f64]| js::max(0.5, js::min(12.0, standard_deviation(window)));
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

/// Each move's verdict and both sides' figures; why not, for a review that cannot be analysed.
pub fn analyse(review: &StoredReview) -> Result<GameAnalysis, String> {
    let analysis = analyse_moves(&Review::of(review))?;
    let [white, black] = analysis.sides;
    Ok(GameAnalysis {
        moves: analysis.moves,
        white,
        black,
        start_chances: analysis.start_chances,
    })
}

/// Where the middlegame and the endgame begin, in a simpler form of Lichess's division, in a game of `count` plies from `pos`, as ply
/// indexes (`count` for a phase never reached): the middlegame once at most 10 queens, rooks,
/// bishops and knights remain or either side has fewer than 4 pieces left on its back rank, the
/// endgame once at most 6 remain. `next` gives ply `i`'s move in the position before it; None
/// when a move it needs is not legal there.
pub fn phase_starts(
    mut pos: VariantPosition,
    count: usize,
    mut next: impl FnMut(&VariantPosition, usize) -> Option<Move>,
) -> Option<(usize, usize)> {
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
        let m = next(&pos, i)?;
        pos.play_unchecked(m);
    }
    Some((middle, end))
}

/// One side's accuracy in each phase of the game it played moves in.
#[derive(Serialize, Debug, Clone, PartialEq, Default)]
pub struct PhaseAccuracy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opening: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub middlegame: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endgame: Option<f64>,
}

/// Both sides' accuracy by phase of the game.
#[derive(Serialize, Debug, Clone, PartialEq, Default)]
pub struct ReviewPhases {
    pub white: PhaseAccuracy,
    pub black: PhaseAccuracy,
}

impl ReviewPhases {
    pub fn side(&self, color: Color) -> &PhaseAccuracy {
        match color {
            Color::White => &self.white,
            Color::Black => &self.black,
        }
    }
}

/// Each side's accuracy in the opening, middlegame and endgame, worked out like
/// the game's, the phases divided by `phase_starts` (a simpler form of Lichess's division). None
/// when the review cannot be analysed.
pub fn phases(review: &StoredReview) -> Option<ReviewPhases> {
    let review = Review::of(review);
    let analysis = analyse_moves(&review).ok()?;
    let count = analysis.colors.len();
    let start = rules::setup_start(review.variant?, review.fen)?;
    let (middle, end) = phase_starts(start, count, |pos, i| {
        rules::parse_uci(review.moves[i]).and_then(|u| rules::legal_move(pos, u))
    })?;
    let mut phases = ReviewPhases::default();
    if let Some(weights) = analysis.weights.as_deref() {
        let [opening, middlegame, endgame] = [0..middle, middle..end, end..count]
            .map(|range| weighted_accuracy(&analysis.colors, &analysis.accuracies, weights, range));
        for (index, side) in [&mut phases.white, &mut phases.black]
            .into_iter()
            .enumerate()
        {
            side.opening = opening[index].map(js::round);
            side.middlegame = middlegame[index].map(js::round);
            side.endgame = endgame[index].map(js::round);
        }
    }
    Some(phases)
}

crate::union!(#[doc = "How a reviewed move is shown: the review's label, or how near it came to the engine's choice."] Classification {
    Brilliant = "brilliant",
    Best = "best",
    Excellent = "excellent",
    Good = "good",
    Inaccuracy = "inaccuracy",
    Mistake = "mistake",
    Blunder = "blunder",
});

impl From<Judgment> for Classification {
    fn from(judgment: Judgment) -> Classification {
        match judgment {
            Judgment::Inaccuracy => Classification::Inaccuracy,
            Judgment::Mistake => Classification::Mistake,
            Judgment::Blunder => Classification::Blunder,
        }
    }
}

/// Display classifications enrich review labels without changing Lichess judgments or practice.
pub fn classifications(review: &StoredReview) -> Option<Vec<Option<Classification>>> {
    let review = Review::of(review);
    let analysis = analyse_moves(&review).ok()?;
    let mut pos = rules::setup_start(review.variant?, review.fen)?;
    let mut labels = Vec::with_capacity(analysis.moves.len());
    for (i, verdict) in analysis.moves.iter().enumerate() {
        let text = review.moves[i];
        let m = rules::parse_uci(text).and_then(|u| rules::legal_move(&pos, u))?;
        let label = match verdict.judgment {
            Some(judgment) => Some(judgment.into()),
            None => review
                .eval(i)
                .zip(review.eval(i + 1))
                .and_then(|(before, after)| {
                    let white = pos.turn() == Color::White;
                    let loss = 50.0
                        * pov(
                            white,
                            win_chances(before.score, white) - win_chances(after.score, !white),
                        );
                    if loss <= 2.0 && before.best == Some(text) {
                        // Material does not measure a sacrifice in the variants (drops, explosions).
                        Some(
                            if review.variant.is_some_and(Variant::standard_rules)
                                && sound_sacrifice(&pos, m, before, after)
                            {
                                Classification::Brilliant
                            } else {
                                Classification::Best
                            },
                        )
                    } else if loss <= 2.0 {
                        Some(Classification::Excellent)
                    } else if loss < 5.0 {
                        Some(Classification::Good)
                    } else {
                        None
                    }
                }),
        };
        labels.push(label);
        pos.play_unchecked(m);
    }
    Some(labels)
}

/// A deliberately conservative brilliant: a non-forced best move whose deep engine line
/// accepts a piece sacrifice, retaining at least equality without an already won position.
/// Require the reply AND our next move so ordinary defended-piece exchanges do not qualify.
fn sound_sacrifice(pos: &VariantPosition, m: Move, before: &Eval, after: &Eval) -> bool {
    use shakmaty::Role;
    if before.depth < 18.0
        || after.depth < 18.0
        || pos.legal_moves().len() < 2
        || matches!(m.role(), Role::Pawn | Role::King)
    {
        return false;
    }
    let white = pos.turn() == Color::White;
    let Some(b) = before.score.cp else {
        return false;
    };
    let viable = after.score.cp.is_some_and(|v| pov(white, v) >= -50.0)
        || after.score.mate.is_some_and(|v| pov(white, v) > 0.0);
    if !(-100.0..=200.0).contains(&pov(white, b)) || !viable || after.pv.len() < 2 {
        return false;
    }
    let material = |p: &VariantPosition| -> i32 {
        p.board()
            .iter()
            .map(|(_, piece)| {
                let value = crate::material::value(piece.role);
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
    for (j, text) in after.pv.iter().take(2).enumerate() {
        let Some(reply) = rules::parse_uci(text).and_then(|u| rules::legal_move(&line, u)) else {
            return false;
        };
        if j == 0 && (reply.to() != m.to() || reply.capture().is_none()) {
            return false;
        }
        line.play_unchecked(reply);
    }
    material(&line) <= baseline - 2
}

/// What the game list shows for a review; why not, for one that cannot be analysed.
pub fn summarize(review: &StoredReview) -> Result<ReviewSummary, String> {
    let analysis = analyse(review)?;
    Ok(ReviewSummary {
        key: review.key.clone(),
        source: review.source,
        engine: None,
        complete: review.complete,
        white: analysis.white,
        black: analysis.black,
    })
}

/* ── Synced Lichess games ── */

/// SAN moves played from `fen`, as UCI, as far as they are legal (standard rules).
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

use crate::rules::INITIAL_FEN;

/// The value of the first non-empty `[FEN "…"]` tag in a PGN.
fn fen_tag(pgn: &str) -> Option<&str> {
    pgn.split("[FEN \"").skip(1).find_map(|after| {
        let (value, rest) = after.split_once('"')?;
        (!value.is_empty() && rest.starts_with(']')).then_some(value)
    })
}

/// A synced game's start position (initial FEN, else its PGN's FEN tag) and its
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

/// `lichess_line` for a game of `variant`, played by its rules from its own start (Horde's,
/// Racing Kings') unless a FEN is given. None for a start its rules refuse.
pub fn lichess_line_in(
    variant: Variant,
    moves: &str,
    pgn: Option<&str>,
    initial_fen: Option<&str>,
) -> Option<(String, Vec<String>)> {
    let tagged = pgn.filter(|p| !p.is_empty()).and_then(fen_tag);
    let mut pos = match initial_fen.or(tagged) {
        Some(fen) => rules::setup_start(variant, fen)?,
        None => rules::default_position(variant.rules()),
    };
    let fen = rules::make_fen(&pos);
    let mut uci = Vec::new();
    for san in moves.split(crate::js::is_space).filter(|s| !s.is_empty()) {
        let Some(m) = rules::parse_san(&pos, san) else {
            break;
        };
        uci.push(rules::make_uci(&m));
        pos.play_unchecked(m);
    }
    Some((fen, uci))
}

/* ── Analysis written in a PGN ── */

/// The score of the last `[%eval …]` in a comment, from White's side, as Lichess writes it in a
/// PGN: pawns (`0.18`, `-1.5`), or a mate in so many moves (`#-3`), with the search's depth
/// after a comma in some exports (`0.18,23`).
pub fn comment_eval(comment: &str) -> Option<ReviewEval> {
    let mut found = None;
    let mut rest = comment;
    while let Some(at) = rest.find("[%eval") {
        let after = &rest[at + 6..];
        rest = after;
        if !after.starts_with(js::is_space) {
            continue;
        }
        let Some((value, _)) = after.split_once(']') else {
            continue;
        };
        let value = value.split(',').next().unwrap_or_default().trim();
        let eval = match value.strip_prefix('#') {
            Some(mate) => mate.parse::<i64>().ok().map(|mate| ReviewEval {
                mate: Some(mate as f64),
                ..ReviewEval::default()
            }),
            // `-0.0` is a score of 0: adding 0 turns its sign away.
            None => value
                .parse::<f64>()
                .ok()
                .filter(|pawns| pawns.is_finite())
                .map(|pawns| ReviewEval {
                    cp: Some((pawns * 100.0).round() + 0.0),
                    ..ReviewEval::default()
                }),
        };
        if eval.is_some() {
            found = eval;
        }
    }
    found
}

/// Lichess's analysis of a game as its PGN carries it, a `[%eval …]` after each move (Lichess's
/// game exports write them when asked for evals), as a review of Lichess's (depth 0, as a synced
/// game's is), labelled by `analyse` from the scores, since the PGN has no labels of Lichess's.
/// None without an eval on any move, or from a start that is not a legal position.
pub fn pgn_review(game: &Game) -> Option<StoredReview> {
    let fen = rules::make_setup_fen(&rules::parse_fen(
        game.header("FEN").unwrap_or(INITIAL_FEN),
    )?);
    rules::standard_from_fen(&fen)?;
    let nodes: Vec<usize> = game.main_line().collect();
    let sans: Vec<&str> = nodes
        .iter()
        .map(|&node| game.node(node).san.as_str())
        .collect();
    let moves = san_to_uci(&fen, &sans);
    let mut evals: Vec<Option<ReviewEval>> = vec![None; moves.len() + 1];
    for (slot, &node) in evals.iter_mut().skip(1).zip(&nodes) {
        *slot = game
            .node(node)
            .comments
            .iter()
            .flatten()
            .filter_map(|comment| comment_eval(comment))
            .next_back();
    }
    if evals.iter().all(Option::is_none) {
        return None;
    }
    // Lichess scores the standard start as +0.15 when it works out accuracy.
    if fen == INITIAL_FEN {
        evals[0] = Some(ReviewEval {
            cp: Some(15.0),
            ..ReviewEval::default()
        });
    }
    Some(StoredReview {
        key: review_key(&fen, &moves, None),
        fen,
        moves,
        source: ReviewSource::Lichess,
        engine: None,
        evals,
        depth: 0,
        complete: true,
        updated_at: 0,
        game_id: None,
        judgments: None,
        accuracy: None,
        variant: None,
        rest: serde_json::Map::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::{Value, json};

    /// A review from its JSON fields; a field left out is the default.
    fn stored(mut fields: Value) -> StoredReview {
        let defaults =
            json!({ "key": "k", "source": "local", "depth": 18, "complete": true, "updatedAt": 0 });
        for (key, value) in defaults.as_object().unwrap() {
            fields
                .as_object_mut()
                .unwrap()
                .entry(key)
                .or_insert(value.clone());
        }
        serde_json::from_value(fields).unwrap()
    }

    /// The analysis serialises as the JSON the review page and the command line read.
    #[test]
    fn an_analysis_reads_as_its_json() {
        let review = stored(json!({
            "fen": INITIAL_FEN, "moves": ["e2e4", "e7e5"],
            "evals": [{ "cp": 20 }, { "cp": 30 }, null],
        }));
        let analysis = analyse(&review).unwrap();
        assert_eq!(analysis.moves[0].color, Color::White);
        assert!(analysis.moves[0].chances.is_some());
        // An unscored position after the move leaves its chances out.
        assert_eq!(analysis.moves[1].chances, None);
        let json = serde_json::to_value(&analysis).unwrap();
        assert_eq!(json["moves"][1]["color"], json!("black"));
        assert!(json["moves"][1].get("chances").is_none());
        assert_eq!(json["white"]["blunder"], json!(0));
        assert!(json["startChances"].is_f64());
    }

    #[test]
    fn reads_variant_lichess_lines() {
        // Crazyhouse: a drop, read from the variant's start with its (empty) pockets.
        let (fen, moves) = lichess_line_in(
            Variant::Crazyhouse,
            "e4 d5 exd5 Qxd5 Nc3 Qa5 P@d4",
            None,
            None,
        )
        .unwrap();
        assert!(
            fen.contains('['),
            "a Crazyhouse FEN keeps its pockets: {fen}"
        );
        assert_eq!(moves.last().map(String::as_str), Some("P@d4"));
        assert_eq!(moves.len(), 7);
        // Horde starts from its own position, where White has no king.
        let (fen, moves) = lichess_line_in(Variant::Horde, "e5 f6", None, None).unwrap();
        assert!(
            fen.starts_with("rnbqkbnr/pppppppp/8/1PP2PP1/PPPPPPPP/"),
            "{fen}"
        );
        assert_eq!(moves, ["e4e5", "f7f6"]);
        // Standard rules would stop at the drop.
        assert_eq!(
            lichess_line("e4 d5 exd5 Qxd5 Nc3 Qa5 P@d4", None, None)
                .1
                .len(),
            6
        );
    }

    fn sacrifice_review() -> Value {
        json!({
            "fen": "rnbq1rk1/ppp1bppp/4pn2/3pP3/3P4/3B1N2/PPP2PPP/RNBQ1RK1 w - - 0 1",
            "moves": ["d3h7"], "source": "local",
            "evals": [{"cp": 20, "best": "d3h7", "depth": 18},
                {"cp": 20, "depth": 18, "pv": ["g8h7", "f3g5"]}]
        })
    }

    fn classify(review: &Value) -> Option<Vec<Option<Classification>>> {
        classifications(&stored(review.clone()))
    }

    #[test]
    fn brilliant_requires_a_deep_sound_accepted_sacrifice() {
        use Classification::{Best, Blunder, Brilliant};
        let raw = sacrifice_review();
        assert_eq!(classify(&raw), Some(vec![Some(Brilliant)]));
        let mut shallow = raw.clone();
        shallow["evals"][1]["depth"] = json!(10);
        assert_eq!(classify(&shallow), Some(vec![Some(Best)]));
        let mut winning = raw.clone();
        winning["evals"][0]["cp"] = json!(500);
        winning["evals"][1]["cp"] = json!(500);
        assert_eq!(classify(&winning), Some(vec![Some(Best)]));
        let mut incomplete = raw.clone();
        incomplete["evals"][1]["pv"] = json!(["g8h7"]);
        assert_eq!(classify(&incomplete), Some(vec![Some(Best)]));
        let mut illegal = raw.clone();
        illegal["evals"][1]["pv"] = json!(["a8h7", "f3g5"]);
        assert_eq!(classify(&illegal), Some(vec![Some(Best)]));
        let mut unsound = raw.clone();
        unsound["evals"][1]["cp"] = json!(-400);
        assert_eq!(classify(&unsound), Some(vec![Some(Blunder)]));
        let mut missing = raw;
        missing["evals"][1] = Value::Null;
        assert_eq!(classify(&missing), Some(vec![None]));
    }

    /// The analysis board's reviews keep each position's depth as a float (`18.0`). Read back
    /// as JSON, that depth was no whole number and counted as 0, so no move of a review made
    /// here was ever brilliant.
    #[test]
    fn a_depth_kept_as_a_float_still_counts() {
        let mut review = stored(sacrifice_review());
        for eval in review.evals.iter_mut().flatten() {
            eval.depth = Some(18.0);
        }
        assert_eq!(
            serde_json::to_value(&review).unwrap()["evals"][0]["depth"],
            json!(18.0)
        );
        assert_eq!(
            classifications(&review),
            Some(vec![Some(Classification::Brilliant)])
        );
    }

    #[test]
    fn classifications_preserve_lichess_errors_and_exclude_ordinary_trades() {
        let mut raw = sacrifice_review();
        raw["source"] = json!("lichess");
        raw["judgments"] = json!(["mistake"]);
        assert_eq!(classify(&raw), Some(vec![Some(Classification::Mistake)]));
        let trade = json!({
            "fen": "3qk3/8/8/8/3n4/2B5/8/4K3 w - - 0 1",
            "moves": ["c3d4"], "source": "local",
            "evals": [{"cp": 0, "best": "c3d4", "depth": 18},
                {"cp": 0, "depth": 18, "pv": ["d8d4", "e1f1"]}]
        });
        assert_eq!(classify(&trade), Some(vec![Some(Classification::Best)]));
    }

    #[test]
    fn exp_matches_known_values() {
        assert_eq!(js_exp(0.0), 1.0);
        assert_eq!(js_exp(1.0), std::f64::consts::E);
        assert!((js_exp(-3.68208) - (-3.68208f64).exp()).abs() < 1e-15);
        assert_eq!(js_exp(f64::NEG_INFINITY), 0.0);
    }

    /// `Math.exp` of V8 on arm64, bit for bit, around every branch of the routine: tiny, small,
    /// the two reduction ranges, k = 1, k below -1021, k = 1024, overflow, underflow, NaN and
    /// infinities, each also one bit either side of its threshold.
    #[test]
    fn exp_matches_v8_bit_for_bit() {
        let table: &[(u64, u64)] = &[
            (0x0000000000000000, 0x3ff0000000000000),
            (0x3ddb7cdfd9d7bdbb, 0x3ff000000006df38),
            (0xbddb7cdfd9d7bdbb, 0x3feffffffff24190),
            (0x3e29c511dc3a41df, 0x3ff0000000ce288f),
            (0xbe29c511dc3a41df, 0x3feffffffe63aee2),
            (0x3e35798ee2308c3a, 0x3ff00000015798ee),
            (0x3fb999999999999a, 0x3ff1aec7b35a00d4),
            (0xbfb999999999999a, 0x3fecf46d99d52b3a),
            (0x3fd3333333333333, 0x3ff599058c8c1a96),
            (0xbfd3333333333333, 0x3fe7b4c869c37c05),
            (0x3fd5c28f5c28f5c3, 0x3ff67aaa532108be),
            (0xbfd5c28f5c28f5c3, 0x3fe6c6d28e50ebb6),
            (0x3fd6666666666666, 0x3ff6b4802c805dec),
            (0xbfd6666666666666, 0x3fe68cce09671f71),
            (0x3fe0000000000000, 0x3ffa61298e1e069c),
            (0xbfe0000000000000, 0x3fe368b2fc6f960a),
            (0x3fe3333333333333, 0x3ffd27660b11a9ef),
            (0xbfe3333333333333, 0x3fe18fdd6b9604e4),
            (0x3feccccccccccccd, 0x4003ad44655c78bc),
            (0xbfeccccccccccccd, 0x3fda053cc0086e1e),
            (0x3ff0000000000000, 0x4005bf0a8b145769),
            (0xbff0000000000000, 0x3fd78b56362cef38),
            (0x3ff0a3d70a3d70a4, 0x4006a23c87af69e8),
            (0xbff0a3d70a3d70a4, 0x3fd69f0062e862fe),
            (0x3ff0a305532617c2, 0x4006a113e5103a32),
            (0x3ff3333333333333, 0x400a8f99761065a4),
            (0xbff3333333333333, 0x3fd346c4167a12de),
            (0x3ff8000000000000, 0x4011ed3fe64fc541),
            (0xbff8000000000000, 0x3fcc8f87724b5c1d),
            (0x4000000000000000, 0x401d8e64b8d4ddae),
            (0xc000000000000000, 0x3fc152aaa3bf81cc),
            (0x400d99999999999a, 0x4044394144eeec81),
            (0xc00d99999999999a, 0x3f99511fc6871044),
            (0x4014000000000000, 0x40628d389970338f),
            (0xc014000000000000, 0x3f7b993fe00d5376),
            (0x4024000000000000, 0x40d5829dcf950560),
            (0xc024000000000000, 0x3f07cd79b5647c9a),
            (0x4049266666666666, 0x4477b655e81f0704),
            (0xc049266666666666, 0x3b65979b805a3ba7),
            (0x4059066666666666, 0x48f5508d07df81bb),
            (0xc059066666666666, 0x36e805628c2e465c),
            (0x4072c80000000000, 0x5b0719c25b8f5b4a),
            (0xc072c80000000000, 0x24d629f5b5352a54),
            (0x4085e20000000000, 0x7f12fd8e4cbfa413),
            (0xc085e20000000000, 0x00caf5fe9a485c8e),
            (0x4086240000000000, 0x7fd1bf058bc994ad),
            (0xc086240000000000, 0x000e6cf6d08897ac),
            (0x40862d999999999a, 0x7fed75ae7a50ee14),
            (0x40862e3d70a3d70a, 0x7fefe9ce5c4c52b4),
            (0xc0862d999999999a, 0x0004584ca6a179a6),
            (0xc086826666666666, 0x000000071844d2b6),
            (0xc08720cccccccccd, 0x000000000000004d),
            (0xc08748cccccccccd, 0x0000000000000001),
            (0xc08749999999999a, 0x0000000000000000),
            (0x4086300000000000, 0x7ff0000000000000),
            (0xc087500000000000, 0x0000000000000000),
            (0x40f86a0000000000, 0x7ff0000000000000),
            (0xc0f86a0000000000, 0x0000000000000000),
            (0x3fd62e4200000000, 0x3ff6a09e0c594ff3),
            (0x3fd62e4200000001, 0x3ff6a09e0c594ff3),
            (0x3fd62e41ffffffff, 0x3ff6a09e0c594ff2),
            (0xbfd62e4200000000, 0x3fe6a09ec0a5290d),
            (0xbfd62e4200000001, 0x3fe6a09ec0a5290d),
            (0x3ff0a2b200000000, 0x4006a09e0d126a0a),
            (0x3ff0a2b1ffffffff, 0x4006a09e0d126a08),
            (0x3ff0a2b200000001, 0x4006a09e0d126a0b),
            (0xbff0a2b200000000, 0x3fd6a09ebfec0ef1),
            (0xbff0a2b1ffffffff, 0x3fd6a09ebfec0ef2),
            (0xbff0a2b200000001, 0x3fd6a09ebfec0ef0),
            (0x3e30000000000000, 0x3ff0000001000000),
            (0x3e2fffffffffffff, 0x3ff0000001000000),
            (0x3e30000000000001, 0x3ff0000001000000),
            (0xbe30000000000000, 0x3feffffffe000000),
            (0xbe2fffffffffffff, 0x3feffffffe000000),
            (0x40862e4200000000, 0x7feffc045693009d),
            (0x40862e41ffffffff, 0x7feffc045692fc9e),
            (0x40862e4200000001, 0x7feffc045693049d),
            (0xc0862e4200000000, 0x0004007f850cf7ea),
            (0xc0862e41ffffffff, 0x0004007f850cf86a),
            (0xc0862e4200000001, 0x0004007f850cf76a),
            (0x7ff0000000000000, 0x7ff0000000000000),
            (0xfff0000000000000, 0x0000000000000000),
            (0x7ff8000000000005, 0x7ff8000000000005),
            (0x7ff8000000000000, 0x7ff8000000000000),
            (0x7ff8000100000000, 0x7ff8000100000000),
            (0x7ff8000000000001, 0x7ff8000000000001),
            (0x7ff8000f00000000, 0x7ff8000f00000000),
            (0x7ff8001000000000, 0x7ff8001000000000),
            (0x7ff8000080000000, 0x7ff8000080000000),
            (0x7fefffffffffffff, 0x7ff0000000000000),
            (0xffefffffffffffff, 0x0000000000000000),
            (0x7ff0000500000005, 0x7ff8000000000000),
            (0x7ff0000500000000, 0x7ff8000000000000),
            (0xfff0000500000005, 0x7ff8000000000000),
            (0xc0874910d52d3051, 0x0000000000000001),
            (0x40862e42fefa39ef, 0x7fefffffffffff2a),
            (0x3fd62e42fefa39ef, 0x3ff6a09e667f3bcc),
            (0xbfd62e42fefa39ef, 0x3fe6a09e667f3bcc),
            (0x3fd62e4280000000, 0x3ff6a09e399a8c39),
            (0xbfd62e4280000000, 0x3fe6a09e9363ebb9),
            (0x3fd62e42ffffffff, 0x3ff6a09e66dbc8d9),
            (0xbfd62e42ffffffff, 0x3fe6a09e6622aec0),
            (0x3fd62e4212345678, 0x3ff6a09e12c8fd7a),
            (0xbfd62e4212345678, 0x3fe6a09eba357b55),
            (0x3fd62e422468ace0, 0x3ff6a09e1938aafd),
            (0xbfd62e422468ace0, 0x3fe6a09eb3c5cda4),
            (0x3fd62e429abcdef0, 0x3ff6a09e430e9325),
            (0xbfd62e429abcdef0, 0x3fe6a09e89efe4ac),
            (0x3fd62e4255555555, 0x3ff6a09e2a84cd6d),
            (0xbfd62e4255555555, 0x3fe6a09ea279aacc),
            (0x3fd62e42aaaaaaaa, 0x3ff6a09e48b04b0f),
            (0xbfd62e42aaaaaaaa, 0x3fe6a09e844e2cb2),
            (0x3fd62e420badf00d, 0x3ff6a09e107a6a28),
            (0xbfd62e420badf00d, 0x3fe6a09ebc840eb8),
            (0x3fd62e42deadbeef, 0x3ff6a09e5b13e263),
            (0xbfd62e42deadbeef, 0x3fe6a09e71ea953c),
            (0x3fd62e4231415926, 0x3ff6a09e1dc363a0),
            (0xbfd62e4231415926, 0x3fe6a09eaf3b14e2),
            (0x3fd62e4271828182, 0x3ff6a09e347b0b2f),
            (0xbfd62e4271828182, 0x3fe6a09e98836cd8),
            (0x3ca0000000000000, 0x3ff0000000000000),
            (0xbca0000000000000, 0x3fefffffffffffff),
            (0x3e30000008000000, 0x3ff0000001000001),
            (0xbe30000008000000, 0x3feffffffdffffff),
            (0x3cb8000000000000, 0x3ff0000000000002),
            (0xbcb8000000000000, 0x3feffffffffffffd),
            (0x3e30000018000000, 0x3ff0000001000002),
            (0xbe30000018000000, 0x3feffffffdfffffd),
            (0x3cc4000000000000, 0x3ff0000000000002),
            (0xbcc4000000000000, 0x3feffffffffffffb),
            (0x3e30000028000000, 0x3ff0000001000003),
            (0xbe30000028000000, 0x3feffffffdfffffb),
            (0x3ff0a2b2a7b82cfc, 0x4006a09efa435745),
            (0xbff0a2b2a7b82cfc, 0x3fd6a09dd2bb2419),
            (0x3ff0a2b24e085000, 0x4006a09e7b6d2f08),
            (0xbff0a2b24e085000, 0x3fd6a09e519148a4),
            (0x3ff0a2b2a1ea7000, 0x4006a09ef20e3060),
            (0xbff0a2b2a1ea7000, 0x3fd6a09ddaf04a96),
            (0x3ff0a2b2fef21000, 0x4006a09f759e9649),
            (0xbff0a2b2fef21000, 0x3fd6a09d575fee01),
            (0x3ff0a2b2adf13000, 0x4006a09f0310335e),
            (0xbff0a2b2adf13000, 0x3fd6a09dc9ee4876),
            (0x3ff0a2b2ac51d000, 0x4006a09f00c4c57f),
            (0xbff0a2b2ac51d000, 0x3fd6a09dcc39b636),
            (0x3ff0a2b22c35f000, 0x4006a09e4b986479),
            (0xbff0a2b22c35f000, 0x3fd6a09e81661340),
            (0x3ff0a2b2e2179080, 0x4006a09f4cd07e93),
            (0xbff0a2b2e2179080, 0x3fd6a09d802e022e),
            (0x3ff0a2b2b50fe600, 0x4006a09f0d21e780),
            (0xbff0a2b2b50fe600, 0x3fd6a09dbfdc94e5),
            (0x3ff0a2b2d3d94e00, 0x4006a09f38abe6c4),
            (0xbff0a2b2d3d94e00, 0x3fd6a09d94529875),
            (0x3ff0a2b2bd4a9600, 0x4006a09f18c53630),
            (0xbff0a2b2bd4a9600, 0x3fd6a09db43946e5),
            (0x3ff0a2b256763e00, 0x4006a09e8758f516),
            (0xbff0a2b256763e00, 0x3fd6a09e45a582b2),
            (0x3ff0a2b2cb3cc600, 0x4006a09f2c7e38c6),
            (0xbff0a2b2cb3cc600, 0x3fd6a09da0804598),
            (0x3ff0a2b23134ae00, 0x4006a09e52a8ce91),
            (0xbff0a2b23134ae00, 0x3fd6a09e7a55a91a),
            (0x3ff0a2b294727680, 0x4006a09edf0203cc),
            (0xbff0a2b294727680, 0x3fd6a09dedfc764f),
            (0x3ff0a2b23bd7d400, 0x4006a09e61b3eb94),
            (0xbff0a2b23bd7d400, 0x3fd6a09e6b4a8c06),
            (0x3ff0a2b2357da480, 0x4006a09e58b8258a),
            (0xbff0a2b2357da480, 0x3fd6a09e74465218),
            (0x3ff0a2b2c99e6b00, 0x4006a09f2a343bfe),
            (0xbff0a2b2c99e6b00, 0x3fd6a09da2ca4238),
            (0x3ff0a2b2df0df000, 0x4006a09f4884c360),
            (0xbff0a2b2df0df000, 0x3fd6a09d8479bd0a),
            (0x3ff0a2b2d80f9000, 0x4006a09f3ea0ca03),
            (0xbff0a2b2d80f9000, 0x3fd6a09d8e5db5a6),
            (0x3ff0a2b21c80b000, 0x4006a09e35617c9c),
            (0xbff0a2b21c80b000, 0x3fd6a09e979cfb68),
            (0x3ff0a2b2786b5080, 0x4006a09eb75ed1e6),
            (0xbff0a2b2786b5080, 0x3fd6a09e159fa6d4),
            (0x3ff0a2b2bb38a600, 0x4006a09f15d7c43b),
            (0xbff0a2b2bb38a600, 0x3fd6a09db726b8ad),
            (0x3ff0a2b25db30e00, 0x4006a09e91953aee),
            (0xbff0a2b25db30e00, 0x3fd6a09e3b693cfc),
            (0x3ff0a2b247815600, 0x4006a09e723210e0),
            (0xbff0a2b247815600, 0x3fd6a09e5acc66c0),
            (0x3ff0a2b25e45fe00, 0x4006a09e926507ff),
            (0xbff0a2b25e45fe00, 0x3fd6a09e3a996ff0),
            (0x3ff0a2b2eb318600, 0x4006a09f59af8e5c),
            (0xbff0a2b2eb318600, 0x3fd6a09d734ef373),
            (0x3ff0a2b2deea6e00, 0x4006a09f48528c2b),
            (0xbff0a2b2deea6e00, 0x3fd6a09d84abf43c),
            (0x3ff0a2b26b553600, 0x4006a09ea4dd0ea4),
            (0xbff0a2b26b553600, 0x3fd6a09e282169a1),
            (0x3ff0a2b26dbc5e00, 0x4006a09ea8430508),
            (0xbff0a2b26dbc5e00, 0x3fd6a09e24bb7350),
            (0x3ff0a2b2d1d86600, 0x4006a09f35d68ac4),
            (0xbff0a2b2d1d86600, 0x3fd6a09d9727f441),
            (0x3ff0a2b22c37ce00, 0x4006a09e4b9b0878),
            (0xbff0a2b22c37ce00, 0x3fd6a09e81636f42),
            (0x3ff0a2b228871680, 0x4006a09e4663007a),
            (0xbff0a2b228871680, 0x3fd6a09e869b774c),
            (0x3ff0a2b2515ff500, 0x4006a09e80273f76),
            (0xbff0a2b2515ff500, 0x3fd6a09e4cd73840),
            (0x3ff0a2b26081b100, 0x4006a09e958d893f),
            (0xbff0a2b26081b100, 0x3fd6a09e3770eebc),
            (0x3ff0a2b28126bd00, 0x4006a09ec3b82ff5),
            (0xbff0a2b28126bd00, 0x3fd6a09e09464924),
            (0x3ff0a2b2f914d800, 0x4006a09f6d538aa9),
            (0xbff0a2b2f914d800, 0x3fd6a09d5faaf8dd),
            (0x3ff0a2b277b05800, 0x4006a09eb6566742),
            (0xbff0a2b277b05800, 0x3fd6a09e16a81171),
            (0x3ff0a2b2c0e5d800, 0x4006a09f1ddee562),
            (0xbff0a2b2c0e5d800, 0x3fd6a09daf1f9806),
            (0x3ff0a2b239ad5800, 0x4006a09e5ea3c2cc),
            (0xbff0a2b239ad5800, 0x3fd6a09e6e5ab4d0),
            (0xbc90000000000000, 0x3ff0000000000000),
            (0xbca8000000000000, 0x3feffffffffffffe),
            (0xbcb4000000000000, 0x3feffffffffffffe),
            (0xbcbc000000000000, 0x3feffffffffffffc),
            (0xbcc2000000000000, 0x3feffffffffffffc),
            (0xbcc6000000000000, 0x3feffffffffffffa),
        ];
        for &(x, expected) in table {
            let (x, expected) = (f64::from_bits(x), f64::from_bits(expected));
            let got = js_exp(x);
            if expected.is_nan() {
                // A NaN comes back quiet, whatever bits of its payload are set.
                assert!(got.is_nan(), "exp({x:e}) should be NaN, got {got:e}");
                assert_eq!(got.to_bits() >> 51 & 1, 1, "exp({x:e}) is a signalling NaN");
            } else {
                assert_eq!(
                    got.to_bits(),
                    expected.to_bits(),
                    "exp({x:e}) = {got:e}, V8 says {expected:e}"
                );
            }
        }
    }

    #[test]
    fn the_sign_of_zero_is_zero() {
        assert_eq!(js_sign(3.5), 1.0);
        assert_eq!(js_sign(-0.25), -1.0);
        assert_eq!(js_sign(0.0).to_bits(), 0.0f64.to_bits());
        assert_eq!(js_sign(-0.0).to_bits(), (-0.0f64).to_bits());
    }

    #[test]
    fn the_standard_deviation_is_the_spread_around_the_mean() {
        assert_eq!(
            standard_deviation(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]),
            2.0
        );
        assert_eq!(standard_deviation(&[5.0, 5.0, 5.0]), 0.0);
        // Not whole numbers, so the order of the arithmetic shows in the last bit.
        let values = [10.1, 33.3, 47.7, 52.9, 0.3, 99.9, 71.2];
        assert_eq!(standard_deviation(&values).to_bits(), 0x403f_e64f_f80e_3431);
    }

    #[test]
    fn a_fen_tag_is_found_after_empty_and_unclosed_ones() {
        assert_eq!(
            fen_tag("[Event \"x\"]\n[FEN \"8/8/8/8/8/8/8/8 w - - 0 1\"]"),
            Some("8/8/8/8/8/8/8/8 w - - 0 1")
        );
        assert_eq!(
            fen_tag("[Event \"x\"]\n[FEN \"\"]\n[FEN \"abc\" x]\n[FEN \"8/8 w\"]"),
            Some("8/8 w")
        );
        assert_eq!(fen_tag("[FEN \"\"]"), None);
        assert_eq!(fen_tag("[FEN \"abc"), None);
    }

    #[test]
    fn variant_lichess_lines_ignore_repeated_spaces() {
        let (_, moves) = lichess_line_in(Variant::Standard, "e4  e5   Nf3", None, None).unwrap();
        assert_eq!(moves, ["e2e4", "e7e5", "g1f3"]);
        // A PGN's FEN tag gives the start, an empty PGN none.
        let tag = "[FEN \"8/8/8/8/8/8/8/K6k w - - 0 1\"]";
        let (fen, moves) = lichess_line_in(Variant::Standard, "Kb2", Some(tag), None).unwrap();
        assert_eq!(fen, "8/8/8/8/8/8/8/K6k w - - 0 1");
        assert_eq!(moves, ["a1b2"]);
        let (fen, _) = lichess_line_in(Variant::Standard, "", Some(""), None).unwrap();
        assert_eq!(fen, INITIAL_FEN);
    }

    /// Black, left with no pieces in Antichess, has won, and that counts as Black's mate in the
    /// last position even though it carries no score.
    #[test]
    fn a_won_variant_ending_without_a_score_favours_the_winner() {
        let review = stored(json!({
            "fen": "8/8/8/8/8/1p6/P7/8 w - - 0 1", "variant": "antichess",
            "moves": ["a2b3"], "evals": [{ "cp": 0 }, null],
        }));
        let analysis = analyse(&review).unwrap();
        let chances = analysis.moves[0].chances.expect("the ending is scored");
        assert!(chances < -0.9, "Black has won: {chances}");
    }

    fn sacrifice_in<'a>(fen: &str, uci: &str, reply: [&'a str; 2]) -> bool {
        let pos = rules::setup_start(Variant::Standard, fen).unwrap();
        let m = rules::parse_uci(uci)
            .and_then(|u| rules::legal_move(&pos, u))
            .unwrap();
        let eval = |pv: &[&'a str]| Eval {
            score: Score {
                cp: Some(0.0),
                mate: None,
                tablebase: false,
            },
            best: None,
            pv: pv.to_vec(),
            depth: 18.0,
        };
        sound_sacrifice(&pos, m, &eval(&[]), &eval(&reply))
    }

    #[test]
    fn a_sacrifice_needs_a_choice_of_moves_and_a_reply_on_its_square() {
        // Two legal moves, the bishop's and a pawn's: Qxb2 wins the bishop.
        let two = "4k3/8/8/8/8/2P5/5q2/B6K w - - 0 1";
        assert!(sacrifice_in(two, "a1b2", ["f2b2", "h1g1"]));
        // The same bishop move is forced once the pawn is blocked: no choice, no brilliancy.
        let forced = "4k3/8/8/8/2p5/2P5/5q2/B6K w - - 0 1";
        assert!(!sacrifice_in(forced, "a1b2", ["f2b2", "h1g1"]));
        // A reply that wins a rook elsewhere does not take the sacrificed piece.
        let elsewhere = "r3k3/8/8/R7/8/3B4/4K3/8 w - - 0 1";
        assert!(!sacrifice_in(elsewhere, "d3h7", ["a8a5", "e2e3"]));
        // A bishop for a knight is an even trade, not a sacrifice.
        let trade = "4k3/5n2/8/8/8/8/8/2B3RK w - - 0 1";
        assert!(!sacrifice_in(trade, "c1g5", ["f7g5", "g1g5"]));
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
            tablebase: false,
        }
    }

    fn mate(value: f64) -> Score {
        Score {
            cp: None,
            mate: Some(value),
            tablebase: false,
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
        assert_eq!(judge(cp(0.0), cp(-60.0), true), Some(Judgment::Inaccuracy));
        assert_eq!(judge(cp(0.0), cp(-120.0), true), Some(Judgment::Mistake));
        assert_eq!(judge(cp(0.0), cp(-300.0), true), Some(Judgment::Blunder));
        // The same for Black, whose losses are White's gains.
        assert_eq!(judge(cp(0.0), cp(60.0), false), Some(Judgment::Inaccuracy));
        assert_eq!(judge(cp(0.0), cp(-300.0), false), None);
    }

    /// The bands of a mate walked into or missed include their lower edge (-999 and -700, 999
    /// and 700 centipawns) in the milder label; a side already mated has no forced mate to miss.
    #[test]
    fn mate_judgments_start_their_bands_at_the_edge() {
        use Judgment::{Blunder, Mistake};
        for (before, after, judgment) in [
            (cp(-999.0), mate(-3.0), Some(Mistake)),
            (cp(-700.0), mate(-3.0), Some(Blunder)),
            (mate(2.0), cp(999.0), Some(Mistake)),
            (mate(2.0), cp(700.0), Some(Blunder)),
            (mate(0.0), cp(0.0), None),
        ] {
            assert_eq!(judge(before, after, true), judgment, "{before:?} {after:?}");
            let flip = |s: Score| Score {
                cp: s.cp.map(|v| -v),
                mate: s.mate.map(|v| -v),
                tablebase: s.tablebase,
            };
            assert_eq!(judge(flip(before), flip(after), false), judgment);
        }
    }

    /// The tablebases' verdicts: a won endgame let go is a blunder however it is reached, and a
    /// mate the engine saw that becomes a tablebase win is no inaccuracy (nor a tablebase loss
    /// that becomes the engine's mate).
    #[test]
    fn tablebase_verdicts_are_as_sure_as_mates() {
        use Judgment::Blunder;
        let tablebase = |cp: f64| Score {
            cp: Some(cp),
            mate: None,
            tablebase: true,
        };
        for (before, after, judgment) in [
            (tablebase(1000.0), tablebase(0.0), Some(Blunder)),
            (tablebase(0.0), tablebase(-1000.0), Some(Blunder)),
            (tablebase(1000.0), tablebase(1000.0), None),
            (tablebase(0.0), tablebase(0.0), None),
            (mate(12.0), tablebase(1000.0), None),
            (mate(12.0), tablebase(0.0), Some(Blunder)),
            (tablebase(-1000.0), mate(-9.0), None),
            (cp(400.0), tablebase(0.0), Some(Blunder)),
        ] {
            assert_eq!(judge(before, after, true), judgment, "{before:?} {after:?}");
            let flip = |s: Score| Score {
                cp: s.cp.map(|v| -v),
                mate: s.mate.map(|v| -v),
                tablebase: s.tablebase,
            };
            assert_eq!(judge(flip(before), flip(after), false), judgment);
        }
    }

    /// A review missing its last position's score still sees the mate the last move gave.
    #[test]
    fn a_mate_ending_a_review_counts_without_its_score() {
        let review = stored(json!({
            "fen": INITIAL_FEN, "moves": ["f2f3", "e7e5", "g2g4", "d8h4"],
            "evals": [{ "cp": 20 }, { "cp": -30 }, { "cp": -20 }, { "cp": -400 }],
        }));
        let analysis = analyse(&review).unwrap();
        assert_eq!(analysis.moves[3].chances, Some(-1.0));
    }

    /// Each side's figures, and its accuracy in each phase, are its own.
    #[test]
    fn each_side_reads_its_own_figures() {
        let review = stored(json!({
            "fen": INITIAL_FEN, "moves": ["e2e4", "e7e5", "d1h5"],
            "evals": [{ "cp": 20 }, { "cp": 30 }, { "cp": 30 }, { "cp": -400 }],
        }));
        let analysis = analyse(&review).unwrap();
        assert_eq!(analysis.side(Color::White).blunder, 1);
        assert_eq!(analysis.side(Color::White), &analysis.white);
        assert_eq!(analysis.side(Color::Black), &analysis.black);
        let phases = phases(&review).unwrap();
        assert!(phases.side(Color::White).opening.is_some());
        assert_eq!(phases.side(Color::White), &phases.white);
        assert_eq!(phases.side(Color::Black), &phases.black);
    }

    /// A sacrifice is brilliant only from a deep enough search before the move as well as after,
    /// and a mate found for the mover keeps it sound; a mate against them, or none, does not.
    #[test]
    fn brilliant_needs_both_depths_and_reads_a_mate_for_the_mover() {
        use Classification::{Best, Brilliant};
        let mut shallow = sacrifice_review();
        shallow["evals"][0]["depth"] = json!(10);
        assert_eq!(classify(&shallow), Some(vec![Some(Best)]));
        for (mate, label) in [(5, Brilliant), (0, Best)] {
            let mut raw = sacrifice_review();
            raw["evals"][1] = json!({ "mate": mate, "depth": 18, "pv": ["g8h7", "f3g5"] });
            assert_eq!(classify(&raw), Some(vec![Some(label)]), "mate {mate}");
        }
    }

    #[test]
    fn is_full_for_a_move_that_keeps_the_winning_chances_and_falls_with_the_loss() {
        assert_eq!(move_accuracy(50.0, 50.0), 100.0);
        assert_eq!(move_accuracy(50.0, 60.0), 100.0);
        assert!((move_accuracy(50.0, 30.0) - 41.0).abs() < 0.5);
        assert_eq!(move_accuracy(100.0, 0.0), 0.0);
    }

    #[test]
    fn eval_comments_read_as_lichess_writes_them() {
        let cp = |comment: &str| comment_eval(comment).and_then(|eval| eval.cp);
        assert_eq!(cp("[%eval 0.18] [%clk 1:30:58]"), Some(18.0));
        assert_eq!(cp("[%eval 1.5,23]"), Some(150.0));
        assert_eq!(
            cp("[%eval 0.1] then [%eval -0.4]"),
            Some(-40.0),
            "the last one"
        );
        assert!(cp("[%eval -0.0]").is_some_and(|cp| cp == 0.0 && cp.is_sign_positive()));
        assert_eq!(
            comment_eval("[%eval #-3]").and_then(|eval| eval.mate),
            Some(-3.0)
        );
        assert_eq!(comment_eval("[%evaluate 3]"), None);
        assert_eq!(comment_eval("Blunder."), None);
    }

    /// The evals of a game Lichess analysed, in its PGN, are a review of Lichess's, labelled from
    /// the scores: the move that lets a mate in one in is a blunder. A game without evals has none.
    #[test]
    fn a_pgns_evals_are_a_lichess_review() {
        let game = crate::pgn::parse_pgn(
            "[Event \"Rated blitz game\"]\n\n1. e4 { [%eval 0.3] [%clk 0:03:00] } \
             1... e5 { [%eval 0.3] } 2. Qh5 { [%eval 0.0] } 2... Nc6 { [%eval 0.1] } \
             3. Bc4 { [%eval 0.2] } 3... Nf6 { [%eval #1] } 4. Qxf7# 1-0",
        )
        .remove(0);
        let review = pgn_review(&game).expect("a review");
        assert_eq!((review.source, review.depth), (ReviewSource::Lichess, 0));
        assert_eq!(review.moves.len(), 7);
        assert_eq!(
            review.evals[0].as_ref().and_then(|eval| eval.cp),
            Some(15.0)
        );
        assert_eq!(
            review.evals[1].as_ref().and_then(|eval| eval.cp),
            Some(30.0)
        );
        assert_eq!(
            review.evals[6].as_ref().and_then(|eval| eval.mate),
            Some(1.0)
        );
        assert!(review.evals[7].is_none(), "the mate has no eval");
        let analysis = analyse(&review).expect("an analysis");
        assert_eq!(analysis.moves[5].judgment, Some(Judgment::Blunder));
        assert_eq!((analysis.black.blunder, analysis.white.blunder), (1, 0));
        assert!(analysis.black.accuracy.is_some());

        let plain = crate::pgn::parse_pgn("1. e4 { [%clk 0:03:00] } 1... e5 *").remove(0);
        assert!(pgn_review(&plain).is_none());
    }
}
