//! View preferences: how each page was last left (tabs, filters, sort orders, toggles), one JSON
//! value per `kchess:` key. They were the Electron app's `localStorage`; kept here, every
//! KChess app over the profile (Electron or GPUI) opens a page as it was left.

use rusqlite::Connection;
use serde_json::{Map, Value};

use super::StoreContext;
use super::documents::{read_document, write_document};
use super::json_codec::{js_json, utf16_len};
use crate::error::{CoreError, Result};

/// The document holding every preference, as `{ key: value }`.
const VIEW_PREFERENCES: &str = "view:preferences";
/// `true` once the Electron renderer's `localStorage` preferences were taken over.
const IMPORTED: &str = "view:preferences-imported";
/// One preference's largest JSON text: lists of recent items stay well below it.
const MAX_VALUE: usize = 16 * 1024;
/// The most keys kept; the app names fewer than a hundred.
const MAX_KEYS: usize = 500;

/// `kchess:` and then lowercase letters, digits, `-` and `:`, at most 100 characters.
pub fn valid_key(key: &str) -> bool {
    key.len() <= 100
        && key.strip_prefix("kchess:").is_some_and(|rest| {
            !rest.is_empty()
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b':')
        })
}

/// This module's storage methods; None when the method is not one of them.
pub fn call(ctx: &StoreContext, method: &str, args: &[Value]) -> Option<Result<Value>> {
    let db = ctx.db;
    Some(match method {
        "store.viewPreferences.all" => all(ctx, db).map(Value::Object),
        "store.viewPreferences.set" => {
            let key = args.first().and_then(Value::as_str).unwrap_or_default();
            set(ctx, db, key, args.get(1).unwrap_or(&Value::Null)).map(|()| Value::Null)
        }
        "store.viewPreferences.imported" => read_document(ctx.host, db, IMPORTED)
            .map(|done| Value::Bool(done == Some(Value::Bool(true)))),
        "store.viewPreferences.importLegacy" => {
            let texts = args.first().and_then(Value::as_object);
            import_legacy(ctx, db, texts).map(Value::Object)
        }
        _ => return None,
    })
}

fn all(ctx: &StoreContext, db: &Connection) -> Result<Map<String, Value>> {
    Ok(match read_document(ctx.host, db, VIEW_PREFERENCES)? {
        Some(Value::Object(preferences)) => preferences,
        _ => Map::new(),
    })
}

/// Take over the preferences an Electron renderer left in `localStorage` (`texts`, its text by
/// name), once: a name saved here already keeps its value, and a name or value this would refuse
/// is passed over. `useLocalStorage` kept strings as they are and everything else as JSON, so text
/// that is not JSON is a string. Answers every preference.
fn import_legacy(
    ctx: &StoreContext,
    db: &Connection,
    texts: Option<&Map<String, Value>>,
) -> Result<Map<String, Value>> {
    if read_document(ctx.host, db, IMPORTED)? == Some(Value::Bool(true)) {
        return all(ctx, db);
    }
    let mut preferences = all(ctx, db)?;
    for (key, text) in texts.into_iter().flatten() {
        let Some(text) = text.as_str() else {
            continue;
        };
        if !valid_key(key) || preferences.contains_key(key) || preferences.len() >= MAX_KEYS {
            continue;
        }
        let value = serde_json::from_str::<Value>(text).unwrap_or_else(|_| Value::from(text));
        if value.is_null() || utf16_len(&js_json(&value)) > MAX_VALUE {
            continue;
        }
        preferences.insert(key.clone(), value);
    }
    write_document(
        db,
        VIEW_PREFERENCES,
        &Value::Object(preferences.clone()),
        "Too many view preferences are saved.",
    )?;
    write_document(db, IMPORTED, &Value::Bool(true), "")?;
    Ok(preferences)
}

