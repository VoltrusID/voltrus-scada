//! GET /api/v1/devices/{id}/traffic against a real poller and a Modbus
//! server on loopback.

use axum::http::StatusCode;
use serde_json::{Value, json};
use tokio_modbus::prelude::*;
use tokio_modbus::server::Service;
use tokio_modbus::server::tcp::{Server, accept_tcp_connection};

use super::helpers::*;

#[derive(Clone)]
struct Meter;

impl Service for Meter {
    type Request = Request<'static>;
    type Response = Response;
    type Exception = ExceptionCode;
    type Future = std::future::Ready<Result<Response, ExceptionCode>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        std::future::ready(match req {
            // Holding registers 0..10 exist; the input table does not.
            Request::ReadHoldingRegisters(a, n) if a + n <= 10 => Ok(
                Response::ReadHoldingRegisters((a..a + n).map(|v| v * 100).collect()),
            ),
            _ => Err(ExceptionCode::IllegalDataAddress),
        })
    }
}

async fn meter() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let on_connected =
            |stream, addr| async move { accept_tcp_connection(stream, addr, |_| Ok(Some(Meter))) };
        let _ = Server::new(listener).serve(&on_connected, |_| {}).await;
    });
    port
}

async fn traffic(app: &axum::Router, cookie: &str, query: &str) -> (StatusCode, Value) {
    send_request(
        app,
        "GET",
        &format!("/api/v1/devices/m1/traffic{query}"),
        None,
        Some(cookie),
        None,
    )
    .await
}

#[tokio::test]
async fn shows_decoded_paired_frames_with_their_tags() {
    let state = seeded_admin_app_state().await;
    let app = crate::api::create_router(state.clone());
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let port = meter().await;
    let (s, b) = send_request(&app, "POST", "/api/v1/modbus-devices", None, Some(&cookie), Some(json!({
        "id": "m1", "name": "Meter", "ip": "127.0.0.1", "port": port, "unit_id": 1, "interval_ms": 100,
        "registers": [
            {"name": "volts", "address": 40001, "register_type": "uint16"},
            {"name": "amps", "address": 40002, "register_type": "uint16"},
            {"name": "missing", "address": 30001, "register_type": "uint16", "register_area": "input"}
        ]
    }))).await;
    assert!(s.is_success(), "{b}");
    let (s, b) = send_request(
        &app,
        "POST",
        "/api/v1/data-sources",
        None,
        Some(&cookie),
        Some(json!({
            "key": "volts", "name": "Voltage", "source_type": "modbus", "device_id": "m1"
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");

    // The first look starts the capture.
    let (s, first) = traffic(&app, &cookie, "").await;
    assert_eq!(s, StatusCode::OK, "{first}");
    assert_eq!(first["capturing"], true);

    let mut body = Value::Null;
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let (_, b) = traffic(&app, &cookie, "?errors=true").await;
        if b["frames"].as_array().is_some_and(|f| !f.is_empty()) {
            body = b;
            break;
        }
    }
    let frames = body["frames"]
        .as_array()
        .expect("an exception frame within 5 s");
    let exc = &frames[0];
    assert_eq!(exc["dir"], "rx");
    assert_eq!(exc["function"], 4);
    assert_eq!(exc["error"]["code"], 2);
    assert!(
        exc["error"]["explanation"]
            .as_str()
            .unwrap()
            .contains("wire address 0 (30001)")
    );
    assert!(exc["latency_us"].is_u64() && exc["pair_id"].is_u64());

    let (_, reads) = traffic(&app, &cookie, "?function=3&space=holding").await;
    let resp = reads["frames"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["dir"] == "rx")
        .expect("a holding register response");
    assert_eq!(
        resp["summary"],
        "Read Holding Registers response, 2 registers"
    );
    let refs = resp["tag_refs"].as_array().unwrap();
    assert_eq!(refs[0]["key"], "volts");
    assert_eq!(refs[0]["tag_name"], "Voltage");
    assert_eq!(
        (refs[0]["offset"].as_u64(), refs[0]["len"].as_u64()),
        (Some(9), Some(2))
    );
    assert_eq!(refs[1]["key"], "amps");
    assert!(refs[1].get("tag_id").is_none(), "no tag for amps");
    assert_eq!(resp["raw"].as_str().unwrap().len() % 2, 0);
    assert_eq!(resp["fields"][0]["children"][0]["id"], "mbtcp.trans_id");

    // Summary only, newer than a seq, bad filters.
    let (_, lean) = traffic(&app, &cookie, "?detail=false&limit=1").await;
    let one = &lean["frames"][0];
    assert!(one.get("fields").is_none() && one.get("raw").is_none());
    let last = one["seq"].as_u64().unwrap();
    let (_, newer) = traffic(&app, &cookie, &format!("?after={last}")).await;
    assert!(
        newer["frames"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["seq"].as_u64().unwrap() > last)
    );
    let (s, _) = traffic(&app, &cookie, "?space=nope").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = send_request(
        &app,
        "GET",
        "/api/v1/devices/nope/traffic",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    state.poller_manager.stop("m1").await;
}

#[tokio::test]
async fn operators_read_traffic_and_viewers_cannot() {
    let state = seeded_admin_app_state().await;
    let app = crate::api::create_router(state.clone());
    for (role, expected) in [
        ("viewer", StatusCode::FORBIDDEN),
        // An operator troubleshooting a device: the device exists only in the
        // other test, so getting past the role check means a 404 here.
        ("operator", StatusCode::NOT_FOUND),
    ] {
        let hash = crate::api::auth::hash_password("pw-1234567");
        state
            .storage
            .create_user(role, &hash, "", role)
            .await
            .unwrap();
        let cookie = format!(
            "voltrus_token={}",
            login_cookie(&app, role, "pw-1234567").await
        );
        let (s, _) = traffic(&app, &cookie, "").await;
        assert_eq!(s, expected, "{role}");
    }
}
