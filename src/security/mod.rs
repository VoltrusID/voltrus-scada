pub mod audit;
pub mod headers;
pub mod intrusion;
pub mod mcp_guard;
pub mod rate_limiter;
pub mod request_filters;
pub mod redact;
pub mod request_id;
pub mod sanitization;
pub mod secrets;
pub mod signing;

// Re-export all public items for backward compatibility
pub use audit::*;
pub use headers::*;
pub use intrusion::*;
pub use mcp_guard::McpGuard;
pub use rate_limiter::*;
pub use request_filters::*;
pub use request_id::*;
pub use sanitization::*;

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_allows_under_limit() {
        let limiter = LoginRateLimiter::new();
        assert!(limiter.is_allowed("user1"));
        limiter.record_attempt("user1");
        assert!(limiter.is_allowed("user1"));
    }

    #[test]
    fn test_rate_limiter_blocks_over_limit() {
        let limiter = LoginRateLimiter::new();
        for _ in 0..LOGIN_MAX_ATTEMPTS {
            limiter.record_attempt("user2");
        }
        assert!(!limiter.is_allowed("user2"));
    }

    #[test]
    fn test_sanitize_brand_strips_html() {
        assert_eq!(
            sanitize_brand_input("Hello <script>alert(1)</script>"),
            "Hello alert(1)"
        );
        assert_eq!(sanitize_brand_input("  Normal Title  "), "Normal Title");
    }

    #[test]
    fn test_sanitize_brand_limits_length() {
        let long = "a".repeat(MAX_BRAND_LENGTH + 100);
        let result = sanitize_brand_input(&long);
        assert_eq!(result.len(), MAX_BRAND_LENGTH);
    }

    #[test]
    fn test_sanitize_brand_cuts_long_non_ascii_titles_on_char_boundaries() {
        let title = "Pabrik Gula Djatiroto — Ruang Kontrol 🏭 ".repeat(10);
        let result = sanitize_brand_input(&title);
        // The 200th character is a space, trimmed off the end.
        assert_eq!(result.chars().count(), MAX_BRAND_LENGTH - 1);
        assert!(title.starts_with(&result));

        // Multi-byte characters straddling the byte the old code cut at.
        for s in [
            "é".repeat(150),
            "🏭".repeat(250),
            format!("a{}", "—".repeat(300)),
        ] {
            let result = sanitize_brand_input(&s);
            assert!(result.chars().count() <= MAX_BRAND_LENGTH);
            assert!(s.starts_with(&result));
        }
        assert_eq!(sanitize_brand_input("Ruang Kontrol 🏭"), "Ruang Kontrol 🏭");
    }

    #[test]
    fn test_sanitize_logo_keeps_valid_logos_whole() {
        let logo = format!(
            "data:image/png;base64,{}",
            "A".repeat(MAX_LOGO_FILE_BYTES.div_ceil(3) * 4)
        );
        assert!(logo.len() <= MAX_LOGO_BYTES, "a 100 KB file fits");
        assert_eq!(sanitize_logo_input(&format!("  {logo} ")).unwrap(), logo);
        assert_eq!(
            sanitize_logo_input("https://example.com/l.svg").unwrap(),
            "https://example.com/l.svg"
        );
        assert_eq!(sanitize_logo_input("").unwrap(), "");
    }

    #[test]
    fn test_sanitize_logo_rejects_oversized_logos_with_400() {
        use axum::response::IntoResponse;
        let logo = "A".repeat(MAX_LOGO_BYTES + 1);
        let err = sanitize_logo_input(&logo).unwrap_err();
        assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST);
        assert!(err.1.contains("100 KB"), "{}", err.1);

        // Through `?` in a handler it answers 400, not 500.
        fn handler(logo: &str) -> Result<String, crate::api::AppError> {
            Ok(sanitize_logo_input(logo)?)
        }
        let resp = handler(&logo).unwrap_err().into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_sanitize_brand_color() {
        assert_eq!(sanitize_brand_color(" #22C55E ").unwrap(), "#22c55e");
        assert_eq!(sanitize_brand_color("#0af").unwrap(), "#0af");
        assert_eq!(sanitize_brand_color("").unwrap(), "");
        for bad in ["22c55e", "#22c55", "#ggg", "red", "#22c55e;x", "url(x)"] {
            assert!(sanitize_brand_color(bad).is_err(), "{bad}");
        }
    }

    fn req_from(peer: &str, headers: &[(&str, &str)]) -> axum::extract::Request {
        let mut b = axum::http::Request::builder();
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        let mut req = b.body(axum::body::Body::empty()).unwrap();
        req.extensions_mut().insert(axum::extract::ConnectInfo(
            peer.parse::<std::net::SocketAddr>().unwrap(),
        ));
        req
    }

    #[test]
    fn client_ip_ignores_proxy_headers_from_remote_peer() {
        let req = req_from(
            "203.0.113.7:5000",
            &[
                ("cf-connecting-ip", "1.2.3.4"),
                ("x-forwarded-for", "5.6.7.8"),
            ],
        );
        assert_eq!(client_ip(&req), "203.0.113.7");
    }

    #[test]
    fn client_ip_trusts_proxy_headers_from_loopback_in_order() {
        let both = req_from(
            "127.0.0.1:5000",
            &[
                ("x-forwarded-for", "5.6.7.8, 10.0.0.1"),
                ("cf-connecting-ip", "1.2.3.4"),
            ],
        );
        assert_eq!(client_ip(&both), "1.2.3.4");
        let xff = req_from("[::1]:5000", &[("x-forwarded-for", " 5.6.7.8 , 10.0.0.1")]);
        assert_eq!(client_ip(&xff), "5.6.7.8");
    }

    #[test]
    fn client_ip_falls_back_to_peer_on_garbage_header_or_unknown_without_peer() {
        let bad = req_from("127.0.0.1:5000", &[("cf-connecting-ip", "not-an-ip")]);
        assert_eq!(client_ip(&bad), "127.0.0.1");
        let none = axum::http::Request::builder()
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(client_ip(&none), "unknown");
    }
}
