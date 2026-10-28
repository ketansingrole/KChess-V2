//! Lichess logins Electron's `safeStorage` sealed (Chromium's os_crypt), opened without Electron.
//!
//! The core kept them as base64 text. Under it:
//!
//! - macOS: `v10` + AES-128-CBC; the key is PBKDF2-HMAC-SHA1 (salt `saltysalt`, 1003 rounds) of
//!   the Keychain password "KChess Safe Storage" / "KChess Key".
//! - Linux: `v11` the same with 1 round, of the Secret Service password Chromium's schema keeps
//!   for KChess; `v10` the same with the fixed password `peanuts`.
//! - Windows: `v10` + a 12-byte nonce + AES-256-GCM; the key is `os_crypt.encrypted_key` in the
//!   profile's `Local State`, `DPAPI` + text protected for the user.
//!
//! The initialization vector of the CBC forms is sixteen spaces.

use aes_gcm::aead::{Aead, KeyInit};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use std::path::Path;

use crate::{Result, error};

const SALT: &[u8] = b"saltysalt";
const IV: [u8; 16] = [b' '; 16];
const NONCE: usize = 12;

const UNREADABLE: &str = "This login was saved by an earlier KChess release and cannot be opened. Reconnect your account.";

/// Open `sealed`, base64 of what `safeStorage.encryptString` returned, for the profile at
/// `profile` (Windows keeps the key in its `Local State`).
pub fn open(profile: &Path, sealed: &str) -> Result<String> {
    let data = STANDARD
        .decode(sealed.trim())
        .map_err(|_| error(UNREADABLE))?;
    let plain = platform::open(profile, &data)?;
    String::from_utf8(plain).map_err(|_| error(UNREADABLE))
}

/// The CBC form (macOS, Linux): `version` + ciphertext under PBKDF2(`password`, `rounds`).
pub fn open_cbc(data: &[u8], version: &[u8], password: &[u8], rounds: u32) -> Result<Vec<u8>> {
    let body = data
        .strip_prefix(version)
        .ok_or_else(|| error(UNREADABLE))?;
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, SALT, rounds, &mut key);
    cbc::Decryptor::<aes::Aes128>::new(&key.into(), &IV.into())
        .decrypt_padded_vec_mut::<Pkcs7>(body)
        .map_err(|_| error(UNREADABLE))
}

/// The GCM form (Windows): `v10` + nonce + ciphertext and tag under the 32-byte `key`.
pub fn open_gcm(data: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    let body = data.strip_prefix(b"v10").ok_or_else(|| error(UNREADABLE))?;
    if body.len() < NONCE || key.len() != 32 {
        return Err(error(UNREADABLE));
    }
    let (nonce, ciphertext) = body.split_at(NONCE);
    aes_gcm::Aes256Gcm::new_from_slice(key)
        .map_err(|_| error(UNREADABLE))?
        .decrypt(aes_gcm::Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| error(UNREADABLE))
}

