use axum::extract::Request;
use axum::http::header;
use axum::middleware::Next;
use axum::response::Response;

/// Middleware that strips the Accept-Ranges header from responses.
/// ServeDir advertises this by default, but we don't enforce range semantics
/// beyond the 10MB limit in limit_range_requests_middleware.
pub async fn strip_accept_ranges_middleware(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    response.headers_mut().remove("accept-ranges");
    response
}

/// Middleware that strips X-Forwarded-Host and X-Forwarded-Port headers
/// to prevent host header injection attacks.
pub async fn strip_forwarded_host_middleware(req: Request, next: Next) -> Response {
    let mut req = req;
    // Remove X-Forwarded-Host to prevent host header injection attacks
    // If you have a known reverse proxy, configure it explicitly instead
    req.headers_mut().remove("X-Forwarded-Host");
    req.headers_mut().remove("X-Forwarded-Port");
    next.run(req).await
}

pub async fn security_headers_middleware(req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    let is_api = path.starts_with("/api/");
    let mut response = next.run(req).await;
    let headers = response.headers_mut();

    // Prevent clickjacking
    headers.insert("X-Frame-Options", header::HeaderValue::from_static("DENY"));

    // Prevent MIME type sniffing
    headers.insert(
        "X-Content-Type-Options",
        header::HeaderValue::from_static("nosniff"),
    );

    // XSS protection for legacy browsers
    headers.insert(
        "X-XSS-Protection",
        header::HeaderValue::from_static("1; mode=block"),
    );

    // Referrer policy
    headers.insert(
        "Referrer-Policy",
        header::HeaderValue::from_static("strict-origin-when-cross-origin"),
    );

    // Content Security Policy
    // Cloudflare injects beacon.min.js for analytics when tunneled, and the
    // frontend loads Google Fonts (Inter, JetBrains Mono) from CDN.
    headers.insert(
        "Content-Security-Policy",
        header::HeaderValue::from_static(
            "default-src 'self'; \
             script-src 'self' https://static.cloudflareinsights.com; \
             style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
             img-src 'self' data: blob:; \
             font-src 'self' https://fonts.gstatic.com; \
             connect-src 'self'; \
             media-src 'self'; \
             object-src 'none'; \
             frame-ancestors 'none'; \
             base-uri 'self'; \
             form-action 'self';",
        ),
    );

    // Permissions Policy
    headers.insert(
        "Permissions-Policy",
        header::HeaderValue::from_static(
            "accelerometer=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), payment=(), usb=()",
        ),
    );

    // HSTS - only in production (when not localhost)
    headers.insert(
        "Strict-Transport-Security",
        header::HeaderValue::from_static("max-age=31536000; includeSubDomains; preload"),
    );

    // Cache control
    if is_api {
        headers.insert(
            "Cache-Control",
            header::HeaderValue::from_static("no-store, private, max-age=0, must-revalidate"),
        );
        headers.insert("Pragma", header::HeaderValue::from_static("no-cache"));
        headers.insert("Expires", header::HeaderValue::from_static("0"));
    } else if path == "/" || path == "/index.html" {
        // Never cache index.html — it contains hashed asset URLs that change on deploy
        headers.insert(
            "Cache-Control",
            header::HeaderValue::from_static(
                "no-store, no-cache, max-age=0, must-revalidate, proxy-revalidate",
            ),
        );
        headers.insert("Pragma", header::HeaderValue::from_static("no-cache"));
        headers.insert("Expires", header::HeaderValue::from_static("0"));
        // Tell Cloudflare explicitly: do not cache
        headers.insert(
            "CDN-Cache-Control",
            header::HeaderValue::from_static("no-store"),
        );
    } else if path.starts_with("/assets/") {
        // Hashed assets can be cached forever — filename changes when content changes
        headers.insert(
            "Cache-Control",
            header::HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    }

    response
}
