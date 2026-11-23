//! Which certificates the console trusts for `https://` and `wss://`.
//!
//! Always the operating system's store (a plant's own CA installed there
//! works) and the bundled webpki roots; besides those, the certificates of
//! an optional `ca_file` in the console's config (PEM), for a server whose
//! CA is not installed on the computer. Verification is never turned off.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use rustls_pki_types::CertificateDer;
use rustls_pki_types::pem::PemObject;

/// The `ca_file`'s certificates, set once at start.
static EXTRA: OnceLock<Vec<CertificateDer<'static>>> = OnceLock::new();

/// The certificates in a PEM file (one or more `CERTIFICATE` blocks).
pub fn read_pem(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let certs = CertificateDer::pem_file_iter(path)
        .and_then(Iterator::collect::<Result<Vec<_>, _>>)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if certs.is_empty() {
        return Err(format!("{} holds no certificate", path.display()));
    }
    Ok(certs)
}

/// Trust the certificates of `path` too, from now on (call before the
/// first request). The first call wins.
pub fn add_ca_file(path: &Path) -> Result<usize, String> {
    let certs = read_pem(path)?;
    let count = certs.len();
    let _ = EXTRA.set(certs);
    Ok(count)
}

fn extra() -> &'static [CertificateDer<'static>] {
    EXTRA.get().map_or(&[], Vec::as_slice)
}

/// An HTTP client with the console's trust and `timeout` per request.
pub fn http_client(timeout: std::time::Duration) -> reqwest::Client {
    let mut builder = reqwest::Client::builder().timeout(timeout);
    for cert in extra() {
        match reqwest::Certificate::from_der(cert) {
            Ok(cert) => builder = builder.add_root_certificate(cert),
            Err(error) => eprintln!("voltrus-native: ca_file certificate skipped: {error}"),
        }
    }
    builder.build().expect("reqwest client")
}

/// The WebSocket's TLS connector: `None` (tokio-tungstenite's own, with
/// the system's and the webpki roots) unless a `ca_file` adds roots.
pub fn ws_connector() -> Option<tokio_tungstenite::Connector> {
    let extra = extra();
    if extra.is_empty() {
        return None;
    }
    Some(tokio_tungstenite::Connector::Rustls(Arc::new(
        client_config(extra),
    )))
}

/// A rustls configuration trusting the system, the webpki roots and `extra`.
fn client_config(extra: &[CertificateDer<'static>]) -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    roots.add_parsable_certificates(native.certs);
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots.add_parsable_certificates(extra.iter().cloned());
    rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("rustls protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A self-signed certificate (CN=voltrus-test), PEM.
    const CERT: &str = "-----BEGIN CERTIFICATE-----
MIIBgzCCASmgAwIBAgIUdKCK0PxwWVWSKUMzMBuwL6JqFHgwCgYIKoZIzj0EAwIw
FzEVMBMGA1UEAwwMdm9sdHJ1cy10ZXN0MB4XDTI2MTEyMzE4MDcwNFoXDTM2MTEy
MDE4MDcwNFowFzEVMBMGA1UEAwwMdm9sdHJ1cy10ZXN0MFkwEwYHKoZIzj0CAQYI
KoZIzj0DAQcDQgAEYeeq70jPj8UY/udKiWnB3x9O2o3WiKlX9VnSL+3cG4DLpej3
xd5gKSAcoIJzK3PN3+RX030jbcOi+0p0ueMPf6NTMFEwHQYDVR0OBBYEFA1BVuM3
E10bqvorlBEUwwviUkuhMB8GA1UdIwQYMBaAFA1BVuM3E10bqvorlBEUwwviUkuh
MA8GA1UdEwEB/wQFMAMBAf8wCgYIKoZIzj0EAwIDSAAwRQIhANykpCyt4KNRWcqi
MV4s6WUL21zJUHavNnmgFkq+nqqJAiAWXrBjxyWRVokxt20e8OdQnEyqqA36N6QL
JtHI5624zA==
-----END CERTIFICATE-----
";

    #[test]
    fn a_ca_file_needs_a_certificate() {
        let dir = std::env::temp_dir().join(format!("voltrus-tls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let empty = dir.join("empty.pem");
        std::fs::write(&empty, "not a certificate\n").unwrap();
        assert!(read_pem(&empty).unwrap_err().contains("no certificate"));
        assert!(read_pem(&dir.join("missing.pem")).is_err());
        let one = dir.join("one.pem");
        std::fs::write(&one, CERT).unwrap();
        let certs = read_pem(&one).unwrap();
        assert_eq!(certs.len(), 1);
        assert!(reqwest::Certificate::from_der(&certs[0]).is_ok());
        // Added to the system's and the webpki roots.
        let mut roots = rustls::RootCertStore::empty();
        assert_eq!(roots.add_parsable_certificates(certs.clone()), (1, 0));
        let _ = client_config(&certs);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_ca_file_the_default_connectors_serve() {
        if EXTRA.get().is_none() {
            assert!(ws_connector().is_none());
        }
        let _ = http_client(std::time::Duration::from_secs(1));
    }
}
