//! A release's checksums and what vouches for them: `SHA256SUMS.txt` in `sha256sum`'s format,
//! its Ed25519 signature `SHA256SUMS.txt.sig` (base64) and the release key's public half
//! (`update-key.pub`). The release tooling writes and signs them; every installed app reads them
//! before it trusts a download, so both ends read them here.

use base64::Engine as _;

/// The checksums' file name.
pub const FILE: &str = "SHA256SUMS.txt";
/// Their signature's file name.
pub const SIGNATURE: &str = "SHA256SUMS.txt.sig";

/// One line of the checksums: `<hex>  <name>`, as `sha256sum` writes a file read as text.
pub fn line(digest: &str, name: &str) -> String {
    format!("{digest}  {name}\n")
}

/// The digests `sums` lists for `name`, in lowercase hex: one, unless the file is missing or
/// listed again. A line is `sha256sum`'s: 64 hex digits, a space, then a space (text) or `*`
/// (binary), then the name; Windows line ends are allowed.
pub fn listed(sums: &str, name: &str) -> Vec<String> {
    sums.lines()
        .filter_map(|line| {
            let (digest, rest) = line.trim_end_matches('\r').split_once(' ')?;
            let listed = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('*'))?;
            (listed == name && digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| digest.to_ascii_lowercase())
        })
        .collect()
}

/// The one digest `sums` lists for `name`: none when it is missing or listed more than once.
pub fn digest_of(sums: &str, name: &str) -> Option<String> {
    match <[String; 1]>::try_from(listed(sums, name)) {
        Ok([digest]) => Some(digest),
        Err(_) => None,
    }
}

/// The release key in an `update-key.pub`: its one line that is not a comment, the base64 of a
/// 32-byte Ed25519 public key. None when there is no such line (no key has been made).
pub fn parse_key(text: &str) -> Option<Vec<u8>> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))?;
    base64::engine::general_purpose::STANDARD
        .decode(line)
        .ok()
        .filter(|key| key.len() == 32)
}

/// Why checksums are not vouched for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsigned {
    /// The signature is not base64.
    Unreadable,
    /// It is not the key's signature of these checksums.
    NotTheKeys,
}

/// Whether `signature` (base64, as `SHA256SUMS.txt.sig` holds it) is `key`'s Ed25519 signature
/// of `sums`.
pub fn verify(key: &[u8], sums: &[u8], signature: &str) -> Result<(), Unsigned> {
    let signature = base64::engine::general_purpose::STANDARD
        .decode(signature.trim())
        .map_err(|_| Unsigned::Unreadable)?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(sums, &signature)
        .map_err(|_| Unsigned::NotTheKeys)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    /// 2026-11-11: xtask read only `<hex>  <name>` and the app any whitespace, a `*` and a
    /// trimmed name, so a line `sha256sum -b` writes passed the app and failed xtask's check.
    /// Both read `sha256sum`'s two forms now, and nothing looser.
    #[test]
    fn the_checksums_read_as_sha256sum_writes_them() {
        let upper = HELLO.to_ascii_uppercase();
        let sums = format!(
            "{}{HELLO} *b.zip\r\n{upper}  c.deb\n{HELLO} d.exe\n{HELLO}\te.exe\n{HELLO}  f.exe \n",
            line(HELLO, "a.dmg")
        );
        assert_eq!(listed(&sums, "a.dmg"), [HELLO]);
        assert_eq!(listed(&sums, "b.zip"), [HELLO]);
        assert_eq!(listed(&sums, "c.deb"), [HELLO]);
        for name in ["d.exe", "e.exe", "f.exe", "a", "missing"] {
            assert!(listed(&sums, name).is_empty(), "{name}");
        }
        assert!(listed("abc  short.zip\n", "short.zip").is_empty());
        assert_eq!(digest_of(&sums, "a.dmg").as_deref(), Some(HELLO));
        let twice = format!("{sums}{}", line(HELLO, "a.dmg"));
        assert_eq!(digest_of(&twice, "a.dmg"), None);
        assert_eq!(digest_of(&sums, "missing"), None);
    }

    #[test]
    fn the_key_is_its_files_one_line_of_32_bytes() {
        let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        assert_eq!(parse_key("# only comments\n\n"), None);
        assert_eq!(
            parse_key(&format!("# comment\n  {}\n", encode(&[7; 32]))),
            Some(vec![7; 32])
        );
        assert_eq!(parse_key(&encode(&[7; 31])), None);
        assert_eq!(parse_key("not base64\n"), None);
    }

    #[test]
    fn checksums_are_vouched_for_by_their_keys_signature_only() {
        use ring::signature::{Ed25519KeyPair, KeyPair as _};
        let random = ring::rand::SystemRandom::new();
        let pair = || {
            let pkcs8 = Ed25519KeyPair::generate_pkcs8(&random).unwrap();
            Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
        };
        let (ours, theirs) = (pair(), pair());
        let signature =
            base64::engine::general_purpose::STANDARD.encode(ours.sign(b"sums").as_ref());
        let key = ours.public_key().as_ref();
        assert_eq!(verify(key, b"sums", &format!("{signature}\n")), Ok(()));
        assert_eq!(verify(key, b"other", &signature), Err(Unsigned::NotTheKeys));
        assert_eq!(
            verify(theirs.public_key().as_ref(), b"sums", &signature),
            Err(Unsigned::NotTheKeys)
        );
        assert_eq!(
            verify(key, b"sums", "not base64!"),
            Err(Unsigned::Unreadable)
        );
    }
}
