//! View preferences: how each page was last left (tabs, filters, sort orders, toggles), one JSON
//! value per `kchess:` key. Kept here, every KChess app over
//! the profile opens a page as it was left.

use serde_json::{Map, Value};

use super::StoreContext;
use super::documents::{read_document, write_document};
use super::json_codec::{js_json, utf16_len};
use crate::error::{CoreError, Result};

/// The document holding every preference, as `{ key: value }`.
const VIEW_PREFERENCES: &str = "view:preferences";
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

pub fn all(ctx: &StoreContext) -> Result<Map<String, Value>> {
    Ok(match read_document(ctx.host, ctx.db, VIEW_PREFERENCES)? {
        Some(Value::Object(preferences)) => preferences,
        _ => Map::new(),
    })
}

/// Save one preference; `null` forgets it, so the page's default applies again.
pub fn set(ctx: &StoreContext, key: &str, value: &Value) -> Result<()> {
    if !valid_key(key) {
        return Err(CoreError::new("Invalid view preference name."));
    }
    if utf16_len(&js_json(value)) > MAX_VALUE {
        return Err(CoreError::new("This view preference is too large to save."));
    }
    let mut preferences = all(ctx)?;
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
        ctx.db,
        VIEW_PREFERENCES,
        &Value::Object(preferences),
        "Too many view preferences are saved.",
    )
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::in_memory;
    use serde_json::json;

    #[test]
    fn preferences_are_saved_replaced_and_forgotten() {
        in_memory(|ctx| {
            assert_eq!(all(ctx).unwrap(), Map::new());
            set(ctx, "kchess:watch-tab", &json!("tv")).unwrap();
            let rotation = json!({ "top": 180, "bottom": 0 });
            set(ctx, "kchess:clock-rotation", &rotation).unwrap();
            set(ctx, "kchess:watch-tab", &json!("friends")).unwrap();
            assert_eq!(
                Value::Object(all(ctx).unwrap()),
                json!({ "kchess:watch-tab": "friends", "kchess:clock-rotation": rotation })
            );
            set(ctx, "kchess:watch-tab", &Value::Null).unwrap();
            assert_eq!(
                Value::Object(all(ctx).unwrap()),
                json!({ "kchess:clock-rotation": rotation })
            );
        });
    }

    #[test]
    fn names_and_sizes_are_bounded() {
        in_memory(|ctx| {
            for key in [
                "watch-tab",
                "kchess:",
                "kchess:Tab",
                "kchess:a b",
                &format!("kchess:{}", "a".repeat(100)),
            ] {
                assert_eq!(
                    set(ctx, key, &json!(1)).unwrap_err().message,
                    "Invalid view preference name.",
                    "{key}"
                );
            }
            assert!(valid_key("kchess:coordinates-on-squares"));
            assert!(valid_key("kchess:history-session:board"));
            let large = json!("x".repeat(MAX_VALUE));
            assert_eq!(
                set(ctx, "kchess:big", &large).unwrap_err().message,
                "This view preference is too large to save."
            );
        });
    }
}
