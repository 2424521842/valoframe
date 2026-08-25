//! Shared HTTPS client setup.
//!
//! `reqwest` is built with the `rustls-no-provider` feature, so `Client::builder().build()`
//! panics unless a rustls crypto provider has been installed for the process first. Every
//! outbound request path must go through [`ensure_crypto_provider`] before building a client.

use std::sync::Once;

static INSTALL_CRYPTO_PROVIDER: Once = Once::new();

/// Installs the process-wide rustls crypto provider exactly once.
///
/// Safe to call from any thread and from every request path. A pre-existing provider (installed
/// by another component) is left alone rather than treated as an error.
pub(crate) fn ensure_crypto_provider() {
    INSTALL_CRYPTO_PROVIDER.call_once(|| {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crypto_provider_is_idempotent_and_lets_a_client_build() {
        ensure_crypto_provider();
        ensure_crypto_provider();
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());

        // Building a client is the operation that would otherwise panic.
        reqwest::Client::builder()
            .build()
            .expect("client should build once a provider is installed");
    }
}
