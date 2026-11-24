//! The outbound client must not use a proxy from the environment: a proxy
//! resolves the host itself, past the address rules. A test binary of its
//! own, because it sets process-wide environment variables.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[tokio::test]
async fn the_outbound_client_ignores_proxy_environment_variables() {
    let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        while let Ok((socket, _)) = proxy.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            drop(socket);
        }
    });
    for name in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
    ] {
        // SAFETY: this test binary has one test and sets the variables before
        // any other thread reads the environment.
        unsafe { std::env::set_var(name, &proxy_url) };
    }
    // SAFETY: as above.
    unsafe {
        std::env::remove_var("NO_PROXY");
        std::env::remove_var("no_proxy");
    }

    // A documentation address nobody answers; only where the client
    // connects matters.
    let target = "http://203.0.113.1:9/hook";
    voltrus::security::outbound::check_url(target).unwrap();

    // A default client goes to the proxy (so the setup is real) ...
    let _ = reqwest::Client::new()
        .post(target)
        .timeout(Duration::from_secs(2))
        .send()
        .await;
    let via_proxy = hits.load(Ordering::SeqCst);
    assert!(via_proxy >= 1, "a default client uses the proxy");

    // ... the outbound client does not.
    let _ = voltrus::security::outbound::client(Some(Duration::from_millis(500)))
        .post(target)
        .send()
        .await;
    assert_eq!(
        hits.load(Ordering::SeqCst),
        via_proxy,
        "the outbound client used the proxy"
    );
}