/// Save one preference; `null` forgets it, so the page's default applies again.
fn set(ctx: &StoreContext, db: &Connection, key: &str, value: &Value) -> Result<()> {
    if !valid_key(key) {
        return Err(CoreError::new("Invalid view preference name."));
    }
    if utf16_len(&js_json(value)) > MAX_VALUE {
        return Err(CoreError::new("This view preference is too large to save."));
    }
    let mut preferences = all(ctx, db)?;
    if value.is_null() {
        if preferences.shift_remove(key).is_none() {
            return Ok(());
        }
    } else {
        if preferences.get(key) == Some(value) {
            return Ok(());
        }
        if !preferences.contains_key(key) && preferences.len() >= MAX_KEYS {
            return Err(CoreError::new("Too many view preferences are saved."));
        }
        preferences.insert(key.to_string(), value.clone());
    }
    write_document(
        db,
        VIEW_PREFERENCES,
        &Value::Object(preferences),
        "Too many view preferences are saved.",
    )
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{Host, Level};
    use serde_json::json;

    struct Quiet;
    impl Host for Quiet {
        fn log(&self, _: Level, _: &str, _: &str) {}
        fn emit(&self, _: &str, _: Value) {}
    }

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        crate::store::migrations::migrate(&db).unwrap();
        db
    }

    fn run(db: &Connection, method: &str, args: &[Value]) -> Result<Value> {
        let config = crate::host::Config::new(std::path::PathBuf::new());
        let cache = crate::store::StoreCache::default();
        let ctx = StoreContext {
            db,
            host: &Quiet,
            config: &config,
            cache: &cache,
        };
        call(&ctx, method, args).unwrap()
    }

    #[test]
    fn preferences_are_saved_replaced_and_forgotten() {
        let db = database();
        assert_eq!(
            run(&db, "store.viewPreferences.all", &[]).unwrap(),
            json!({})
        );
        run(
            &db,
            "store.viewPreferences.set",
            &[json!("kchess:watch-tab"), json!("tv")],
        )
        .unwrap();
        run(
            &db,
            "store.viewPreferences.set",
            &[
                json!("kchess:clock-rotation"),
                json!({ "top": 180, "bottom": 0 }),
            ],
        )
        .unwrap();
        run(
            &db,
            "store.viewPreferences.set",
            &[json!("kchess:watch-tab"), json!("friends")],
        )
        .unwrap();
        assert_eq!(
            run(&db, "store.viewPreferences.all", &[]).unwrap(),
            json!({ "kchess:watch-tab": "friends", "kchess:clock-rotation": { "top": 180, "bottom": 0 } })
        );
        run(
            &db,
            "store.viewPreferences.set",
            &[json!("kchess:watch-tab"), Value::Null],
        )
        .unwrap();
        assert_eq!(
            run(&db, "store.viewPreferences.all", &[]).unwrap(),
            json!({ "kchess:clock-rotation": { "top": 180, "bottom": 0 } })
        );
    }

    #[test]
    fn names_and_sizes_are_bounded() {
        let db = database();
        for key in [
            "watch-tab",
            "kchess:",
            "kchess:Tab",
            "kchess:a b",
            &format!("kchess:{}", "a".repeat(100)),
        ] {
            assert_eq!(
                run(&db, "store.viewPreferences.set", &[json!(key), json!(1)])
                    .unwrap_err()
                    .message,
                "Invalid view preference name.",
                "{key}"
            );
        }
        assert!(valid_key("kchess:coordinates-on-squares"));
        assert!(valid_key("kchess:history-session:board"));
        let large = json!("x".repeat(MAX_VALUE));
        assert_eq!(
            run(
                &db,
                "store.viewPreferences.set",
                &[json!("kchess:big"), large]
            )
            .unwrap_err()
            .message,
            "This view preference is too large to save."
        );
    }

    /// The Electron renderer kept these in `localStorage`; the first read takes them over once,
    /// keeping any value this profile saved since.
    #[test]
    fn takes_over_electrons_preferences_once() {
        let db = database();
        run(
            &db,
            "store.viewPreferences.set",
            &[json!("kchess:studies-sort"), json!("name")],
        )
        .unwrap();
        let texts = json!({
            "kchess:studies-sort": "updated",
            "kchess:sidebar-open": "false",
            "kchess:puzzle-angle": "{\"theme\":\"fork\"}",
            "kchess:studies-tab": "lichess",
            "kchess:studies:v1": "[]",
            "Not a preference": "x",
        });
        let all = run(&db, "store.viewPreferences.importLegacy", &[texts]).unwrap();
        assert_eq!(
            all["kchess:studies-sort"],
            json!("name"),
            "saved here since"
        );
        assert_eq!(all["kchess:sidebar-open"], json!(false));
        assert_eq!(all["kchess:puzzle-angle"], json!({ "theme": "fork" }));
        assert_eq!(all["kchess:studies-tab"], json!("lichess"));
        assert!(all.get("Not a preference").is_none());

        run(
            &db,
            "store.viewPreferences.set",
            &[json!("kchess:studies-tab"), Value::Null],
        )
        .unwrap();
        let again = json!({ "kchess:studies-tab": "lichess", "kchess:chat-open": "true" });
        let all = run(&db, "store.viewPreferences.importLegacy", &[again]).unwrap();
        assert!(all.get("kchess:studies-tab").is_none(), "only once");
        assert!(all.get("kchess:chat-open").is_none());
    }
}
