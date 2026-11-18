//! Judging a memory benchmark's heap samples.

/// Bytes each cycle keeps, judged from `samples` (the settled heap after each batch of
/// `per_batch` cycles, from 0): the smallest batch's growth. Memory the cycles keep grows every
/// batch by about the same; a map or buffer sized by the work grows to its working capacity in a
/// few steps, in whichever batches first need it, and leaves the others flat. The median batch
/// still landed on such a step when three of six batches held one (CI's coverage build).
pub fn kept_per_cycle(samples: &[isize], per_batch: usize) -> isize {
    let smallest = samples
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .min()
        .unwrap_or(0);
    smallest.max(0) / per_batch.max(1) as isize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The judging rule against the trails CI recorded and a steady leak (KiB as printed).
    #[test]
    fn growth_in_steps_is_capacity_and_growth_in_every_batch_is_kept() {
        let trail =
            |kib: &[f64]| -> Vec<isize> { kib.iter().map(|k| (k * 1024.) as isize).collect() };
        // `a_long_connection_with_calls_in_flight`, CI coverage builds: 33 and 49 B "per cycle" by
        // the median batch, though the heap only stepped up to its working size.
        assert_eq!(
            kept_per_cycle(&trail(&[0.0, 0.0, 2.2, 2.2, 3.5, 5.6, 5.6]), 40),
            0
        );
        assert_eq!(
            kept_per_cycle(&trail(&[0.0, 2.1, 4.3, 4.3, 4.3, 6.2, 6.2]), 40),
            0
        );
        // 432 B a cycle (the idle timer `6b9f89d` fixed) grows every batch of 40 by about 17 KiB.
        let leak: Vec<isize> = (0..7)
            .map(|batch| batch * 432 * 40 + batch % 2 * 300)
            .collect();
        assert!(kept_per_cycle(&leak, 40) >= 420);
    }
}
