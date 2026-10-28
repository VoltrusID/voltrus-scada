//! The web UI built into the executable.
//!
//! `build.rs` embeds `static/` into release builds (see there). The embedded UI
//! is served unless a `static/index.html` exists in the working directory: then
//! that directory is served instead, whole, through `ServeDir`. That lets a site
//! replace the UI (a patched build, a custom theme) without rebuilding, and keeps
//! `npm run build` plus a refresh working against a running dev server. The
//! consequence for upgrades: a `static/` folder left over from an older release
//! shadows the new UI, which is why [`log_source`] says so at startup.

use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// A file of the embedded UI.
pub struct Asset {
    /// Path relative to the UI root, `/`-separated: `index.html`, `assets/app-1a2b.js`.
    pub path: &'static str,
    /// The bytes as embedded: gzip when `gzipped`, else the file as it is.
    pub data: &'static [u8],
    pub gzipped: bool,
    /// Size of the file itself, before compression.
    pub len: usize,
    /// Hash of the file's content, the base of its `ETag`.
    pub hash: u64,
}

mod generated {
    use super::Asset;
    include!(concat!(env!("OUT_DIR"), "/ui_assets.rs"));
}

/// The files embedded by `build.rs`, sorted by path. Empty in a debug build.
pub fn embedded() -> &'static [Asset] {
    generated::ASSETS
}

const DISK_INDEX: &str = "static/index.html";

async fn disk_override_present() -> bool {
    tokio::fs::metadata(DISK_INDEX)
        .await
        .is_ok_and(|m| m.is_file())
}

/// Say at startup which UI the server hands out.
pub async fn log_source() {
    if disk_override_present().await {
        tracing::warn!(
            "Serving the web UI from ./static, which overrides the one built into this binary. \
             Delete or rename that folder to use the built-in UI (a leftover from an older release shows the old UI)."
        );
    } else if embedded().is_empty() {
        tracing::warn!(
            "No web UI: this build embeds none and ./static/index.html does not exist. \
             Build the client (cd client && npm run build) or use a release binary."
        );
    } else {
        tracing::info!(
            "Serving the web UI built into this binary ({} files)",
            embedded().len()
        );
    }
}

/// Answer from the embedded UI. Anything else goes on to the disk service
/// behind it: with an override present, or when nothing is embedded.
pub async fn embedded_ui_middleware(req: Request, next: Next) -> Response {
    if embedded().is_empty() || disk_override_present().await {
        return next.run(req).await;
    }
    respond(embedded(), req.method(), req.uri().path(), req.headers())
}

fn find<'a>(assets: &'a [Asset], path: &str) -> Option<&'a Asset> {
    assets
        .binary_search_by(|a| a.path.cmp(path))
        .ok()
        .map(|i| &assets[i])
}

/// Serve one request from `assets`: the file at the path, or `index.html` for a
/// path without a file extension (a client-side route such as `/alarms`, so a
/// reload or a shared link works). A missing path that names a file answers 404,
/// not the page, so a stale script reference fails visibly.
fn respond(assets: &[Asset], method: &Method, uri_path: &str, headers: &HeaderMap) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET, HEAD")],
        )
            .into_response();
    }

    let path = uri_path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let asset = match find(assets, path) {
        Some(a) => a,
        None => {
            let last = path.rsplit('/').next().unwrap_or(path);
            match (last.contains('.'), find(assets, "index.html")) {
                (false, Some(index)) => index,
                _ => return (StatusCode::NOT_FOUND, "Not found").into_response(),
            }
        }
    };
    serve(asset, method == Method::HEAD, headers)
}

fn serve(asset: &Asset, head_only: bool, request_headers: &HeaderMap) -> Response {
    let send_gzip = asset.gzipped && accepts_gzip(request_headers);
    let etag = if send_gzip {
        format!("\"{:x}-gz\"", asset.hash)
    } else {
        format!("\"{:x}\"", asset.hash)
    };

    let mut resp = if if_none_match(request_headers, &etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let body: Vec<u8> = if head_only {
            Vec::new()
        } else if asset.gzipped && !send_gzip {
            match gunzip(asset.data) {
                Some(b) => b,
                None => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        } else {
            asset.data.to_vec()
        };
        let sent_len = if send_gzip {
            asset.data.len()
        } else {
            asset.len
        };
        let mut r = Response::new(axum::body::Body::from(body));
        let h = r.headers_mut();
        h.insert(header::CONTENT_TYPE, content_type(asset.path));
        h.insert(header::CONTENT_LENGTH, HeaderValue::from(sent_len));
        if send_gzip {
            h.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        }
        r
    };

    let h = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&etag) {
        h.insert(header::ETAG, v);
    }
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control(asset.path)),
    );
    if asset.gzipped {
        h.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    }
    resp
}

