//! The notification centre's history: the notifications KChess showed, newest first, as one
//! bounded, versioned document. It keeps at most `MAX_ENTRIES`, none older than `RETENTION`.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::documents::{DAMAGED_DOCUMENT, read_document, read_document_text, write_document};
use crate::host::{Host, Level};
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{NotificationEntry, NotificationInput};

/// The document holding the history.
const HISTORY: &str = "notifications:history";
/// The document's shape: a later one is never written over by this release.
const VERSION: u32 = 1;
/// The most notifications kept.
pub const MAX_ENTRIES: usize = 100;
/// How long one is kept: 30 days, in milliseconds.
pub const RETENTION: f64 = 30. * 24. * 60. * 60. * 1000.;
const TOO_LARGE: &str = "Too many notifications are kept.";

#[derive(Serialize, Deserialize)]
struct History {
    version: u32,
    entries: Vec<NotificationEntry>,
}

/// The history as stored, or none when this release cannot read it in full (damaged, or from a
/// newer KChess), so a write never replaces it.
fn stored(host: &dyn Host, db: &Connection) -> Result<Option<Vec<NotificationEntry>>> {
    let Some(value) = read_document(host, db, HISTORY)? else {
        // Missing is empty; text that is not JSON is damaged.
        return Ok(read_document_text(db, HISTORY)?.is_none().then(Vec::new));
    };
    Ok(match serde_json::from_value::<History>(value) {
        Ok(history) => (history.version == VERSION).then_some(history.entries),
        Err(cause) => {
            host.log(
                Level::Warn,
                "notifications",
                &format!("The notification history is unreadable: {cause}"),
            );
            None
        }
    })
}

/// What is kept at `now`: newest first, the last 30 days, at most `MAX_ENTRIES`.
fn kept(mut entries: Vec<NotificationEntry>, now: f64) -> Vec<NotificationEntry> {
    entries.retain(|entry| entry.at >= now - RETENTION);
    entries.sort_by(|a, b| b.at.total_cmp(&a.at));
    entries.truncate(MAX_ENTRIES);
    entries
}

/// The notifications kept at `now`, newest first. Unreadable, there are none to show.
pub fn list(host: &dyn Host, db: &Connection, now: f64) -> Result<Vec<NotificationEntry>> {
    Ok(kept(stored(host, db)?.unwrap_or_default(), now))
}

/// Change the history with `change`, then keep what `now` keeps, and answer it.
fn change(
    host: &dyn Host,
    db: &Connection,
    now: f64,
    change: impl FnOnce(&mut Vec<NotificationEntry>),
) -> Result<Vec<NotificationEntry>> {
    let Some(mut entries) = stored(host, db)? else {
        return Err(CoreError::new(DAMAGED_DOCUMENT));
    };
    let before = entries.clone();
    change(&mut entries);
    let entries = kept(entries, now);
    if entries != before {
        let history = History {
            version: VERSION,
            entries: entries.clone(),
        };
        let value =
            serde_json::to_value(&history).map_err(|e| CoreError::wrap("notifications", e))?;
        write_document(db, HISTORY, &value, TOO_LARGE)?;
    }
    Ok(entries)
}

/// Keep `input`, arrived at `now`, as `id`. One with the key of a kept one replaces it, unless it
/// says the same, which leaves the kept one as it was (read or not).
pub fn record(
    host: &dyn Host,
    db: &Connection,
    input: NotificationInput,
    id: String,
    now: f64,
) -> Result<Vec<NotificationEntry>> {
    change(host, db, now, |entries| {
        if let Some(key) = &input.key {
            if let Some(at) = entries.iter().position(|e| e.key.as_ref() == Some(key)) {
                if entries[at].says(&input) {
                    return;
                }
                entries.remove(at);
            }
        }
        entries.insert(
            0,
            NotificationEntry {
                id,
                at: now,
                title: input.title,
                body: input.body,
                link: input.link,
                read: input.read,
                key: input.key,
            },
        );
    })
}

/// Mark `id` read, or every notification when `id` is none.
pub fn mark_read(
    host: &dyn Host,
    db: &Connection,
    id: Option<&str>,
    now: f64,
) -> Result<Vec<NotificationEntry>> {
    change(host, db, now, |entries| {
        for entry in entries.iter_mut() {
            if id.is_none_or(|id| entry.id == id) {
                entry.read = true;
            }
        }
    })
}

/// Remove `id`, or every notification when `id` is none.
pub fn clear(
    host: &dyn Host,
    db: &Connection,
    id: Option<&str>,
    now: f64,
) -> Result<Vec<NotificationEntry>> {
    change(host, db, now, |entries| {
        entries.retain(|entry| id.is_some_and(|id| entry.id != id))
    })
}

