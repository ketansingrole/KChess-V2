//! Random ids and their text.

use crate::error::{CoreError, Result};

/// Lowercase hexadecimal of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A version 4 UUID over `bytes` (16 random bytes).
fn uuid_of(bytes: [u8; 16]) -> String {
    uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .hyphenated()
        .to_string()
}

/// A new random version 4 UUID.
pub fn random_uuid() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|cause| CoreError::wrap("random id", cause))?;
    Ok(uuid_of(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uuid_is_version_4_and_random() {
        let id = random_uuid().expect("random bytes");
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"), "{id}");
        assert_ne!(id, random_uuid().expect("random bytes"));
        assert_eq!(uuid_of([0xff; 16]), "ffffffff-ffff-4fff-bfff-ffffffffffff");
        assert_eq!(hex(&[0, 15, 255]), "000fff");
    }
}