/// Files the client build names by content hash never change: cache them for a
/// year. Everything else (the page, the service worker, locales, icons) is
/// revalidated against its ETag on every load.
fn cache_control(path: &str) -> &'static str {
    if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    }
}

fn content_type(path: &str) -> HeaderValue {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let text = mime.type_() == mime_guess::mime::TEXT
        || matches!(mime.subtype().as_str(), "javascript" | "json" | "xml")
        || mime
            .suffix()
            .is_some_and(|s| matches!(s.as_str(), "json" | "xml"));
    let value = if text {
        format!("{mime}; charset=utf-8")
    } else {
        mime.to_string()
    };
    HeaderValue::from_str(&value).unwrap_or(HeaderValue::from_static("application/octet-stream"))
}

fn accepts_gzip(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|item| {
            let mut parts = item.split(';');
            let coding = parts.next().unwrap_or("").trim();
            if !(coding.eq_ignore_ascii_case("gzip") || coding == "*") {
                return false;
            }
            // `gzip;q=0` means "not gzip".
            !parts.any(|p| {
                p.trim()
                    .strip_prefix("q=")
                    .and_then(|q| q.trim().parse::<f32>().ok())
                    .is_some_and(|q| q <= 0.0)
            })
        })
}

fn if_none_match(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|t| t.trim().trim_start_matches("W/"))
        .any(|t| t == "*" || t == etag)
}

