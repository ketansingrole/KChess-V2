//! Work a view repeats while it lives: a running clock's redraw, a poll, a periodic check.

use std::time::Duration;

use gpui_kit::{Context, Task};

/// How often a running clock is looked at: a face shows tenths in its last ten seconds. A tick
/// redraws only what changed (`game::surface::Faces`), so every clock in the app uses it.
pub const TICK: Duration = Duration::from_millis(100);

/// Run `tick` every `period` while the view lives and `tick` says to go on; dropping the task
/// stops it.
pub fn every<V: 'static>(
    period: Duration,
    cx: &mut Context<V>,
    mut tick: impl FnMut(&mut V, &mut Context<V>) -> bool + 'static,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(period).await;
            match this.update(cx, |this, cx| tick(this, cx)) {
                Ok(true) => {}
                Ok(false) | Err(_) => break,
            }
        }
    })
}

/// `every`, for a view that only redraws: each period it is drawn again.
pub fn redraw_every<V: 'static>(period: Duration, cx: &mut Context<V>) -> Task<()> {
    every(period, cx, |_, cx| {
        cx.notify();
        true
    })
}
