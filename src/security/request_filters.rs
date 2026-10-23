use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;

// ── Range Request Limiting ──

const MAX_RANGE_SIZE_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

/// Middleware that limits HTTP Range request sizes to prevent DoS via
/// large range requests on static assets.
pub async fn limit_range_requests_middleware(req: Request, next: Next) -> Response {
    let mut req = req;
    if let Some(range) = req.headers().get("range") {
        if let Ok(range_str) = range.to_str() {
            // Parse simple byte ranges like "bytes=0-10485759"
            // Reject if the requested range exceeds the limit
            if let Some(size) = parse_range_size(range_str) {
                if size > MAX_RANGE_SIZE_BYTES {
                    return (
                        StatusCode::RANGE_NOT_SATISFIABLE,
                        axum::Json(json!({"error": "Requested range too large"})),
                    )
                        .into_response();
                }
            } else {
                // If we can't parse the range, strip it to be safe
                req.headers_mut().remove("range");
            }
        } else {
            req.headers_mut().remove("range");
        }
    }
    next.run(req).await
}

fn parse_range_size(range: &str) -> Option<u64> {
    // Handle "bytes=start-end" format
    let range = range.strip_prefix("bytes=")?;
    for part in range.split(',') {
        let part = part.trim();
        if let Some((start, end)) = part.split_once('-') {
            let start: u64 = start.parse().ok()?;
            if end.is_empty() {
                // Range like "bytes=start-" means from start to end of file
                // We can't know the file size, so treat as potentially large
                return None;
            }
            let end: u64 = end.parse().ok()?;
            if end >= start {
                return Some(end - start + 1);
            }
        }
    }
    None
}

// ── Source Map Blocking ──

/// Middleware that blocks requests for source map files (.map) to prevent
/// information disclosure in production.
pub async fn block_source_maps_middleware(req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if path.ends_with(".map") {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    next.run(req).await
}
