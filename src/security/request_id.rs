use axum::extract::Request;
use axum::http::header;
use axum::middleware::Next;
use axum::response::Response;
use rand::Rng as _;

pub fn generate_request_id() -> String {
    let id: String = rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(16)
        .map(char::from)
        .collect();
    id.to_lowercase()
}

/// Middleware that adds a unique X-Request-Id to every request for tracing.
pub async fn request_id_middleware(req: Request, next: Next) -> Response {
    let request_id = generate_request_id();
    let mut response = next.run(req).await;
    response.headers_mut().insert(
        "X-Request-Id",
        header::HeaderValue::from_str(&request_id).unwrap(),
    );
    response
}
