//! Seeded random tests, as Zed runs its `random_*` tests (`#[gpui::test(iterations = …)]`) for
//! code without GPUI: a test makes random changes over many seeds and checks what must hold after
//! each. Every run tries the same seeds, so a failure is never a fluke of one machine, and it names
//! its seed: `SEED=<n> cargo test <name>` replays that one alone. `ITERATIONS=<n>` tries more.
//!
//! A test whose code runs on GPUI uses GPUI's own (`#[gpui_kit::test(iterations = …)]` with an
//! `StdRng` argument), which also shuffles the order its tasks run in under the same `SEED`.

use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

/// A small deterministic generator (SplitMix64): the same seed gives the same choices everywhere.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ (mixed >> 31)
    }

    /// A number under `bound`; 0 when `bound` is 0.
    pub fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        (self.next_u64() % bound as u64) as usize
    }

    /// True `percent` times in a hundred.
    pub fn chance(&mut self, percent: u32) -> bool {
        self.below(100) < percent as usize
    }

    /// One of `items`, or `None` when there are none.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            return None;
        }
        items.get(self.below(items.len()))
    }
}

/// The seeds a run tries: `SEED` alone when set, else 0 up to `ITERATIONS` or the test's own count.
pub fn seeds(iterations: u64) -> Vec<u64> {
    seeds_from(
        kchess_host::env::var("SEED").as_deref(),
        kchess_host::env::var("ITERATIONS").as_deref(),
        iterations,
    )
}

fn seeds_from(seed: Option<&str>, iterations: Option<&str>, default: u64) -> Vec<u64> {
    if let Some(seed) = seed.and_then(|seed| seed.trim().parse().ok()) {
        return vec![seed];
    }
    let count = iterations
        .and_then(|count| count.trim().parse().ok())
        .unwrap_or(default);
    (0..count).collect()
}

/// Run `test` once for each seed (`seeds`); a failing run panics with its seed, to replay it.
pub fn check(iterations: u64, test: impl Fn(&mut Rng)) {
    for seed in seeds(iterations) {
        if let Err(cause) = catch_unwind(AssertUnwindSafe(|| test(&mut Rng::new(seed)))) {
            eprintln!("failed with SEED={seed}: replay it alone with `SEED={seed} cargo test …`");
            resume_unwind(cause);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The generator is SplitMix64: its first outputs for seed 0 are the published ones.
    #[test]
    fn the_same_seed_makes_the_same_choices() {
        let mut rng = Rng::new(0);
        assert_eq!(rng.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(rng.next_u64(), 0x6E78_9E6A_A1B9_65F4);
        let draws = |seed| {
            let mut rng = Rng::new(seed);
            (0..32).map(|_| rng.below(1000)).collect::<Vec<_>>()
        };
        assert_eq!(draws(7), draws(7));
        assert_ne!(draws(7), draws(8));
    }

    /// `below` stays under its bound and reaches every number under it; `pick` every item.
    #[test]
    fn choices_cover_their_range_and_no_more() {
        let mut rng = Rng::new(3);
        let mut seen = [false; 5];
        for _ in 0..200 {
            let drawn = rng.below(5);
            assert!(drawn < 5);
            seen[drawn] = true;
        }
        assert_eq!(seen, [true; 5]);
        assert_eq!(rng.below(0), 0);
        assert_eq!(rng.pick::<u8>(&[]), None);
        let items = ['a', 'b', 'c'];
        let mut picked = [false; 3];
        for _ in 0..100 {
            let item = rng.pick(&items).copied();
            let index = items.iter().position(|it| Some(*it) == item);
            picked[index.expect("an item of the list")] = true;
        }
        assert_eq!(picked, [true; 3]);
        assert!((0..100).all(|_| rng.chance(100)));
        assert!((0..100).all(|_| !rng.chance(0)));
        let some = (0..1000).filter(|_| rng.chance(30)).count();
        assert!((200..400).contains(&some), "{some} of 1000 at 30%");
    }

    /// `SEED` replays one seed; `ITERATIONS` replaces the test's own count; neither, the count.
    #[test]
    fn a_seed_replays_alone_and_iterations_set_the_count() {
        assert_eq!(seeds_from(Some("42"), Some("9"), 3), vec![42]);
        assert_eq!(seeds_from(None, Some("4"), 3), vec![0, 1, 2, 3]);
        assert_eq!(seeds_from(None, None, 3), vec![0, 1, 2]);
        assert_eq!(seeds_from(Some("x"), Some(" 2 "), 3), vec![0, 1]);
    }

    /// Every seed runs, in order, and a failure carries the failing run's panic out.
    #[test]
    fn every_seed_runs_and_a_failure_is_raised() {
        let ran = std::cell::RefCell::new(Vec::new());
        check(4, |rng| ran.borrow_mut().push(rng.clone().next_u64()));
        let expected: Vec<u64> = (0..4).map(|seed| Rng::new(seed).next_u64()).collect();
        assert_eq!(*ran.borrow(), expected);
        let failed = catch_unwind(|| check(3, |rng| assert!(rng.below(2) > 5, "always fails")));
        assert!(failed.is_err());
    }
}