/// A history document written by hand, for the tests.
#[cfg(test)]
fn write_raw(db: &Connection, value: &serde_json::Value) {
    write_document(db, HISTORY, value, TOO_LARGE).expect("written");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::in_memory;
    use kchess_contracts::types::NotificationLink;
    use serde_json::json;

    const DAY: f64 = 24. * 60. * 60. * 1000.;
    const NOW: f64 = 1_800_000_000_000.;

    fn input(title: &str) -> NotificationInput {
        NotificationInput {
            title: title.into(),
            ..NotificationInput::default()
        }
    }

    fn titles(entries: &[NotificationEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.title.as_str()).collect()
    }

    #[test]
    fn the_history_keeps_the_newest_hundred() {
        in_memory(|ctx| {
            for n in 0..(MAX_ENTRIES + 5) {
                let at = NOW + n as f64;
                record(
                    ctx.host,
                    ctx.db,
                    input(&format!("n{n}")),
                    format!("id{n}"),
                    at,
                )
                .unwrap();
            }
            let entries = list(ctx.host, ctx.db, NOW + 200.).unwrap();
            assert_eq!(entries.len(), MAX_ENTRIES);
            assert_eq!(entries[0].title, format!("n{}", MAX_ENTRIES + 4));
            assert_eq!(entries[MAX_ENTRIES - 1].title, "n5");
        });
    }

    #[test]
    fn a_notification_older_than_thirty_days_is_dropped() {
        in_memory(|ctx| {
            record(ctx.host, ctx.db, input("old"), "a".into(), NOW).unwrap();
            record(ctx.host, ctx.db, input("new"), "b".into(), NOW + 20. * DAY).unwrap();
            assert_eq!(
                titles(&list(ctx.host, ctx.db, NOW + 29. * DAY).unwrap()),
                ["new", "old"]
            );
            // Read after 30 days, it is no longer shown; the next write leaves it out.
            assert_eq!(
                titles(&list(ctx.host, ctx.db, NOW + 31. * DAY).unwrap()),
                ["new"]
            );
            let after = mark_read(ctx.host, ctx.db, None, NOW + 31. * DAY).unwrap();
            assert_eq!(titles(&after), ["new"]);
            assert_eq!(titles(&list(ctx.host, ctx.db, NOW).unwrap()), ["new"]);
        });
    }

    #[test]
    fn read_and_cleared_one_at_a_time_or_all() {
        in_memory(|ctx| {
            for (n, title) in ["a", "b", "c"].iter().enumerate() {
                record(
                    ctx.host,
                    ctx.db,
                    input(title),
                    title.to_string(),
                    NOW + n as f64,
                )
                .unwrap();
            }
            let entries = mark_read(ctx.host, ctx.db, Some("b"), NOW).unwrap();
            let read: Vec<_> = entries.iter().map(|e| e.read).collect();
            assert_eq!(read, [false, true, false]);
            assert_eq!(
                titles(&clear(ctx.host, ctx.db, Some("c"), NOW).unwrap()),
                ["b", "a"]
            );
            assert!(
                mark_read(ctx.host, ctx.db, None, NOW)
                    .unwrap()
                    .iter()
                    .all(|e| e.read)
            );
            assert!(clear(ctx.host, ctx.db, None, NOW).unwrap().is_empty());
            assert!(list(ctx.host, ctx.db, NOW).unwrap().is_empty());
        });
    }

    #[test]
    fn the_same_key_replaces_or_leaves_the_one_kept() {
        in_memory(|ctx| {
            let update = |version: &str| NotificationInput {
                title: format!("KChess {version} is available"),
                link: NotificationLink::Update {
                    version: version.into(),
                },
                key: Some("update".into()),
                ..NotificationInput::default()
            };
            record(ctx.host, ctx.db, update("2026.12.1"), "a".into(), NOW).unwrap();
            mark_read(ctx.host, ctx.db, None, NOW).unwrap();
            // Announced again on the next start: still the one entry, still read.
            let again =
                record(ctx.host, ctx.db, update("2026.12.1"), "b".into(), NOW + 1.).unwrap();
            assert_eq!(again.len(), 1);
            assert_eq!((again[0].id.as_str(), again[0].read), ("a", true));
            // A newer version replaces it, unread.
            let newer =
                record(ctx.host, ctx.db, update("2026.12.2"), "c".into(), NOW + 2.).unwrap();
            assert_eq!(newer.len(), 1);
            assert_eq!((newer[0].id.as_str(), newer[0].read), ("c", false));
        });
    }

    #[test]
    fn a_history_from_a_newer_kchess_is_never_replaced() {
        in_memory(|ctx| {
            write_raw(ctx.db, &json!({ "version": 2, "entries": [] }));
            assert!(list(ctx.host, ctx.db, NOW).unwrap().is_empty());
            let refused = record(ctx.host, ctx.db, input("x"), "a".into(), NOW).unwrap_err();
            assert_eq!(refused.message, DAMAGED_DOCUMENT);
            assert!(clear(ctx.host, ctx.db, None, NOW).is_err());
        });
    }
}
