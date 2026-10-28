use super::*;
use crate::models::CookieSecure;
use axum::body::Body;
use axum::extract::ConnectInfo;
use http::{Request, StatusCode};
use tower::ServiceExt;

/// `Set-Cookie` header of `name` from a response.
fn set_cookie(resp: &http::Response<Body>, name: &str) -> String {
    resp.headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no {name} cookie"))
        .to_string()
}

fn has_secure(cookie: &str) -> bool {
    cookie
        .split(';')
        .any(|a| a.trim().eq_ignore_ascii_case("secure"))
}

async fn login_as(
    policy: CookieSecure,
    peer: &str,
    headers: &[(&str, &str)],
) -> http::Response<Body> {
    let mut state = seeded_admin_app_state().await;
    state.cookie_secure = policy;
    let app = create_router(state);

    let mut req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header("content-type", "application/json");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut req = req
        .body(Body::from(r#"{"username":"admin","password":"admin"}"#))
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(peer.parse::<std::net::SocketAddr>().unwrap()));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    resp
}

#[tokio::test]
async fn auto_leaves_the_cookie_usable_over_plain_http_on_the_lan() {
    let resp = login_as(CookieSecure::Auto, "192.168.1.50:51000", &[]).await;
    for name in ["voltrus_token", "voltrus_refresh"] {
        let cookie = set_cookie(&resp, name);
        assert!(
            !has_secure(&cookie),
            "{name} must not be Secure over http: {cookie}"
        );
        assert!(cookie.contains("HttpOnly"), "{cookie}");
    }
}

#[tokio::test]
async fn auto_sets_secure_behind_a_loopback_https_proxy() {
    for headers in [
        vec![("x-forwarded-proto", "https")],
        vec![("x-forwarded-proto", "HTTPS, http")],
        vec![("cf-visitor", r#"{"scheme":"https"}"#)],
    ] {
        let resp = login_as(CookieSecure::Auto, "127.0.0.1:51000", &headers).await;
        for name in ["voltrus_token", "voltrus_refresh"] {
            assert!(has_secure(&set_cookie(&resp, name)), "{headers:?} {name}");
        }
    }
}

#[tokio::test]
async fn auto_is_not_secure_when_the_proxy_says_http_or_is_not_on_loopback() {
    for (peer, headers) in [
        ("127.0.0.1:51000", vec![("x-forwarded-proto", "http")]),
        (
            "127.0.0.1:51000",
            vec![("cf-visitor", r#"{"scheme":"http"}"#)],
        ),
        ("127.0.0.1:51000", vec![]),
        // A remote client cannot claim HTTPS.
        ("203.0.113.9:51000", vec![("x-forwarded-proto", "https")]),
    ] {
        let resp = login_as(CookieSecure::Auto, peer, &headers).await;
        assert!(
            !has_secure(&set_cookie(&resp, "voltrus_token")),
            "{peer} {headers:?}"
        );
    }
}

#[tokio::test]
async fn explicit_settings_ignore_the_request() {
    let resp = login_as(CookieSecure::Always, "192.168.1.50:51000", &[]).await;
    assert!(has_secure(&set_cookie(&resp, "voltrus_token")));
    let resp = login_as(
        CookieSecure::Never,
        "127.0.0.1:51000",
        &[("x-forwarded-proto", "https")],
    )
    .await;
    assert!(!has_secure(&set_cookie(&resp, "voltrus_token")));
}

#[tokio::test]
async fn refresh_follows_the_same_rule() {
    let mut state = seeded_admin_app_state().await;
    state.cookie_secure = CookieSecure::Auto;
    let app = create_router(state);
    let peer: std::net::SocketAddr = "192.168.1.50:51000".parse().unwrap();

    let mut login = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"username":"admin","password":"admin"}"#))
        .unwrap();
    login.extensions_mut().insert(ConnectInfo(peer));
    let resp = app.clone().oneshot(login).await.unwrap();
    let refresh = extract_cookie_value(resp.headers(), "voltrus_refresh").unwrap();

    let mut req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/refresh")
        .header("cookie", format!("voltrus_refresh={refresh}"))
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(peer));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(!has_secure(&set_cookie(&resp, "voltrus_token")));
    assert!(!has_secure(&set_cookie(&resp, "voltrus_refresh")));
}

#[test]
fn cookie_secure_config_accepts_bool_or_auto_and_defaults_to_auto() {
    fn parse(yaml: &str) -> Result<crate::models::AppConfig, serde_yaml_neo::Error> {
        serde_yaml_neo::from_str(yaml)
    }
    assert_eq!(parse("").unwrap().cookie_secure, CookieSecure::Auto);
    assert_eq!(
        parse("cookie_secure: auto").unwrap().cookie_secure,
        CookieSecure::Auto
    );
    assert_eq!(
        parse("cookie_secure: \"auto\"").unwrap().cookie_secure,
        CookieSecure::Auto
    );
    assert_eq!(
        parse("cookie_secure: true").unwrap().cookie_secure,
        CookieSecure::Always
    );
    assert_eq!(
        parse("cookie_secure: false").unwrap().cookie_secure,
        CookieSecure::Never
    );
    assert!(parse("cookie_secure: sometimes").is_err());
}
