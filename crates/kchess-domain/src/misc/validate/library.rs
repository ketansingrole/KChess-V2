//! Validators of the library's ids.

use super::*;

/// A local study or chapter id: 1 to 80 characters.
pub(super) fn local_id(id: &str, path: &str) -> Checked<()> {
    min_chars(id, 1, None, path)?;
    max_chars(id, 80, None, path)
}

/// A library entry's id: 1 to 80 characters.
pub(super) fn assert_library_id(value: J) -> Checked<String> {
    let id: String = decode(value, plain)?;
    local_id(&id, "")?;
    Ok(id)
}