fn gunzip(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(data)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;

    fn gz(bytes: &[u8]) -> &'static [u8] {
        let mut e = GzEncoder::new(Vec::new(), Compression::best());
        e.write_all(bytes).unwrap();
        Box::leak(e.finish().unwrap().into_boxed_slice())
    }

    const INDEX: &[u8] = b"<!doctype html><title>Voltrus</title>";
    const JS: &[u8] = b"console.log('hello hello hello hello hello hello hello hello');";

    fn table() -> &'static [Asset] {
        // Sorted by path, as build.rs emits them.
        Box::leak(Box::new([
            Asset {
                path: "assets/app-1a2b3c.js",
                data: gz(JS),
                gzipped: true,
                len: JS.len(),
                hash: 0xabc,
            },
            Asset {
                path: "favicon.png",
                data: b"\x89PNG-bytes",
                gzipped: false,
                len: 10,
                hash: 0xdef,
            },
            Asset {
                path: "index.html",
                data: gz(INDEX),
                gzipped: true,
                len: INDEX.len(),
                hash: 0x123,
            },
            Asset {
                path: "locales/en/translation.json",
                data: gz(b"{}"),
                gzipped: true,
                len: 2,
                hash: 0x456,
            },
        ]))
    }

    fn get(path: &str, headers: &[(&str, &str)]) -> Response {
        let mut map = HeaderMap::new();
        for (k, v) in headers {
            map.append(
                header::HeaderName::try_from(*k).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        respond(table(), &Method::GET, path, &map)
    }

    async fn body(resp: Response) -> Vec<u8> {
        axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    fn header_of(resp: &Response, name: &str) -> String {
        resp.headers()
            .get(name)
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn root_and_index_serve_the_page_with_the_right_type() {
        for path in ["/", "/index.html"] {
            let resp = get(path, &[("accept-encoding", "gzip, br")]);
            assert_eq!(resp.status(), StatusCode::OK);
            assert_eq!(header_of(&resp, "content-type"), "text/html; charset=utf-8");
            assert_eq!(header_of(&resp, "cache-control"), "no-cache");
            assert_eq!(header_of(&resp, "content-encoding"), "gzip");
            assert_eq!(header_of(&resp, "vary"), "accept-encoding");
            assert_eq!(header_of(&resp, "etag"), "\"123-gz\"");
        }
    }

    #[tokio::test]
    async fn gzip_clients_get_the_stored_bytes_others_get_the_file() {
        let resp = get("/assets/app-1a2b3c.js", &[("accept-encoding", "gzip")]);
        assert_eq!(header_of(&resp, "content-encoding"), "gzip");
        let stored = body(resp).await;
        assert_eq!(stored, gz(JS));

        let resp = get("/assets/app-1a2b3c.js", &[]);
        assert_eq!(header_of(&resp, "content-encoding"), "");
        assert_eq!(header_of(&resp, "content-length"), JS.len().to_string());
        assert_eq!(header_of(&resp, "etag"), "\"abc\"");
        assert_eq!(body(resp).await, JS);

        // gzip;q=0 refuses gzip.
        let resp = get(
            "/assets/app-1a2b3c.js",
            &[("accept-encoding", "gzip;q=0, identity")],
        );
        assert_eq!(header_of(&resp, "content-encoding"), "");
        assert_eq!(body(resp).await, JS);
    }

    #[tokio::test]
    async fn hashed_assets_are_immutable_and_other_files_revalidate() {
        let resp = get("/assets/app-1a2b3c.js", &[]);
        assert_eq!(
            header_of(&resp, "cache-control"),
            "public, max-age=31536000, immutable"
        );
        assert_eq!(
            header_of(&resp, "content-type"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            header_of(&get("/locales/en/translation.json", &[]), "cache-control"),
            "no-cache"
        );
        assert_eq!(
            header_of(&get("/locales/en/translation.json", &[]), "content-type"),
            "application/json; charset=utf-8"
        );

        // A file that did not compress is sent as it is, with its own type.
        let png = get("/favicon.png", &[("accept-encoding", "gzip")]);
        assert_eq!(header_of(&png, "content-type"), "image/png");
        assert_eq!(header_of(&png, "content-encoding"), "");
        assert_eq!(header_of(&png, "vary"), "");
        assert_eq!(body(png).await, b"\x89PNG-bytes");
    }

    #[tokio::test]
    async fn a_matching_etag_answers_304_without_a_body() {
        let resp = get("/assets/app-1a2b3c.js", &[("if-none-match", "\"abc\"")]);
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(header_of(&resp, "etag"), "\"abc\"");
        assert!(body(resp).await.is_empty());

        // The gzip variant has its own tag.
        let resp = get(
            "/assets/app-1a2b3c.js",
            &[("accept-encoding", "gzip"), ("if-none-match", "\"abc\"")],
        );
        assert_eq!(resp.status(), StatusCode::OK);
        let resp = get(
            "/assets/app-1a2b3c.js",
            &[
                ("accept-encoding", "gzip"),
                ("if-none-match", "W/\"zzz\", \"abc-gz\""),
            ],
        );
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    }

    #[tokio::test]
    async fn client_routes_get_the_page_and_missing_files_get_404() {
        for path in ["/alarms", "/devices/modbus/new", "/some/deep/route/"] {
            let resp = get(path, &[]);
            assert_eq!(resp.status(), StatusCode::OK, "{path}");
            assert_eq!(header_of(&resp, "content-type"), "text/html; charset=utf-8");
            assert_eq!(body(resp).await, INDEX);
        }
        for path in [
            "/assets/old-99.js",
            "/missing.png",
            "/locales/xx/translation.json",
        ] {
            assert_eq!(get(path, &[]).status(), StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[tokio::test]
    async fn head_has_the_headers_and_no_body_and_other_methods_are_refused() {
        let resp = respond(
            table(),
            &Method::HEAD,
            "/assets/app-1a2b3c.js",
            &HeaderMap::new(),
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(header_of(&resp, "content-length"), JS.len().to_string());
        assert!(body(resp).await.is_empty());

        let resp = respond(table(), &Method::POST, "/", &HeaderMap::new());
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn path_traversal_finds_nothing() {
        for path in [
            "/../Cargo.toml",
            "/assets/../index.html",
            "//etc/passwd",
            "/%2e%2e/secret",
        ] {
            let resp = get(path, &[]);
            // Not a file in the table: a route (page) or a 404, never other content.
            assert!(
                resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::OK,
                "{path}"
            );
        }
        assert!(find(table(), "../Cargo.toml").is_none());
    }

    /// The table `build.rs` writes must be sorted, or `find` misses files.
    #[test]
    fn the_embedded_table_is_sorted_by_path() {
        let paths: Vec<_> = embedded().iter().map(|a| a.path).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
    }
}
