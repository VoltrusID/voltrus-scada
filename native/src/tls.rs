//! Which certificates the console trusts for `https://` and `wss://`.
//!
//! Always the operating system's store (a plant's own CA installed there
//! works) and the bundled webpki roots; besides those, the certificates of
//! an optional `ca_file` in the console's config (PEM), for a server whose
//! CA is not installed on the computer. Verification is never turned off.

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use rustls_pki_types::CertificateDer;
use rustls_pki_types::pem::PemObject;

/// The `ca_file`'s certificates, set once at start.
static EXTRA: OnceLock<Vec<CertificateDer<'static>>> = OnceLock::new();

/// What went wrong with the `ca_file`, for the sign-in card: each once.
static PROBLEMS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Remember a problem with the console's trust settings (shown on the
/// sign-in card and printed).
pub fn note_problem(problem: String) {
    if let Ok(mut problems) = PROBLEMS.lock()
        && !problems.contains(&problem)
    {
        eprintln!("voltrus-native: {problem}");
        problems.push(problem);
    }
}

/// The problems with the console's trust settings so far.
pub fn problems() -> Vec<String> {
    PROBLEMS
        .lock()
        .map(|problems| problems.clone())
        .unwrap_or_default()
}

/// The valid certificates in a PEM file (one or more `CERTIFICATE`
/// blocks), and why any others were left out. A file without one valid
/// certificate is an error.
pub fn read_pem(path: &Path) -> Result<(Vec<CertificateDer<'static>>, Vec<String>), String> {
    let certs = CertificateDer::pem_file_iter(path)
        .and_then(Iterator::collect::<Result<Vec<_>, _>>)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if certs.is_empty() {
        return Err(format!("{} holds no certificate", path.display()));
    }
    let mut valid = Vec::new();
    let mut skipped = Vec::new();
    for (index, cert) in certs.into_iter().enumerate() {
        // What rustls itself accepts as a trust anchor.
        match rustls::RootCertStore::empty().add(cert.clone()) {
            Ok(()) => valid.push(cert),
            Err(error) => skipped.push(format!(
                "certificate {} of {} left out: {error}",
                index + 1,
                path.display()
            )),
        }
    }
    if valid.is_empty() {
        return Err(format!(
            "{} holds no valid certificate ({})",
            path.display(),
            skipped.join("; ")
        ));
    }
    Ok((valid, skipped))
}

/// Trust the valid certificates of `path` too, from now on (call before
/// the first request). The first call wins. Problems are also kept for
/// [`problems`].
pub fn add_ca_file(path: &Path) -> Result<usize, String> {
    let (certs, skipped) = read_pem(path).inspect_err(|error| {
        note_problem(format!("ca_file not used: {error}"));
    })?;
    for problem in skipped {
        note_problem(problem);
    }
    let count = certs.len();
    let _ = EXTRA.set(certs);
    Ok(count)
}

fn extra() -> &'static [CertificateDer<'static>] {
    EXTRA.get().map_or(&[], Vec::as_slice)
}

/// An HTTP client with the console's trust and `timeout` per request.
/// Never panics on the `ca_file`: if the client cannot be built with its
/// certificates, it is built without them and the problem is noted.
pub fn http_client(timeout: std::time::Duration) -> reqwest::Client {
    http_client_with(timeout, extra())
}

fn http_client_with(
    timeout: std::time::Duration,
    extra: &[CertificateDer<'static>],
) -> reqwest::Client {
    let mut builder = reqwest::Client::builder().timeout(timeout);
    for cert in extra {
        match reqwest::Certificate::from_der(cert) {
            Ok(cert) => builder = builder.add_root_certificate(cert),
            Err(error) => note_problem(format!("ca_file certificate left out: {error}")),
        }
    }
    match builder.build() {
        Ok(client) => client,
        Err(error) => {
            note_problem(format!(
                "ca_file not used: the HTTP client refused it ({error})"
            ));
            reqwest::Client::builder()
                .timeout(timeout)
                .build()
                // Without extra roots this is reqwest's ordinary client.
                .unwrap_or_else(|_| reqwest::Client::new())
        }
    }
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
    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ));
    match builder.with_safe_default_protocol_versions() {
        Ok(builder) => builder.with_root_certificates(roots).with_no_client_auth(),
        // ring supports the default versions; should it not, rustls's own
        // default configuration serves.
        Err(_) => rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    }
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
        let (certs, skipped) = read_pem(&one).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(certs.len(), 1);
        assert!(reqwest::Certificate::from_der(&certs[0]).is_ok());
        // Added to the system's and the webpki roots.
        let mut roots = rustls::RootCertStore::empty();
        assert_eq!(roots.add_parsable_certificates(certs.clone()), (1, 0));
        let _ = client_config(&certs);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A PEM block whose content is not a certificate.
    const CORRUPT: &str =
        "-----BEGIN CERTIFICATE-----\nAAECAwQFBgcICQ==\n-----END CERTIFICATE-----\n";

    #[test]
    fn corrupt_certificates_are_left_out_and_never_panic() {
        let dir = std::env::temp_dir().join(format!("voltrus-tls-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let only_bad = dir.join("bad.pem");
        std::fs::write(&only_bad, CORRUPT).unwrap();
        let error = read_pem(&only_bad).unwrap_err();
        assert!(error.contains("no valid certificate"), "{error}");
        let mixed = dir.join("mixed.pem");
        std::fs::write(&mixed, format!("{CORRUPT}{CERT}")).unwrap();
        let (certs, skipped) = read_pem(&mixed).unwrap();
        assert_eq!(certs.len(), 1);
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].contains("certificate 1"), "{}", skipped[0]);
        // Handed straight to the clients, a corrupt one costs a noted
        // problem, not a panic.
        let bad = CertificateDer::from(vec![0_u8, 1, 2, 3]);
        let _ = http_client_with(
            std::time::Duration::from_secs(1),
            std::slice::from_ref(&bad),
        );
        let _ = client_config(&[bad]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn problems_are_kept_once() {
        note_problem("test problem".into());
        note_problem("test problem".into());
        assert_eq!(
            problems().iter().filter(|p| *p == "test problem").count(),
            1
        );
    }

    #[test]
    fn without_a_ca_file_the_default_connectors_serve() {
        if EXTRA.get().is_none() {
            assert!(ws_connector().is_none());
        }
        let _ = http_client(std::time::Duration::from_secs(1));
    }
}
