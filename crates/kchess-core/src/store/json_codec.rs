//! Study-name cleanup shared by the storage modules. Stored documents are written with
//! `kchess_domain::js::stringify`, so they keep matching the TypeScript bytes.

use kchess_domain::js;

/// `name.trim().slice(0, 120)`
pub fn clean_name(name: &str) -> String {
    js::slice_utf16(js::trim(name), 120).to_string()
}
