pub mod audit;
pub mod csrf;
pub mod headers;
pub mod intrusion;
pub mod rate_limiter;
pub mod request_filters;
pub mod request_id;
pub mod sanitization;
pub mod signing;

// Re-export all public items for backward compatibility
pub use audit::*;
pub use csrf::*;
pub use headers::*;
pub use intrusion::*;
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
    fn test_generate_csrf_token() {
        let t1 = generate_csrf_token();
        let t2 = generate_csrf_token();
        assert_eq!(t1.len(), CSRF_TOKEN_LENGTH);
        assert_ne!(t1, t2);
    }
}
