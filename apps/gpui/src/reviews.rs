//! Game reviews, as the core works them out: the reviews seen lately, kept in
//! memory, and the review queue's progress. Updates stream in as core events (`review:update`,
//! `review:status`).

use kchess_contracts::calls;
use std::collections::VecDeque;

use gpui_kit::{App, AppContext as _, Context, Entity, Global, Subscription};
use kchess_contracts::Level;
use kchess_contracts::check::{Fen, Moves};
use kchess_contracts::types::{ReviewCurrent, ReviewSide, ReviewStatus, StoredReview};

use crate::core::{AppCore, ClientEvent, CoreEvent};

/// Full reviews kept in memory, least recently seen dropped first; the core keeps them all.
const CACHE_SIZE: usize = 24;

/// The reviews seen lately, least recently seen first; the pinned one stays however many others
/// come in.
#[derive(Default)]
pub struct ReviewCache {
    reviews: VecDeque<StoredReview>,
    pinned: String,
}

impl ReviewCache {
    pub fn get(&self, key: &str) -> Option<&StoredReview> {
        self.reviews.iter().find(|review| review.key == key)
    }

    pub fn remember(&mut self, review: StoredReview) {
        self.reviews.retain(|known| known.key != review.key);
        self.reviews.push_back(review);
        while self.reviews.len() > CACHE_SIZE {
            let Some(index) = self
                .reviews
                .iter()
                .position(|known| known.key != self.pinned)
            else {
                break;
            };
            self.reviews.remove(index);
        }
    }
}

pub struct ReviewStore {
    pub cache: ReviewCache,
    /// `ReviewStatus`: the review being worked on, the queue, the last failure.
    pub status: ReviewStatus,
    /// Status events received, so a status read never replaces one that streamed in meanwhile.
    status_events: u64,
    _events: Subscription,
}

/// How views reach the reviews: `cx.global::<ReviewHandle>().0`.
#[derive(Clone)]
pub struct ReviewHandle(pub Entity<ReviewStore>);

impl Global for ReviewHandle {}

pub fn init(cx: &mut App) {
    let events = cx.global::<AppCore>().events.clone();
    let store = cx.new(|cx: &mut Context<ReviewStore>| ReviewStore {
        cache: ReviewCache::default(),
        status: ReviewStatus::default(),
        status_events: 0,
        _events: cx.subscribe(&events, |this, _, event: &ClientEvent, cx| {
            match event {
                ClientEvent::Event(CoreEvent::ReviewUpdate(update)) => {
                    this.cache.remember(update.review.clone());
                    cx.notify();
                }
                ClientEvent::Event(CoreEvent::ReviewStatus(status)) => {
                    this.status_events += 1;
                    this.status = status.clone();
                    cx.notify();
                }
                // The events missed meanwhile are not sent again.
                ClientEvent::Reconnected => this.resynchronize(cx),
                _ => {}
            }
        }),
    });
    store.update(cx, |store, cx| store.read_status(cx));
    cx.set_global(ReviewHandle(store));
}

impl ReviewStore {
    /// Read the queue's status, unless a status event arrives first.
    fn read_status(&mut self, cx: &mut Context<Self>) {
        let seen = self.status_events;
        let status = cx.global::<AppCore>().request(calls::ReviewStatus {});
        cx.spawn(async move |this, cx| match status.await {
            Ok(status) => {
                let _ = this.update(cx, |store, cx| {
                    if store.status_events == seen {
                        store.status = status;
                        cx.notify();
                    }
                });
            }
            Err(cause) => crate::log::write(
                Level::Warn,
                "review",
                &format!("reading review status failed: {}", cause.message),
            ),
        })
        .detach();
    }

