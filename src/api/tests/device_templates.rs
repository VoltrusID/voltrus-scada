//! SCADA-5: the shared device template library: built-ins, save from a device,
//! instantiate, import and export, and the public catalog (against a local server).

use super::*;
use crate::device_templates::catalog::{SETTING_CATALOG_URL, sha256_hex};
use axum::http::Uri;
use axum::response::IntoResponse;
use http::StatusCode;
use serde_json::{Value, json};
use std::collections::HashMap;

const BASE: &str = "/api/v1/device-templates";

async fn admin_app() -> (Router, String, Storage) {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    (app, cookie, storage)
}

async fn viewer_cookie(app: &Router, storage: &Storage) -> String {
    let hash = crate::api::auth::hash_password("viewer-pass");
    storage
        .create_user("viewer", &hash, "", "viewer")
        .await
        .unwrap();
    format!(
        "voltrus_token={}",
        login_cookie(app, "viewer", "viewer-pass").await
    )
}

async fn call(
    app: &Router,
    cookie: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    send_request(app, method, uri, None, Some(cookie), body).await
}

pub(super) fn modbus_template(id: &str, version: &str) -> Value {
    json!({
        "format": "voltrus-device-template/1",
        "template_id": id,
        "manufacturer": "Acme",
        "model": "Meter 9",
        "protocol": "modbus",
        "description": "Test meter",
        "version": version,
        "verified": true,
        "verified_notes": "Firmware 2.1 on site",
        "quirks": ["Word order is swapped on firmware before 2.0"],
        "port": 1502,
        "unit_id": 7,
        "endianness": "swapped",
        "registers": [
            {"name": "voltage", "offset": 0, "register_type": "float32", "scale": 1.0, "unit": "V", "description": "L1-N", "writable": false},
            {"name": "current", "offset": 2, "register_type": "int16", "register_area": "input", "scale": 0.01}
        ]
    })
}

