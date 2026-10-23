//! Live end-to-end test against a running voltrus server.
//! Opt-in: only runs when VOLTRUS_E2E_URL / VOLTRUS_E2E_PASSWORD are set.
//!
//!   VOLTRUS_E2E_URL=http://localhost:3100 \
//!   VOLTRUS_E2E_USERNAME=admin \
//!   VOLTRUS_E2E_PASSWORD=... \
//!   cargo test -p voltrus-scada-native --test e2e_live -- --nocapture

use std::time::Duration;

use voltrus_scada_native::runtime::{self, RtEvent, UiCmd};

#[tokio::test]
async fn live_login_sensors_ws_stream() {
    let url = match std::env::var("VOLTRUS_E2E_URL") {
        Ok(u) => u,
        Err(_) => {
            eprintln!("skipping: VOLTRUS_E2E_URL not set");
            return;
        }
    };
    let user = std::env::var("VOLTRUS_E2E_USERNAME").unwrap_or_else(|_| "admin".into());
    let pass = std::env::var("VOLTRUS_E2E_PASSWORD").expect("VOLTRUS_E2E_PASSWORD");

    let (handle, mut rx) = runtime::start();

    // Probe: a deliberately bad login must produce a LoginResult event back.
    // If this hangs, the UI→runtime→UI channel wiring is broken.
    handle.send(UiCmd::Login {
        url: url.clone(),
        username: "no-such-user-probe".into(),
        password: "wrong".into(),
    });
    let mut probe_done = false;
    for _ in 0..30 {
        match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
            Ok(Some(RtEvent::LoginResult(_))) => {
                probe_done = true;
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) => panic!("event channel closed"),
            Err(_) => {}
        }
    }
    assert!(
        probe_done,
        "probe login produced no event — channel wiring broken"
    );

    handle.send(UiCmd::Login {
        url,
        username: user,
        password: pass,
    });

    let mut login_ok = false;
    let mut sensors: Option<usize> = None;
    let mut live = false;
    let mut tag_updates = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);

    loop {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        if login_ok && sensors.is_some() && live && tag_updates > 0 {
            break;
        }
        match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
            Ok(Some(ev)) => match ev {
                RtEvent::LoginResult(Ok(_)) => login_ok = true,
                RtEvent::LoginResult(Err(e)) => panic!("login failed: {e}"),
                RtEvent::Sensors(list) => sensors = Some(list.len()),
                RtEvent::Conn(ref s) => {
                    eprintln!("conn: {s}");
                    if s == "live" {
                        live = true;
                    }
                }
                RtEvent::Tags(updates) => tag_updates += updates.len(),
                _ => {}
            },
            Ok(None) => break, // channel closed
            Err(_) => {}       // timeout tick
        }
    }

    assert!(login_ok, "expected successful login");
    let n = sensors.expect("expected data sources");
    assert!(n > 0, "expected at least one data source, got 0");
    assert!(live, "expected WS to reach 'live' state");
    println!("sensors: {n}, tag_updates: {tag_updates}");
    if tag_updates == 0 {
        eprintln!("note: no live tag updates — are protocol simulators running?");
    }

    handle.send(UiCmd::Shutdown);
}
