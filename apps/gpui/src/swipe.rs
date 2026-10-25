//! Two-finger swipe between pages (`utils/pageSwipe.ts`): a horizontal swipe on the trackpad goes
//! back or forward through the pages visited, with an arrow at the edge that fills as the swipe
//! nears the distance that commits it. The trackpad says when the fingers land and lift, so the
//! release is known rather than guessed.

use gpui_kit::TouchPhase;

/// Horizontal travel that engages the swipe.
const LOCK_TRAVEL: f32 = 10.;
/// A firm vertical scroll claims the gesture.
const VERTICAL_TRAVEL: f32 = 20.;
const VERTICAL_RATIO: f32 = 2.;
/// Long travel that never turned horizontal is a diagonal scroll, not a swipe.
const POISON_TRAVEL: f32 = 56.;
/// Travel that commits the navigation.
pub const THRESHOLD: f32 = 100.;
/// Past the threshold the swipe is armed; falling back below this share of it disarms it.
const DISARM_RATIO: f32 = 0.8;
/// How far the page follows the fingers, with growing resistance.
const SHIFT_MAX: f32 = 28.;
const BLOCKED_SHIFT_MAX: f32 = 8.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Pending,
    Horizontal,
    Armed,
    /// A scroll, not a swipe: ignored until the fingers lift.
    Finished,
}

/// Back (-1) or forward (1).
pub type Direction = i8;

/// What a gesture event asks the page for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    Nothing,
    /// Show the arrow for `direction`, `progress` of the way (0 to 1), the page `shift`ed.
    Feedback {
        direction: Direction,
        progress: f32,
        ready: bool,
        blocked: bool,
        shift: f32,
    },
    /// Hide the arrow and settle the page.
    Clear,
    Go(Direction),
}

#[derive(Debug)]
pub struct Swipe {
    state: State,
    dx: f32,
    dy: f32,
}

impl Default for Swipe {
    fn default() -> Swipe {
        Swipe {
            state: State::Pending,
            dx: 0.,
            dy: 0.,
        }
    }
}

impl Swipe {
    fn reset(&mut self) {
        *self = Swipe::default();
    }

    /// One trackpad event: its phase and travel (content moving right is positive `dx`), and
    /// whether the page can go `direction`.
    pub fn event(
        &mut self,
        phase: TouchPhase,
        dx: f32,
        dy: f32,
        can_go: impl Fn(Direction) -> bool,
    ) -> Outcome {
        match phase {
            TouchPhase::Started => {
                let was = self.state;
                self.reset();
                if was == State::Pending {
                    Outcome::Nothing
                } else {
                    Outcome::Clear
                }
            }
            TouchPhase::Ended => {
                let outcome = match self.state {
                    State::Armed => Outcome::Go(self.direction()),
                    State::Horizontal => Outcome::Clear,
                    _ => Outcome::Nothing,
                };
                // Momentum after the release is not part of the swipe.
                self.state = State::Finished;
                self.dx = 0.;
                self.dy = 0.;
                outcome
            }
            TouchPhase::Cancelled => {
                let shown = matches!(self.state, State::Horizontal | State::Armed);
                self.state = State::Finished;
                if shown {
                    Outcome::Clear
                } else {
                    Outcome::Nothing
                }
            }
            TouchPhase::Moved => self.moved(dx, dy, can_go),
        }
    }

    /// Fingers moving right (content moving right) go back.
    fn direction(&self) -> Direction {
        if self.dx > 0. { -1 } else { 1 }
    }

    fn moved(&mut self, dx: f32, dy: f32, can_go: impl Fn(Direction) -> bool) -> Outcome {
        if self.state == State::Finished {
            return Outcome::Nothing;
        }
        self.dx += dx;
        self.dy += dy;
        let (x, y) = (self.dx.abs(), self.dy.abs());
        if self.state == State::Pending {
            if y >= VERTICAL_TRAVEL && y > x * VERTICAL_RATIO {
                self.state = State::Finished;
                return Outcome::Nothing;
            }
            if x >= LOCK_TRAVEL && x > y * VERTICAL_RATIO {
                self.state = State::Horizontal;
            } else if x.max(y) >= POISON_TRAVEL {
                self.state = State::Finished;
                return Outcome::Nothing;
            } else {
                return Outcome::Nothing;
            }
        }
        let direction = self.direction();
        let allowed = can_go(direction);
        if allowed && x >= THRESHOLD {
            self.state = State::Armed;
        } else if self.state == State::Armed && x < THRESHOLD * DISARM_RATIO {
            self.state = State::Horizontal;
        }
        if !allowed {
            self.state = State::Horizontal;
        }
        let ratio = x / THRESHOLD;
        let reach = if allowed {
            SHIFT_MAX
        } else {
            BLOCKED_SHIFT_MAX
        };
        Outcome::Feedback {
            direction,
            progress: ratio.min(1.),
            ready: self.state == State::Armed,
            blocked: !allowed,
            shift: -f32::from(direction) * reach * (1. - (-ratio).exp()),
        }
    }
}

