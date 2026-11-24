//! The one place KChess starts building an HTTP client: the core's Lichess, Chess.com and
//! download clients, and the app's updater and Lichess pictures.
//!
//! reqwest's TLS is rustls with the operating system's certificate store
//! (`rustls-platform-verifier`). It takes its cryptography from the process-wide rustls provider,
//! which [`client`] sets to ring, the library KChess already builds for its update patches,
//! rather than reqwest's own default, aws-lc-rs, a second one compiled from C. A client built
//! without going through here panics when it first needs TLS, so every builder starts here
//! (`tooling/xtask/src/boundaries.rs` refuses `reqwest::Client::builder()` and
//! `reqwest::Client::new()` elsewhere).

use std::sync::Once;

/// A client builder, after making ring the process's rustls provider.
pub fn client() -> reqwest::ClientBuilder {
    static PROVIDER: Once = Once::new();
    PROVIDER.call_once(|| {
        // An error means a provider was already installed, which reqwest then uses: nothing to do.
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();
    });
    reqwest::Client::builder()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_builds_with_tls_and_ring_is_the_provider() {
        let built = client().https_only(true).build();
        assert!(built.is_ok(), "{:?}", built.err());
        let provider = rustls::crypto::CryptoProvider::get_default().expect("a provider");
        let ring = rustls::crypto::ring::default_provider();
        assert_eq!(
            provider.cipher_suites.len(),
            ring.cipher_suites.len(),
            "ring's cipher suites"
        );
        // A second call reuses the provider.
        assert!(client().build().is_ok());
    }
}
