//! Game reviews, as the core works them out: the reviews seen lately, kept in
//! memory, and the review queue's progress. Updates stream in as core events (`review:update`,
//! `review:status`).

use std::collections::VecDeque;

use gpui_kit::{App, AppContext as _, Context, Entity, Global, Subscription};
use kchess_core::Level;
use serde_json::{Value, json};

use crate::core::{AppCore, CoreEvent};

/// Full reviews kept in memory, least recently seen dropped first; the core keeps them all.
const CACHE_SIZE: usize = 24;

/// The reviews seen lately, least recently seen first; the pinned one stays however many others
/// come in.
#[derive(Default)]
pub struct ReviewCache {
    reviews: VecDeque<Value>,
    pinned: String,
}

impl ReviewCache {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.reviews
            .iter()
            .find(|review| review["key"] == json!(key))
    }

    pub fn remember(&mut self, review: Value) {
        if review.is_null() {
            return;
        }
        let key = review["key"].clone();
        self.reviews.retain(|known| known["key"] != key);
        self.reviews.push_back(review);
        while self.reviews.len() > CACHE_SIZE {
            let Some(index) = self
                .reviews
                .iter()
                .position(|known| known["key"] != json!(self.pinned))
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
    pub status: Value,
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
        status: json!({ "waiting": 0 }),
        status_events: 0,
        _events: cx.subscribe(&events, |this, _, event: &CoreEvent, cx| {
            match event.name.as_str() {
                "review:update" => {
                    this.cache.remember(event.payload["review"].clone());
                    cx.notify();
                }
                "review:status" => {
                    this.status_events += 1;
                    this.status = event.payload.clone();
                    cx.notify();
                }
                // The events missed meanwhile are not sent again.
                crate::core::RECONNECTED => this.resynchronize(cx),
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
        let status = cx.global::<AppCore>().call("reviewStatus", Vec::new());
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
        if review["complete"].as_bool() == Some(true) {
            return;
        }
        let found = cx.global::<AppCore>().call(
            "reviewGet",
            vec![review["fen"].clone(), review["moves"].clone()],
        );
        cx.spawn(async move |this, cx| match found.await {
            Ok(found) if !found.is_null() => {
                let _ = this.update(cx, |store, cx| {
                    // A review that streamed in meanwhile is at least as new.
                    let known = store.cache.get(found["key"].as_str().unwrap_or_default());
                    if known.is_none_or(|known| known["updatedAt"] == review["updatedAt"]) {
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

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.cache.get(key)
    }

    /// Keep the review on the board whatever else streams in.
    pub fn pin(&mut self, key: &str) {
        self.cache.pinned = key.to_string();
    }

    /// The review being worked on (`current`), when it is this one.
    pub fn progress(&self, key: &str) -> Option<&Value> {
        let current = &self.status["current"];
        (current["key"] == json!(key)).then_some(current)
    }

    /// Why the review asked for could not be done, when it is this one.
    pub fn failure(&self, key: &str) -> Option<String> {
        let failed = &self.status["failed"];
        (failed["key"] == json!(key))
            .then(|| failed["message"].as_str().unwrap_or_default().to_string())
    }
}

/// `judgmentCounts`: "2 blunders", worst first, leaving out the kinds that did not happen, from a
/// side's counts (`ReviewSide`: `blunder`, `mistake`, `inaccuracy`).
pub fn judgment_counts(side: &Value) -> Vec<(&'static str, String)> {
    [
        ("blunder", "blunders"),
        ("mistake", "mistakes"),
        ("inaccuracy", "inaccuracies"),
    ]
    .into_iter()
    .filter_map(|(judgment, plural)| {
        let count = side[judgment].as_u64().unwrap_or_default();
        (count > 0).then(|| {
            (
                judgment,
                format!("{count} {}", if count == 1 { judgment } else { plural }),
            )
        })
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::{CACHE_SIZE, ReviewCache, judgment_counts};

    #[test]
    fn judgments_are_counted_worst_first() {
        let counts = judgment_counts(&json!({ "blunder": 1, "mistake": 0, "inaccuracy": 3 }));
        assert_eq!(
            counts,
            vec![
                ("blunder", "1 blunder".to_string()),
                ("inaccuracy", "3 inaccuracies".to_string())
            ]
        );
    }
    use serde_json::json;

    #[test]
    fn the_pinned_review_outlasts_the_cache() {
        let mut cache = ReviewCache {
            pinned: "k0".into(),
            ..ReviewCache::default()
        };
        for index in 0..30 {
            cache.remember(json!({ "key": format!("k{index}") }));
        }
        assert_eq!(cache.reviews.len(), CACHE_SIZE);
        assert!(cache.get("k0").is_some());
        assert!(cache.get("k1").is_none());
        assert!(cache.get("k29").is_some());
        // Seen again, a review moves to the newest end.
        cache.remember(json!({ "key": "k10", "complete": true }));
        assert_eq!(cache.get("k10").unwrap()["complete"], json!(true));
    }
}
