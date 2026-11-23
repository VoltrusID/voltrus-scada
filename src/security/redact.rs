//! Masking of secrets that sit inside configuration (HTTP header values and
//! credentials in a URL) for readers that may not configure: viewers,
//! operators and read-only API keys see the configuration with the secrets
//! replaced by [`SECRET_MASK`]. Roles that configure read the stored values.
//!
//! An editor that sends a masked value back keeps the stored one: see
//! [`restore_headers`], [`restore_url`] and [`restore_flow_graph`].

use serde_json::Value;

/// What a masked secret reads as. Same as the chat-channel and SMTP masks.
pub const SECRET_MASK: &str = "********";

/// `url` with the password of its userinfo (or the user name when there is
/// no password) replaced by [`SECRET_MASK`]. A URL that does not parse, or
/// has no userinfo, comes back unchanged.
pub fn mask_url(url: &str) -> String {
    let Ok(mut parsed) = reqwest::Url::parse(url) else {
        return url.to_string();
    };
    let changed = if parsed.password().is_some() {
        parsed.set_password(Some(SECRET_MASK)).is_ok()
    } else if !parsed.username().is_empty() {
        parsed.set_username(SECRET_MASK).is_ok()
    } else {
        false
    };
    if changed {
        parsed.to_string()
    } else {
        url.to_string()
    }
}

/// The stored URL when `submitted` is the masked form of it, else `submitted`.
pub fn restore_url(submitted: &str, stored: Option<&str>) -> String {
    match stored {
        Some(stored) if submitted != stored && mask_url(stored) == submitted => stored.to_string(),
        _ => submitted.to_string(),
    }
}