fn ids(list: &Value) -> Vec<String> {
    list["templates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["template_id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn builtins_list_the_sim_profiles_and_cannot_be_deleted() {
    let (app, cookie, _) = admin_app().await;

    let (status, list) = call(&app, &cookie, "GET", BASE, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let found = ids(&list);
    for id in [
        "sma-stp-60-us-10",
        "schneider-electric-pm5560",
        "endress-hauser-promag-10w-dn50",
        "abb-azl402",
        "emerson-daniel-1000",
        "raritan-px3-5902v",
        "trane-tracer-sc",
        "johnson-controls-facility-explorer-fec",
    ] {
        assert!(
            found.contains(&id.to_string()),
            "{id} missing from {found:?}"
        );
    }
    let pm = list["templates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["template_id"] == "schneider-electric-pm5560")
        .unwrap();
    assert_eq!(pm["source"], "builtin");
    assert_eq!(pm["verified"], false);
    assert_eq!(pm["read_only"], true);
    assert_eq!(pm["protocol"], "modbus");

    let (status, body) = call(
        &app,
        &cookie,
        "DELETE",
        &format!("{BASE}/schneider-electric-pm5560"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, _) = call(
        &app,
        &cookie,
        "DELETE",
        &format!("{BASE}/schneider-electric-pm5560?source=builtin&version=1.0.0"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Still there.
    let (status, t) = call(
        &app,
        &cookie,
        "GET",
        &format!("{BASE}/schneider-electric-pm5560"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(t["read_only"], true);
    assert_eq!(t["format"], "voltrus-device-template/1");
    assert!(t["registers"].as_array().unwrap().len() > 10);

    let (status, _) = call(
        &app,
        &cookie,
        "DELETE",
        &format!("{BASE}/no-such-template"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn search_filters_by_text_manufacturer_protocol_and_source() {
    let (app, cookie, _) = admin_app().await;
    let (status, body) = call(
        &app,
        &cookie,
        "POST",
        BASE,
        Some(modbus_template("acme-meter-9", "1.0.0")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (_, all) = call(&app, &cookie, "GET", BASE, None).await;
    assert_eq!(all["count"], 9);

    let (_, q) = call(&app, &cookie, "GET", &format!("{BASE}?q=pm5560"), None).await;
    assert_eq!(ids(&q), vec!["schneider-electric-pm5560"]);

    let (_, q) = call(&app, &cookie, "GET", &format!("{BASE}?q=METER%209"), None).await;
    assert_eq!(ids(&q), vec!["acme-meter-9"]);

    let (_, m) = call(
        &app,
        &cookie,
        "GET",
        &format!("{BASE}?manufacturer=trane"),
        None,
    )
    .await;
    assert_eq!(ids(&m), vec!["trane-tracer-sc"]);

    let (_, p) = call(
        &app,
        &cookie,
        "GET",
        &format!("{BASE}?protocol=bacnet"),
        None,
    )
    .await;
    assert_eq!(p["count"], 2);
    let (_, p) = call(
        &app,
        &cookie,
        "GET",
        &format!("{BASE}?protocol=modbus&source=local"),
        None,
    )
    .await;
    assert_eq!(ids(&p), vec!["acme-meter-9"]);
    let (_, p) = call(
        &app,
        &cookie,
        "GET",
        &format!("{BASE}?source=builtin&protocol=modbus"),
        None,
    )
    .await;
    assert_eq!(p["count"], 6);

    let (_, none) = call(&app, &cookie, "GET", &format!("{BASE}?q=zzz-nothing"), None).await;
    assert_eq!(none["count"], 0);
}

async fn register_view(
    app: &Router,
    cookie: &str,
    device_id: &str,
) -> Vec<(String, String, String, f64)> {
    let (status, dev) = call(
        app,
        cookie,
        "GET",
        &format!("/api/v1/modbus-devices/{device_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{dev}");
    dev["registers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["name"].as_str().unwrap().to_string(),
                r["register_type"].as_str().unwrap().to_string(),
                r["register_area"].as_str().unwrap().to_string(),
                r["scale"].as_f64().unwrap(),
            )
        })
        .collect()
}

#[tokio::test]
async fn modbus_device_saved_as_template_instantiates_with_identical_registers() {
    let (app, cookie, _) = admin_app().await;

    // A legacy 5-digit reference, an explicit offset and a coil, as the device routes accept them.
    let (status, body) = call(
        &app,
        &cookie,
        "POST",
        "/api/v1/modbus-devices",
        Some(json!({
            "id": "meter-a", "name": "Meter A", "ip": "127.0.0.1", "port": 1, "unit_id": 5,
            "interval_ms": 2500, "endianness": "little",
            "registers": [
                {"name": "volts", "address": 40001, "register_type": "float32"},
                {"name": "amps", "offset": 100, "register_type": "int16", "register_area": "input", "scale": 0.1},
                {"name": "trip", "address": 30003, "register_type": "uint16", "register_area": "input", "scale": 2.0},
                {"name": "ok", "offset": 9, "register_type": "uint16", "register_area": "coil"}
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (status, saved) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/from-device/meter-a"),
        Some(json!({
            "manufacturer": "Acme", "model": "Meter A", "description": "Saved from a site",
            "verified": true, "verified_notes": "Firmware 1.4", "quirks": ["low word first"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    assert_eq!(saved["template_id"], "acme-meter-a");
    assert_eq!(saved["version"], "1.0.0");
    assert_eq!(saved["source"], "local");

    let (_, t) = call(&app, &cookie, "GET", &format!("{BASE}/acme-meter-a"), None).await;
    assert_eq!(t["protocol"], "modbus");
    assert_eq!(t["port"], 1);
    assert_eq!(t["unit_id"], 5);
    assert_eq!(t["interval_ms"], 2500);
    assert_eq!(t["endianness"], "little");
    assert_eq!(t["verified"], true);
    assert_eq!(t["license"], "CC-BY-4.0");
    let offsets: Vec<u64> = t["registers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["offset"].as_u64().unwrap())
        .collect();
    assert_eq!(offsets, vec![0, 100, 2, 9]);
    assert!(t.get("ip").is_none());

    // Saving the same id and version again is a conflict.
    let (status, _) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/from-device/meter-a"),
        Some(json!({"manufacturer": "Acme", "model": "Meter A"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, made) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/acme-meter-a/instantiate"),
        Some(json!({"name": "Meter B", "ip": "127.0.0.1", "port": 1, "id": "meter-b"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(made["id"], "meter-b");
    assert_eq!(made["item_count"], 4);

    assert_eq!(
        register_view(&app, &cookie, "meter-a").await,
        register_view(&app, &cookie, "meter-b").await
    );
    let (_, b) = call(&app, &cookie, "GET", "/api/v1/modbus-devices/meter-b", None).await;
    assert_eq!(b["device"]["unit_id"], 5);
    assert_eq!(b["device"]["interval_ms"], 2500);
    assert_eq!(b["device"]["endianness"], "little");
    assert_eq!(b["device"]["name"], "Meter B");

    // Saving the copy back gives the same registers, wire address for wire address.
    let (status, _) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/from-device/meter-b"),
        Some(
            json!({"manufacturer": "Acme", "model": "Meter A", "template_id": "acme-meter-b-copy"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, copy) = call(
        &app,
        &cookie,
        "GET",
        &format!("{BASE}/acme-meter-b-copy"),
        None,
    )
    .await;
    assert_eq!(copy["registers"], t["registers"]);

    // Overrides on instantiate, and a taken id.
    let (status, made) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/acme-meter-a/instantiate"),
        Some(json!({"name": "Meter C", "ip": "127.0.0.1", "port": 1, "unit_id": 9, "endianness": "big", "interval_ms": 500})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(made["id"], "meter-c");
    let (_, c) = call(&app, &cookie, "GET", "/api/v1/modbus-devices/meter-c", None).await;
    assert_eq!(c["device"]["unit_id"], 9);
    assert_eq!(c["device"]["endianness"], "big");
    assert_eq!(c["device"]["interval_ms"], 500);

    let (status, _) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/acme-meter-a/instantiate"),
        Some(json!({"name": "Again", "ip": "127.0.0.1", "id": "meter-b"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    for bad in [
        json!({"name": "", "ip": "127.0.0.1"}),
        json!({"name": "x", "ip": ""}),
        json!({"name": "x", "ip": "1.2.3.4", "port": 0}),
        json!({"name": "x", "ip": "1.2.3.4", "endianness": "sideways"}),
        json!({"name": "x", "ip": "1.2.3.4", "device_instance": 4}),
    ] {
        let (status, body) = call(
            &app,
            &cookie,
            "POST",
            &format!("{BASE}/acme-meter-a/instantiate"),
            Some(bad.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }
}

#[tokio::test]
async fn builtin_pm5560_instantiates_and_saves_back() {
    let (app, cookie, _) = admin_app().await;
    let (_, tmpl) = call(
        &app,
        &cookie,
        "GET",
        &format!("{BASE}/schneider-electric-pm5560"),
        None,
    )
    .await;
    let n = tmpl["registers"].as_array().unwrap().len();

    let (status, made) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/schneider-electric-pm5560/instantiate"),
        Some(json!({"name": "Grid meter", "ip": "127.0.0.1", "port": 1, "unit_id": 3})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(made["id"], "grid-meter");
    let (_, dev) = call(
        &app,
        &cookie,
        "GET",
        "/api/v1/modbus-devices/grid-meter",
        None,
    )
    .await;
    assert_eq!(dev["registers"].as_array().unwrap().len(), n);
    assert_eq!(dev["device"]["unit_id"], 3);
    assert_eq!(dev["registers"][0]["name"], tmpl["registers"][0]["name"]);

    let (status, saved) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/from-device/grid-meter"),
        Some(json!({"manufacturer": "Schneider Electric", "model": "PM5560", "version": "1.0.1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    // Same slug as the built-in, a different source and version: both are listed, the newest wins.
    let (_, listed) = call(&app, &cookie, "GET", &format!("{BASE}?q=pm5560"), None).await;
    assert_eq!(listed["count"], 2);
    let (_, newest) = call(
        &app,
        &cookie,
        "GET",
        &format!("{BASE}/schneider-electric-pm5560"),
        None,
    )
    .await;
    assert_eq!(newest["source"], "local");
    assert_eq!(newest["version"], "1.0.1");
    assert_eq!(newest["registers"].as_array().unwrap().len(), n);
    assert_eq!(
        newest["registers"][3]["offset"],
        tmpl["registers"][3]["offset"]
    );

    // Export is a download of the same JSON.
    let resp = {
        use tower::ServiceExt;
        app.clone()
            .oneshot(
                http::Request::builder()
                    .uri(format!(
                        "{BASE}/schneider-electric-pm5560/export?source=local"
                    ))
                    .header("cookie", &cookie)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    };
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()["content-disposition"],
        "attachment; filename=\"schneider-electric-pm5560-1.0.1.json\""
    );
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let exported: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(exported["template_id"], "schneider-electric-pm5560");
    assert_eq!(exported["registers"], newest["registers"]);

    // The exported file imports on another server as local, with a new version.
    let (other, other_cookie, _) = admin_app().await;
    let mut file = exported.clone();
    file["version"] = json!("2.0.0");
    file["source"] = json!("builtin");
    let (status, imported) = call(&other, &other_cookie, "POST", BASE, Some(file)).await;
    assert_eq!(status, StatusCode::CREATED, "{imported}");
    assert_eq!(imported["source"], "local");
}

#[tokio::test]
async fn bacnet_device_round_trips_through_a_template() {
    let (app, cookie, _) = admin_app().await;
    let (status, made) = call(
        &app,
        &cookie,
        "POST",
        "/api/v1/bacnet/devices",
        Some(json!({"name": "AHU 1", "ip": "10.0.0.5", "port": 47809, "device_instance": 1001, "interval_ms": 3000, "cov_enabled": true, "cov_lifetime": 120})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let id = made["id"].as_i64().unwrap();
    for (ty, inst, name, cov) in [
        ("analogInput", 1, "SupplyTemp", 0.5),
        ("binaryOutput", 3, "Fan", 0.0),
        ("multiStateValue", 7, "Mode", 0.0),
    ] {
        let (status, body) = call(
            &app,
            &cookie,
            "POST",
            &format!("/api/v1/bacnet/devices/{id}/objects"),
            Some(json!({"object_type": ty, "object_instance": inst, "name": name, "display_name": format!("{name} shown"), "cov_increment": cov})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    let (status, saved) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/from-device/{id}"),
        Some(json!({"manufacturer": "Trane", "model": "Custom AHU", "protocol": "bacnet"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let (_, t) = call(
        &app,
        &cookie,
        "GET",
        &format!("{BASE}/trane-custom-ahu"),
        None,
    )
    .await;
    assert_eq!(t["protocol"], "bacnet");
    assert_eq!(t["port"], 47809);
    assert_eq!(t["cov_enabled"], true);
    assert_eq!(t["objects"].as_array().unwrap().len(), 3);
    assert!(t.get("registers").is_none());

    let (status, made2) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/trane-custom-ahu/instantiate"),
        Some(json!({"name": "AHU 2", "ip": "10.0.0.6", "device_instance": 1002})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{made2}");
    let id2 = made2["id"].as_i64().unwrap();
    assert_ne!(id, id2);
    let strip = |v: &Value| -> Vec<Value> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|o| {
                json!({
                    "t": o["object_type"], "i": o["object_instance"], "p": o["property"],
                    "n": o["name"], "d": o["display_name"], "c": o["cov_increment"]
                })
            })
            .collect()
    };
    let (_, a) = call(
        &app,
        &cookie,
        "GET",
        &format!("/api/v1/bacnet/devices/{id}/objects"),
        None,
    )
    .await;
    let (_, b) = call(
        &app,
        &cookie,
        "GET",
        &format!("/api/v1/bacnet/devices/{id2}/objects"),
        None,
    )
    .await;
    assert_eq!(strip(&a), strip(&b));
    let (_, devices) = call(&app, &cookie, "GET", "/api/v1/bacnet/devices", None).await;
    let d2 = devices
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == id2)
        .unwrap();
    assert_eq!(d2["device_instance"], 1002);
    assert_eq!(d2["port"], 47809);
    assert_eq!(d2["interval_ms"], 3000);
    assert_eq!(d2["cov_enabled"], true);
    assert_eq!(d2["cov_lifetime"], 120);

    // The built-in AHU instantiates too.
    let (status, made3) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/trane-tracer-sc/instantiate"),
        Some(json!({"name": "Hall A AHU", "ip": "10.0.0.7", "device_instance": 5})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{made3}");
    let (_, tmpl) = call(
        &app,
        &cookie,
        "GET",
        &format!("{BASE}/trane-tracer-sc"),
        None,
    )
    .await;
    assert_eq!(
        made3["item_count"],
        tmpl["objects"].as_array().unwrap().len()
    );
}

#[tokio::test]
async fn import_validates_and_refuses_duplicates() {
    let (app, cookie, _) = admin_app().await;

    let mut bad = modbus_template("Bad Id", "1.0");
    bad["registers"] = json!([
        {"name": "a", "offset": 0, "register_type": "float128"},
        {"name": "a", "offset": 65535}
    ]);
    let (status, body) = call(&app, &cookie, "POST", BASE, Some(bad)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let mut too_big = modbus_template("acme-x", "1.0.0");
    too_big["registers"] = json!([{"name": "a", "offset": 70000}]);
    let (status, _) = call(&app, &cookie, "POST", BASE, Some(too_big)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let msg = body["error"].as_str().unwrap();
    assert!(
        msg.contains("template_id")
            && msg.contains("semantic version")
            && msg.contains("register_type"),
        "{msg}"
    );

    let (status, _) = call(&app, &cookie, "POST", BASE, Some(json!({"hello": "world"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let mut other_format = modbus_template("acme-x", "1.0.0");
    other_format["format"] = json!("voltrus-device-template/9");
    let (status, _) = call(&app, &cookie, "POST", BASE, Some(other_format)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let mut mixed = modbus_template("acme-y", "1.0.0");
    mixed["objects"] = json!([{"object_type": "analogInput", "object_instance": 1, "name": "x"}]);
    let (status, _) = call(&app, &cookie, "POST", BASE, Some(mixed)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let mut reserved = modbus_template("catalog", "1.0.0");
    reserved["template_id"] = json!("catalog");
    let (status, _) = call(&app, &cookie, "POST", BASE, Some(reserved)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, ok) = call(
        &app,
        &cookie,
        "POST",
        BASE,
        Some(modbus_template("acme-meter-9", "1.0.0")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{ok}");
    let (status, _) = call(
        &app,
        &cookie,
        "POST",
        BASE,
        Some(modbus_template("acme-meter-9", "1.0.0")),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(
        &app,
        &cookie,
        "POST",
        BASE,
        Some(modbus_template("acme-meter-9", "1.1.0")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Newest version by default, a named one on request; the metadata survives.
    let (_, newest) = call(&app, &cookie, "GET", &format!("{BASE}/acme-meter-9"), None).await;
    assert_eq!(newest["version"], "1.1.0");
    assert_eq!(newest["verified_notes"], "Firmware 2.1 on site");
    assert_eq!(
        newest["quirks"][0],
        "Word order is swapped on firmware before 2.0"
    );
    assert_eq!(newest["registers"][0]["unit"], "V");
    assert_eq!(newest["registers"][1]["register_area"], "input");
    let (_, old) = call(
        &app,
        &cookie,
        "GET",
        &format!("{BASE}/acme-meter-9?version=1.0.0"),
        None,
    )
    .await;
    assert_eq!(old["version"], "1.0.0");

    // Delete the newest; the older one is next.
    let (status, _) = call(
        &app,
        &cookie,
        "DELETE",
        &format!("{BASE}/acme-meter-9"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, newest) = call(&app, &cookie, "GET", &format!("{BASE}/acme-meter-9"), None).await;
    assert_eq!(newest["version"], "1.0.0");
}

#[tokio::test]
async fn viewers_read_but_do_not_write() {
    let (app, admin, storage) = admin_app().await;
    let viewer = viewer_cookie(&app, &storage).await;
    let (status, _) = call(&app, &viewer, "GET", BASE, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&app, &viewer, "GET", &format!("{BASE}/abb-azl402"), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        &viewer,
        "POST",
        BASE,
        Some(modbus_template("acme-meter-9", "1.0.0")),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &app,
        &viewer,
        "POST",
        &format!("{BASE}/abb-azl402/instantiate"),
        Some(json!({"name": "x", "ip": "1.2.3.4"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &app,
        &viewer,
        "POST",
        &format!("{BASE}/catalog/import"),
        Some(json!({"template_id": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &app,
        &viewer,
        "GET",
        &format!("{BASE}/catalog?url=http://127.0.0.1:1/index.json"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let _ = admin;
}

/// A catalog served from a local socket: `files` maps a path to its bytes.
pub(super) async fn serve(files: HashMap<String, Vec<u8>>) -> String {
    let files = std::sync::Arc::new(files);
    let app = Router::new().fallback(move |uri: Uri| {
        let files = files.clone();
        async move {
            match files.get(uri.path()) {
                Some(body) => (StatusCode::OK, body.clone()).into_response(),
                None => StatusCode::NOT_FOUND.into_response(),
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

pub(super) fn entry(t: &Value, path: &str, sha: &str) -> Value {
    json!({
        "template_id": t["template_id"], "manufacturer": t["manufacturer"], "model": t["model"],
        "protocol": t["protocol"], "version": t["version"], "verified": t["verified"],
        "path": path, "sha256": sha
    })
}

#[tokio::test]
async fn catalog_import_verifies_sha256() {
    let (app, cookie, storage) = admin_app().await;

    let good = modbus_template("acme-good", "1.2.0");
    let tampered = modbus_template("acme-tampered", "1.0.0");
    let wrong_id = modbus_template("acme-other", "1.0.0");
    let good_bytes = serde_json::to_vec_pretty(&good).unwrap();
    let tampered_bytes = serde_json::to_vec_pretty(&tampered).unwrap();
    let wrong_bytes = serde_json::to_vec_pretty(&wrong_id).unwrap();
    // The index lists the hash of what the file held before someone edited it.
    let stale_sha = sha256_hex(b"the file as published");
    let mut index_entries = vec![
        entry(&good, "templates/good.json", &sha256_hex(&good_bytes)),
        entry(&tampered, "templates/tampered.json", &stale_sha),
        // Lists acme-claimed but the file is another template, with a matching hash.
        entry(
            &json!({"template_id": "acme-claimed", "manufacturer": "Acme", "model": "M", "protocol": "modbus", "version": "1.0.0", "verified": false}),
            "templates/wrong.json",
            &sha256_hex(&wrong_bytes),
        ),
        entry(
            &json!({"template_id": "acme-missing", "manufacturer": "Acme", "model": "M", "protocol": "modbus", "version": "1.0.0", "verified": false}),
            "templates/missing.json",
            &sha256_hex(b"x"),
        ),
    ];
    // An older version of the good one: import without a version takes the newest.
    let mut old = good.clone();
    old["version"] = json!("1.0.0");
    let old_bytes = serde_json::to_vec_pretty(&old).unwrap();
    index_entries.push(entry(
        &old,
        "templates/good-old.json",
        &sha256_hex(&old_bytes),
    ));
    let index = json!({"format": "voltrus-template-catalog/1", "templates": index_entries});

    let base = serve(HashMap::from([
        (
            "/cat/index.json".to_string(),
            serde_json::to_vec(&index).unwrap(),
        ),
        ("/cat/templates/good.json".to_string(), good_bytes),
        ("/cat/templates/good-old.json".to_string(), old_bytes),
        ("/cat/templates/tampered.json".to_string(), tampered_bytes),
        ("/cat/templates/wrong.json".to_string(), wrong_bytes),
    ]))
    .await;
    let url = format!("{base}/cat/index.json");
    storage
        .upsert_setting(SETTING_CATALOG_URL, &url)
        .await
        .unwrap();

    let (status, cat) = call(&app, &cookie, "GET", &format!("{BASE}/catalog"), None).await;
    assert_eq!(status, StatusCode::OK, "{cat}");
    assert_eq!(cat["url"], url.as_str());
    assert_eq!(cat["templates"].as_array().unwrap().len(), 5);
    assert!(
        cat["templates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["installed"] == false)
    );

    // A file that does not match its hash is rejected and nothing is stored.
    let (status, body) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/catalog/import"),
        Some(json!({"template_id": "acme-tampered", "version": "1.0.0"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("sha256 mismatch"),
        "{body}"
    );
    let (status, _) = call(&app, &cookie, "GET", &format!("{BASE}/acme-tampered"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A matching hash on a file that is not the listed template is rejected too.
    let (status, body) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/catalog/import"),
        Some(json!({"template_id": "acme-claimed"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");

    // A listed file the server does not have is a fetch failure, an unlisted id is not found.
    let (status, _) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/catalog/import"),
        Some(json!({"template_id": "acme-missing"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let (status, _) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/catalog/import"),
        Some(json!({"template_id": "acme-nope"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The good one imports, newest version by default, stored with its catalog as source.
    let (status, ok) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/catalog/import"),
        Some(json!({"template_id": "acme-good"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{ok}");
    assert_eq!(ok["version"], "1.2.0");
    assert_eq!(ok["source"], format!("catalog:{url}"));
    let (_, t) = call(&app, &cookie, "GET", &format!("{BASE}/acme-good"), None).await;
    assert_eq!(t["source"], format!("catalog:{url}"));
    assert_eq!(t["read_only"], false);
    assert_eq!(t["registers"].as_array().unwrap().len(), 2);
    let (status, _) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/catalog/import"),
        Some(json!({"template_id": "acme-good"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (_, cat) = call(&app, &cookie, "GET", &format!("{BASE}/catalog"), None).await;
    let installed: Vec<(String, bool)> = cat["templates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                format!(
                    "{} {}",
                    t["template_id"].as_str().unwrap(),
                    t["version"].as_str().unwrap()
                ),
                t["installed"].as_bool().unwrap(),
            )
        })
        .collect();
    assert!(installed.contains(&("acme-good 1.2.0".to_string(), true)));
    assert!(installed.contains(&("acme-good 1.0.0".to_string(), false)));

    let (_, listed) = call(
        &app,
        &cookie,
        "GET",
        &format!("{BASE}?source=catalog"),
        None,
    )
    .await;
    assert_eq!(ids(&listed), vec!["acme-good"]);

    // The imported template instantiates like any other.
    let (status, _) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/acme-good/instantiate"),
        Some(json!({"name": "Good", "ip": "127.0.0.1", "port": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // An explicit url (admin) beats the setting; a bad one is refused before any request.
    let (status, _) = call(
        &app,
        &cookie,
        "GET",
        &format!("{BASE}/catalog?url=ftp://example.com/index.json"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = call(
        &app,
        &cookie,
        "POST",
        &format!("{BASE}/catalog/import"),
        Some(json!({"template_id": "acme-good", "version": "1.0.0", "url": url})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

#[tokio::test]
async fn catalog_rejects_unreachable_and_malformed_indexes() {
    let (app, cookie, storage) = admin_app().await;

    // Nothing listens on port 1.
    storage
        .upsert_setting(SETTING_CATALOG_URL, "http://127.0.0.1:1/index.json")
        .await
        .unwrap();
    let (status, _) = call(&app, &cookie, "GET", &format!("{BASE}/catalog"), None).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);

    let base = serve(HashMap::from([
        ("/not-json".to_string(), b"<html>".to_vec()),
        ("/other-format".to_string(), br#"{"format":"x/1","templates":[]}"#.to_vec()),
        (
            "/escape".to_string(),
            serde_json::to_vec(&json!({"format": "voltrus-template-catalog/1", "templates": [
                {"template_id": "acme-a", "manufacturer": "A", "model": "A", "protocol": "modbus", "version": "1.0.0", "path": "../secret.json", "sha256": "a".repeat(64)}
            ]})).unwrap(),
        ),
        ("/big".to_string(), vec![b' '; 5 * 1024 * 1024]),
    ]))
    .await;
    for path in ["not-json", "other-format", "escape", "big"] {
        let (status, body) = call(
            &app,
            &cookie,
            "GET",
            &format!("{BASE}/catalog?url={base}/{path}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{path}: {body}");
    }
    let (status, _) = call(
        &app,
        &cookie,
        "GET",
        &format!("{BASE}/catalog?url={base}/missing"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
}
