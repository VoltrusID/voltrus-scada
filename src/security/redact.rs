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

/// Scheme, host and port of a URL (`None` when it does not parse).
fn origin(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()
        .map(|u| u.origin().ascii_serialization())
}

fn holds_mask(node: &Value) -> bool {
    let header_masked = node
        .get("headers")
        .and_then(Value::as_object)
        .is_some_and(|h| h.values().any(|v| v.as_str() == Some(SECRET_MASK)));
    let url_masked = node
        .get("url")
        .and_then(Value::as_str)
        .and_then(|u| reqwest::Url::parse(u).ok())
        .is_some_and(|u| u.password() == Some(SECRET_MASK) || u.username() == SECRET_MASK);
    header_masked || url_masked
}

/// `submitted` (a v2 graph document) with the masked secrets of each
/// `http_out` node put back from the node of the same id in `stored` (the
/// flow's stored nodes as JSON text). Returns whether anything was restored.
///
/// Masked header values are restored only while the node still posts to
/// the same origin (scheme, host, port) as the stored node, so an edit
/// cannot send the stored secrets to another host. A mask left over (a
/// changed origin, a new node or header) is an error asking for the secrets
/// to be entered again.
pub fn restore_flow_graph(submitted: &mut Value, stored: Option<&str>) -> Result<bool, String> {
    let stored = stored.and_then(|s| serde_json::from_str::<Value>(s).ok());
    let stored_nodes: Vec<&Value> = match &stored {
        Some(Value::Array(list)) => list.iter().collect(),
        Some(Value::Object(map)) => map
            .get("nodes")
            .and_then(Value::as_array)
            .map(|l| l.iter().collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let Some(list) = submitted.get_mut("nodes").and_then(Value::as_array_mut) else {
        return Ok(false);
    };
    let mut changed = false;
    for node in list.iter_mut().filter(|n| is_http_out(n)) {
        let id = node
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let old = stored_nodes
            .iter()
            .find(|n| is_http_out(n) && n.get("id").and_then(Value::as_str) == Some(id.as_str()));
        if let Some(old) = old {
            let old_url = old.get("url").and_then(Value::as_str);
            if let Some(url) = node.get("url").and_then(Value::as_str) {
                let restored = restore_url(url, old_url);
                if restored != url {
                    node["url"] = Value::String(restored);
                    changed = true;
                }
            }
            let same_origin = match (node.get("url").and_then(Value::as_str), old_url) {
                (Some(new), Some(old)) => origin(new).is_some() && origin(new) == origin(old),
                _ => false,
            };
            if same_origin && let Some(headers) = node.get("headers") {
                let restored = restore_headers(headers, old.get("headers"));
                if &restored != headers {
                    node["headers"] = restored;
                    changed = true;
                }
            }
        }
        if holds_mask(node) {
            return Err(format!(
                "node '{id}': its URL changed or a value is new, so the masked secrets \
                 ({SECRET_MASK}) cannot be kept; enter the secrets again"
            ));
        }
    }
    Ok(changed)
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
        assert_eq!(restore_flow_graph(&mut submitted, Some(&stored)), Ok(true));
        let original: Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(submitted, original);

        // Another host does not get the stored header values.
        let mut moved: Value = serde_json::from_str(&masked).unwrap();
        moved["nodes"][0]["url"] = json!("https://elsewhere.invalid/");
        let err = restore_flow_graph(&mut moved, Some(&stored)).unwrap_err();
        assert!(err.contains("enter the secrets again"), "{err}");
        // The same origin, another path, keeps them.
        let mut same: Value = serde_json::from_str(&masked).unwrap();
        same["nodes"][0]["url"] = json!("https://hooks.invalid/other");
        assert_eq!(restore_flow_graph(&mut same, Some(&stored)), Ok(true));
        assert_eq!(same["nodes"][0]["headers"]["Authorization"], "Bearer abc");
        // A mask on a node that has no stored one.
        let mut fresh = json!({"version": 2, "nodes": [
            {"type": "http_out", "id": "new", "url": "https://hooks.invalid/", "headers": {"X": SECRET_MASK}}
        ], "edges": []});
        assert!(restore_flow_graph(&mut fresh, Some(&stored)).is_err());
    }
}
