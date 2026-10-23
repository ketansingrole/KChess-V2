//! Game reviews, as the core works them out (`stores/review.ts`): the reviews seen lately, kept in
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
        _events: cx.subscribe(&events, |this, _, event: &CoreEvent, cx| {
            match event.name.as_str() {
                "review:update" => {
                    this.cache.remember(event.payload["review"].clone());
                    cx.notify();
                }
                "review:status" => {
                    this.status = event.payload.clone();
                    cx.notify();
                }
                _ => {}
            }
        }),
    });
    let status = cx.global::<AppCore>().call("reviewStatus", Vec::new());
    let target = store.clone();
    cx.spawn(async move |cx| match status.await {
        Ok(status) => target.update(cx, |store, cx| {
            store.status = status;
            cx.notify();
        }),
        Err(cause) => crate::log::write(
            Level::Warn,
            "review",
            &format!("reading review status failed: {}", cause.message),
        ),
    })
    .detach();
    cx.set_global(ReviewHandle(store));
}

impl ReviewStore {
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

#[cfg(test)]
mod tests {
    use super::{CACHE_SIZE, ReviewCache};
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
