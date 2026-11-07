//! The record that what the Electron app left in this profile was taken over
//! (`electron_migration`): the release that finished it, or nothing yet.

use serde_json::Value;

use super::StoreContext;
use super::documents::{read_document, write_document};
use crate::electron_migration::MARKER;
use crate::error::{CoreError, Result};

/// This module's storage methods; None when the method is not one of them.
pub fn call(ctx: &StoreContext, method: &str, args: &[Value]) -> Option<Result<Value>> {
    Some(match method {
        "store.electron.migrated" => {
            read_document(ctx.host, ctx.db, MARKER).map(|found| found.unwrap_or(Value::Null))
        }
        "store.electron.markMigrated" => match args.first().and_then(Value::as_str) {
            Some(release) if !release.is_empty() => write_document(
                ctx.db,
                MARKER,
                &serde_json::json!({ "release": release }),
                "",
            ),
            _ => Err(CoreError::new("The migrating release is missing.")),
        },
        _ => return None,
    })
}
