//! The release key: an Ed25519 pair whose secret half signs every release's `SHA256SUMS.txt`
//! (`SHA256SUMS.txt.sig`) and whose public half the app carries (`apps/gpui/update-key.pub`), so
//! an update whose packages and checksums were both replaced on GitHub is refused.

use std::path::Path;

use base64::Engine as _;
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair as _, UnparsedPublicKey};

use crate::util::{Result, root, say};

/// The environment variable holding the secret half (base64 PKCS#8) in the release job.
const SECRET: &str = "KCHESS_UPDATE_SIGNING_KEY";

fn key_file() -> std::path::PathBuf {
    root().join("apps/gpui/update-key.pub")
}

/// The committed public key, if one has been made: the base64 of 32 bytes on the file's one line
/// that is not a comment, as the app reads it (`apps/gpui/src/updates.rs`, `parse_update_key`).
pub fn public_key() -> Option<Vec<u8>> {
    let text = std::fs::read_to_string(key_file()).ok()?;
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))?;
    base64::engine::general_purpose::STANDARD
        .decode(line)
        .ok()
        .filter(|key| key.len() == 32)
}

/// The committed public key, which every release step that signs or checks needs.
pub fn require_public_key() -> Result<Vec<u8>> {
    public_key().ok_or_else(|| {
        "apps/gpui/update-key.pub holds no release key, so installed apps could not tell a \
         release's checksums from replaced ones. Run `cargo xtask update-key`."
            .into()
    })
}

/// `update-key <secret file>`: make the pair, write the secret half to `secret` (owner-only,
/// outside the repository) and the public half into `update-key.pub`.
pub fn generate(secret: &Path) -> Result {
    if public_key().is_some() {
        return Err(
            "apps/gpui/update-key.pub already holds a key. Replacing it locks out every \
             installed app's updates: remove it by hand only if the secret is lost."
                .into(),
        );
    }
    let absolute = std::path::absolute(secret).map_err(|cause| cause.to_string())?;
    if absolute.starts_with(root()) {
        return Err("Write the secret key outside the repository.".into());
    }
    let random = ring::rand::SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&random)
        .map_err(|_| "Could not generate a key".to_string())?;
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
        .map_err(|_| "Could not read the generated key".to_string())?;
    let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    write_private(&absolute, &encode(pkcs8.as_ref()))?;
    let mut text = std::fs::read_to_string(key_file()).map_err(|cause| cause.to_string())?;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&encode(pair.public_key().as_ref()));
    text.push('\n');
    std::fs::write(key_file(), text).map_err(|cause| cause.to_string())?;
    say(&format!(
        "Wrote the public key to apps/gpui/update-key.pub and the secret to {}.\n\
         Store the secret as the `release` environment's {SECRET} \
         (gh secret set {SECRET} --env release < {}), keep a copy in a password manager, \
         delete the file, and commit update-key.pub.",
        absolute.display(),
        absolute.display()
    ));
    Ok(())
}

#[cfg(unix)]
fn write_private(path: &Path, text: &str) -> Result {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|cause| format!("Could not create {}: {cause}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|cause| cause.to_string())
}

#[cfg(not(unix))]
fn write_private(path: &Path, text: &str) -> Result {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|cause| format!("Could not create {}: {cause}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|cause| cause.to_string())
}

/// `sign-sums <file>`: sign `file` with the secret in `KCHESS_UPDATE_SIGNING_KEY` into
/// `<file>.sig`, and check the signature against the committed public key, so a wrong secret
/// fails the release instead of every app's update.
pub fn sign(file: &Path) -> Result {
    let public = require_public_key()?;
    let secret = std::env::var(SECRET).map_err(|_| format!("{SECRET} is not set."))?;
    let pkcs8 = base64::engine::general_purpose::STANDARD
        .decode(secret.trim())
        .map_err(|_| format!("{SECRET} is not base64."))?;
    let pair = Ed25519KeyPair::from_pkcs8(&pkcs8)
        .map_err(|_| format!("{SECRET} is not an Ed25519 key."))?;
    let sums = std::fs::read(file).map_err(|cause| format!("{}: {cause}", file.display()))?;
    let signature = signature_of(&pair, &sums);
    check(&public, &sums, &signature)?;
    let mut target = file.as_os_str().to_owned();
    target.push(".sig");
    std::fs::write(&target, format!("{signature}\n")).map_err(|cause| cause.to_string())?;
    say(&format!("Signed {}", file.display()));
    Ok(())
}

fn signature_of(pair: &Ed25519KeyPair, bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(pair.sign(bytes).as_ref())
}

fn check(public: &[u8], bytes: &[u8], signature: &str) -> Result {
    let signature = base64::engine::general_purpose::STANDARD
        .decode(signature)
        .map_err(|_| "The signature is not base64.".to_string())?;
    UnparsedPublicKey::new(&ED25519, public)
        .verify(bytes, &signature)
        .map_err(|_| {
            format!("{SECRET} does not match apps/gpui/update-key.pub: the app would refuse it.")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_checks_only_against_its_own_key() {
        let random = ring::rand::SystemRandom::new();
        let pair = |random| {
            let pkcs8 = Ed25519KeyPair::generate_pkcs8(random).unwrap();
            Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
        };
        let (ours, theirs) = (pair(&random), pair(&random));
        let signature = signature_of(&ours, b"sums");
        assert!(check(ours.public_key().as_ref(), b"sums", &signature).is_ok());
        assert!(check(ours.public_key().as_ref(), b"other", &signature).is_err());
        assert!(check(theirs.public_key().as_ref(), b"sums", &signature).is_err());
    }
}