/// `os_crypt.encrypted_key` of a `Local State` file's text: the DPAPI-protected key, without its
/// `DPAPI` mark.
pub fn local_state_key(text: &str) -> Result<Vec<u8>> {
    let state: serde_json::Value = serde_json::from_str(text).map_err(|_| error(UNREADABLE))?;
    let key = state["os_crypt"]["encrypted_key"]
        .as_str()
        .ok_or_else(|| error(UNREADABLE))?;
    let key = STANDARD.decode(key).map_err(|_| error(UNREADABLE))?;
    key.strip_prefix(b"DPAPI")
        .map(<[u8]>::to_vec)
        .ok_or_else(|| error(UNREADABLE))
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    pub fn open(_profile: &Path, data: &[u8]) -> Result<Vec<u8>> {
        let service = format!("{} Safe Storage", crate::APP_NAME);
        let account = format!("{} Key", crate::APP_NAME);
        let password = keyring::Entry::new(&service, &account)
            .and_then(|entry| entry.get_password())
            .map_err(|_| error(UNREADABLE))?;
        open_cbc(data, b"v10", password.as_bytes(), 1003)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use super::*;
    use std::collections::HashMap;

    pub fn open(_profile: &Path, data: &[u8]) -> Result<Vec<u8>> {
        if data.starts_with(b"v11") {
            let password = secret_service_password().ok_or_else(|| error(UNREADABLE))?;
            return open_cbc(data, b"v11", &password, 1);
        }
        open_cbc(data, b"v10", b"peanuts", 1)
    }

    /// The password Chromium's libsecret schemas keep for this app.
    fn secret_service_password() -> Option<Vec<u8>> {
        use secret_service::EncryptionType;
        use secret_service::blocking::SecretService;
        let service = SecretService::connect(EncryptionType::Dh).ok()?;
        let label = format!("{} Safe Storage", crate::APP_NAME);
        for schema in [
            "chrome_libsecret_os_crypt_password_v2",
            "chrome_libsecret_os_crypt_password",
        ] {
            let found = service
                .search_items(HashMap::from([("xdg:schema", schema)]))
                .ok()?;
            for item in found.unlocked.iter().chain(found.locked.iter()) {
                let application = item
                    .get_attributes()
                    .ok()
                    .and_then(|attributes| attributes.get("application").cloned())
                    .unwrap_or_default();
                let ours = application.eq_ignore_ascii_case(crate::APP_NAME)
                    || item.get_label().is_ok_and(|text| text == label);
                if !ours {
                    continue;
                }
                if item.is_locked().unwrap_or(true) && item.unlock().is_err() {
                    continue;
                }
                if let Ok(secret) = item.get_secret() {
                    return Some(secret);
                }
            }
        }
        None
    }
}

#[cfg(windows)]
mod platform {
    use super::*;

    pub fn open(profile: &Path, data: &[u8]) -> Result<Vec<u8>> {
        let text =
            std::fs::read_to_string(profile.join("Local State")).map_err(|_| error(UNREADABLE))?;
        let key = unprotect(&local_state_key(&text)?)?;
        open_gcm(data, &key)
    }

    /// `CryptUnprotectData` for the current user: the one call this needs that Rust cannot make
    /// safely.
    #[allow(unsafe_code)]
    fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData};
        let input = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(data.len()).map_err(|_| error(UNREADABLE))?,
            pbData: data.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: `input` points at `data`, which outlives the call and is only read; every
        // optional argument is null, which the API allows; `output` is a valid blob it fills.
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                &mut output,
            )
        };
        if ok == 0 || output.pbData.is_null() {
            return Err(error(UNREADABLE));
        }
        // SAFETY: on success `pbData` points at `cbData` bytes the API allocated with LocalAlloc.
        let plain =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
        // SAFETY: the buffer came from the API, is freed once, and is not used after this.
        unsafe { LocalFree(output.pbData.cast()) };
        Ok(plain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cbc::cipher::BlockEncryptMut;

    fn seal_cbc(plain: &[u8], version: &[u8], password: &[u8], rounds: u32) -> Vec<u8> {
        let mut key = [0u8; 16];
        pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, SALT, rounds, &mut key);
        let body = cbc::Encryptor::<aes::Aes128>::new(&key.into(), &IV.into())
            .encrypt_padded_vec_mut::<Pkcs7>(plain);
        [version, &body].concat()
    }

    /// Chromium's published derivation: PBKDF2-HMAC-SHA1("peanuts", "saltysalt", 1) as 16 bytes.
    #[test]
    fn derives_chromiums_fixed_linux_key() {
        let mut key = [0u8; 16];
        pbkdf2::pbkdf2_hmac::<sha1::Sha1>(b"peanuts", SALT, 1, &mut key);
        assert_eq!(
            key,
            [
                0xfd, 0x62, 0x1f, 0xe5, 0xa2, 0xb4, 0x02, 0x53, 0x9d, 0xfa, 0x14, 0x7c, 0xa9, 0x27,
                0x27, 0x78
            ]
        );
    }

    #[test]
    fn opens_the_cbc_forms() {
        let sealed = seal_cbc(b"lip_token", b"v10", b"keychain password", 1003);
        assert_eq!(
            open_cbc(&sealed, b"v10", b"keychain password", 1003).unwrap(),
            b"lip_token"
        );
        assert!(open_cbc(&sealed, b"v10", b"another password", 1003).is_err());
        assert!(open_cbc(&sealed, b"v11", b"keychain password", 1003).is_err());
    }

    #[test]
    fn opens_the_gcm_form_with_the_local_state_key() {
        let key = [7u8; 32];
        let nonce = [9u8; NONCE];
        let body = aes_gcm::Aes256Gcm::new_from_slice(&key)
            .unwrap()
            .encrypt(aes_gcm::Nonce::from_slice(&nonce), b"lip_token".as_slice())
            .unwrap();
        let sealed = [b"v10".as_slice(), &nonce, &body].concat();
        assert_eq!(open_gcm(&sealed, &key).unwrap(), b"lip_token");
        assert!(open_gcm(&sealed, &[8u8; 32]).is_err());
        assert!(open_gcm(b"v10short", &key).is_err());

        let state = format!(
            r#"{{"os_crypt":{{"encrypted_key":"{}"}}}}"#,
            STANDARD.encode(b"DPAPIprotected")
        );
        assert_eq!(local_state_key(&state).unwrap(), b"protected");
        assert!(local_state_key("{}").is_err());
    }

    #[test]
    fn says_to_reconnect_when_it_cannot_open() {
        let failure = open(Path::new("/nonexistent"), "not base64!").unwrap_err();
        assert!(failure.0.contains("Reconnect"));
    }
}