/// The pages visited, to go back and forward through as a browser does.
#[derive(Debug)]
pub struct History<T: Copy + PartialEq> {
    back: Vec<T>,
    forward: Vec<T>,
}

impl<T: Copy + PartialEq> Default for History<T> {
    fn default() -> History<T> {
        History {
            back: Vec::new(),
            forward: Vec::new(),
        }
    }
}

impl<T: Copy + PartialEq> History<T> {
    /// Leaving `from` for another page: it can be gone back to; forward is forgotten.
    pub fn visit(&mut self, from: T, to: T) {
        if from == to {
            return;
        }
        self.back.push(from);
        self.forward.clear();
    }

    pub fn can_go(&self, direction: Direction) -> bool {
        if direction < 0 {
            !self.back.is_empty()
        } else {
            !self.forward.is_empty()
        }
    }

    /// The page to show going `direction` from `current`.
    pub fn go(&mut self, direction: Direction, current: T) -> Option<T> {
        let (from, to) = if direction < 0 {
            (&mut self.back, &mut self.forward)
        } else {
            (&mut self.forward, &mut self.back)
        };
        let page = from.pop()?;
        to.push(current);
        Some(page)
    }
}

#[cfg(test)]
mod tests {
    use super::{History, Outcome, Swipe, THRESHOLD};
    use gpui_kit::TouchPhase;

    fn swipe(steps: &[(f32, f32)], can_go: bool) -> Vec<Outcome> {
        let mut swipe = Swipe::default();
        let mut out = vec![swipe.event(TouchPhase::Started, 0., 0., |_| can_go)];
        for &(dx, dy) in steps {
            out.push(swipe.event(TouchPhase::Moved, dx, dy, |_| can_go));
        }
        out.push(swipe.event(TouchPhase::Ended, 0., 0., |_| can_go));
        out
    }

    #[test]
    fn a_long_swipe_right_goes_back() {
        let out = swipe(&[(20., 1.); 6], true);
        assert_eq!(*out.last().unwrap(), Outcome::Go(-1));
        let left = swipe(&[(-20., 0.); 6], true);
        assert_eq!(*left.last().unwrap(), Outcome::Go(1));
    }

    #[test]
    fn a_short_swipe_or_a_scroll_does_nothing() {
        let short = swipe(&[(20., 0.), (20., 0.)], true);
        assert_eq!(*short.last().unwrap(), Outcome::Clear);
        let scroll = swipe(&[(1., 30.), (60., 0.), (60., 0.)], true);
        assert!(scroll.iter().all(|o| *o == Outcome::Nothing));
    }

    #[test]
    fn nowhere_to_go_never_commits() {
        let out = swipe(&[(30., 0.); 6], false);
        assert_eq!(*out.last().unwrap(), Outcome::Clear);
        assert!(
            out.iter()
                .any(|o| matches!(o, Outcome::Feedback { blocked: true, .. }))
        );
    }

    #[test]
    fn coming_back_below_the_threshold_disarms() {
        let mut steps = vec![(30., 0.); 4];
        steps.push((-(120. - THRESHOLD * 0.7), 0.));
        let out = swipe(&steps, true);
        assert_eq!(*out.last().unwrap(), Outcome::Clear);
    }

    #[test]
    fn history_goes_back_and_forward() {
        let mut history = History::default();
        history.visit(1, 2);
        history.visit(2, 3);
        assert!(history.can_go(-1) && !history.can_go(1));
        assert_eq!(history.go(-1, 3), Some(2));
        assert_eq!(history.go(1, 2), Some(3));
        history.go(-1, 3);
        history.visit(2, 4);
        assert!(!history.can_go(1));
    }
}