/// Every string value of a JSON object replaced by [`SECRET_MASK`] (empty
/// values stay empty). Anything else comes back unchanged.
pub fn mask_header_values(headers: &Value) -> Value {
    match headers {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let v = match v {
                        Value::String(s) if !s.is_empty() => Value::String(SECRET_MASK.into()),
                        other => other.clone(),
                    };
                    (k.clone(), v)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Header values sent as [`SECRET_MASK`] take the stored value of the same
/// header back.
pub fn restore_headers(submitted: &Value, stored: Option<&Value>) -> Value {
    let (Value::Object(map), Some(Value::Object(stored))) = (submitted, stored) else {
        return submitted.clone();
    };
    Value::Object(
        map.iter()
            .map(|(k, v)| {
                let v = match (v.as_str(), stored.get(k)) {
                    (Some(SECRET_MASK), Some(old)) => old.clone(),
                    _ => v.clone(),
                };
                (k.clone(), v)
            })
            .collect(),
    )
}

/// Headers stored as JSON text (outbound webhooks) with their values masked.
pub fn mask_headers_text(headers: &str) -> String {
    match serde_json::from_str::<Value>(headers) {
        Ok(v @ Value::Object(_)) => mask_header_values(&v).to_string(),
        _ => headers.to_string(),
    }
}

fn is_http_out(node: &Value) -> bool {
    node.get("type").and_then(Value::as_str) == Some("http_out")
}

/// A flow graph (`{version, nodes, edges}` or a v1 node array, as JSON text)
/// with the headers and URL credentials of its `http_out` nodes masked.
/// Text that does not parse comes back unchanged.
pub fn mask_flow_nodes(nodes: &str) -> String {
    let Ok(mut doc) = serde_json::from_str::<Value>(nodes) else {
        return nodes.to_string();
    };
    let list = match &mut doc {
        Value::Array(list) => list,
        Value::Object(map) => match map.get_mut("nodes") {
            Some(Value::Array(list)) => list,
            _ => return nodes.to_string(),
        },
        _ => return nodes.to_string(),
    };
    for node in list.iter_mut().filter(|n| is_http_out(n)) {
        if let Some(headers) = node.get("headers") {
            let masked = mask_header_values(headers);
            node["headers"] = masked;
        }
        if let Some(url) = node.get("url").and_then(Value::as_str) {
            let masked = mask_url(url);
            node["url"] = Value::String(masked);
        }
    }
    doc.to_string()
}

/// `submitted` (a v2 graph document) with the masked secrets of each
/// `http_out` node put back from the node of the same id in `stored` (the
/// flow's stored nodes as JSON text). Returns whether anything was restored.
pub fn restore_flow_graph(submitted: &mut Value, stored: Option<&str>) -> bool {
    let Some(stored) = stored.and_then(|s| serde_json::from_str::<Value>(s).ok()) else {
        return false;
    };
    let stored_nodes: Vec<&Value> = match &stored {
        Value::Array(list) => list.iter().collect(),
        Value::Object(map) => map
            .get("nodes")
            .and_then(Value::as_array)
            .map(|l| l.iter().collect())
            .unwrap_or_default(),
        _ => return false,
    };
    let Some(list) = submitted.get_mut("nodes").and_then(Value::as_array_mut) else {
        return false;
    };
    let mut changed = false;
    for node in list.iter_mut().filter(|n| is_http_out(n)) {
        let id = node.get("id").and_then(Value::as_str).unwrap_or_default();
        let Some(old) = stored_nodes
            .iter()
            .find(|n| is_http_out(n) && n.get("id").and_then(Value::as_str) == Some(id))
        else {
            continue;
        };
        if let Some(headers) = node.get("headers") {
            let restored = restore_headers(headers, old.get("headers"));
            if &restored != headers {
                node["headers"] = restored;
                changed = true;
            }
        }
        if let Some(url) = node.get("url").and_then(Value::as_str) {
            let restored = restore_url(url, old.get("url").and_then(Value::as_str));
            if restored != url {
                node["url"] = Value::String(restored);
                changed = true;
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn url_credentials_are_masked_and_restored() {
        let url = format!("https://bob:{}@example.com/hook?x=1", "s3cret");
        let url = url.as_str();
        let masked = mask_url(url);
        assert!(!masked.contains("s3cret"), "{masked}");
        assert!(masked.contains("bob"));
        assert_eq!(restore_url(&masked, Some(url)), url);
        // A changed URL is kept as sent.
        assert_eq!(
            restore_url("https://example.com/other", Some(url)),
            "https://example.com/other"
        );
        // Token as user name only.
        let masked = mask_url("https://tok123@example.com/");
        assert!(!masked.contains("tok123"));
        // No userinfo: unchanged.
        assert_eq!(mask_url("http://10.0.0.5/x"), "http://10.0.0.5/x");
    }

    #[test]
    fn header_values_are_masked_and_restored() {
        let stored = json!({"Authorization": "Bearer abc", "X-Empty": ""});
        let masked = mask_header_values(&stored);
        assert_eq!(masked["Authorization"], SECRET_MASK);
        assert_eq!(masked["X-Empty"], "");
        let submitted = json!({"Authorization": SECRET_MASK, "X-New": "v"});
        let restored = restore_headers(&submitted, Some(&stored));
        assert_eq!(restored["Authorization"], "Bearer abc");
        assert_eq!(restored["X-New"], "v");
    }

    #[test]
    fn flow_graph_round_trip_keeps_secrets() {
        let url = format!("https://u:{}@hooks.invalid/", "pw");
        let stored = json!({"version": 2, "nodes": [
            {"type": "http_out", "id": "h", "url": url, "headers": {"Authorization": "Bearer abc"}},
            {"type": "filter", "id": "f", "condition": "true"}
        ], "edges": []})
        .to_string();
        let masked = mask_flow_nodes(&stored);
        assert!(!masked.contains("Bearer abc"));
        assert!(!masked.contains(":pw@"));
        let mut submitted: Value = serde_json::from_str(&masked).unwrap();
        assert!(restore_flow_graph(&mut submitted, Some(&stored)));
        let original: Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(submitted, original);
    }
}