    /// After reconnecting, read the status and the board's unfinished review once more. Nothing
    /// is reviewed again.
    fn resynchronize(&mut self, cx: &mut Context<Self>) {
        self.read_status(cx);
        let Some(review) = self.cache.get(&self.cache.pinned).cloned() else {
            return;
        };
        if review.complete {
            return;
        }
        let found = cx.global::<AppCore>().request(calls::ReviewGet {
            fen: Fen(review.fen.clone()),
            moves: Moves(review.moves.clone()),
            variant: None,
        });
        cx.spawn(async move |this, cx| match found.await {
            Ok(Some(found)) => {
                let _ = this.update(cx, |store, cx| {
                    // A review that streamed in meanwhile is at least as new.
                    let known = store.cache.get(&found.key);
                    if known.is_none_or(|known| known.updated_at == review.updated_at) {
                        store.cache.remember(found);
                        cx.notify();
                    }
                });
            }
            Ok(_) => {}
            Err(cause) => crate::log::write(
                Level::Warn,
                "review",
                &format!("reloading review failed: {}", cause.message),
            ),
        })
        .detach();
    }

    pub fn get(&self, key: &str) -> Option<&StoredReview> {
        self.cache.get(key)
    }

    /// Keep the review on the board whatever else streams in.
    pub fn pin(&mut self, key: &str) {
        self.cache.pinned = key.to_string();
    }

    /// The review being worked on (`current`), when it is this one.
    pub fn progress(&self, key: &str) -> Option<&ReviewCurrent> {
        self.status
            .current
            .as_ref()
            .filter(|current| current.key == key)
    }

    /// Why the review asked for could not be done, when it is this one.
    pub fn failure(&self, key: &str) -> Option<String> {
        self.status
            .failed
            .as_ref()
            .filter(|failed| failed.key == key)
            .map(|failed| failed.message.clone())
    }
}

/// "2 blunders", worst first, leaving out the kinds that did not happen, from a
/// side's counts.
pub fn judgment_counts(side: &ReviewSide) -> Vec<(&'static str, String)> {
    [
        ("blunder", "blunders", side.blunder),
        ("mistake", "mistakes", side.mistake),
        ("inaccuracy", "inaccuracies", side.inaccuracy),
    ]
    .into_iter()
    .filter(|(_, _, count)| *count > 0)
    .map(|(judgment, plural, count)| {
        (
            judgment,
            format!("{count} {}", if count == 1 { judgment } else { plural }),
        )
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::{CACHE_SIZE, ReviewCache, judgment_counts};

    #[test]
    fn judgments_are_counted_worst_first() {
        let counts = judgment_counts(&ReviewSide {
            blunder: 1,
            inaccuracy: 3,
            ..ReviewSide::default()
        });
        assert_eq!(
            counts,
            vec![
                ("blunder", "1 blunder".to_string()),
                ("inaccuracy", "3 inaccuracies".to_string())
            ]
        );
    }
    use kchess_contracts::types::{ReviewSide, ReviewSource, StoredReview};

    fn review(key: &str, complete: bool) -> StoredReview {
        StoredReview {
            key: key.into(),
            fen: String::new(),
            moves: Vec::new(),
            source: ReviewSource::Local,
            engine: None,
            evals: Vec::new(),
            depth: 0,
            complete,
            updated_at: 0,
            game_id: None,
            judgments: None,
            accuracy: None,
            variant: None,
            rest: serde_json::Map::new(),
        }
    }

    #[test]
    fn the_pinned_review_outlasts_the_cache() {
        let mut cache = ReviewCache {
            pinned: "k0".into(),
            ..ReviewCache::default()
        };
        for index in 0..30 {
            cache.remember(review(&format!("k{index}"), false));
        }
        assert_eq!(cache.reviews.len(), CACHE_SIZE);
        assert!(cache.get("k0").is_some());
        assert!(cache.get("k1").is_none());
        assert!(cache.get("k29").is_some());
        // Seen again, a review moves to the newest end.
        cache.remember(review("k10", true));
        assert!(cache.get("k10").is_some_and(|review| review.complete));
    }
}
