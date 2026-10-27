//! Secrets never leave the database in a project file.
//!
//! On export every secret-bearing string becomes a `${VOLTRUS_<SECTION>_<KEY>_<PATH>}`
//! reference. On import the references are resolved from the process
//! environment; when a variable is unset the value already stored for the same
//! entity is kept, so an export re-imported into the same database never needs
//! the environment.
//!
//! A string is secret-bearing when its key looks like a credential
//! (`password`, `token`, `api_key`, `authorization`, ...), when it is a URL
//! carrying `user:password@` or a credential query parameter, or when the
//! section forces it (webhook URLs, webhook notification targets).

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde_yaml_neo::{Mapping, Value};

static PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$\{([A-Z_][A-Z0-9_]*)\}").expect("placeholder regex"));
static URL_USERINFO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([A-Za-z][A-Za-z0-9+.\-]*://[^/?#@\s:]*):([^/?#@\s]+)@").expect("userinfo regex")
});
static URL_QUERY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)([?&])(access_token|api_?key|apikey|auth|key|password|passwd|pwd|secret|sig|signature|token)=([^&#\s]*)")
        .expect("query regex")
});

const SECRET_KEY_PARTS: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "secret",
    "token",
    "apikey",
    "api_key",
    "api-key",
    "authorization",
    "bearer",
    "cookie",
    "credential",
    "private_key",
    "private-key",
    "privatekey",
    "auth_key",
];
const SECRET_KEY_EXACT: &[&str] = &["auth", "pwd", "x-auth"];
const URL_KEYS: &[&str] = &[
    "url",
    "uri",
    "endpoint",
    "destination",
    "webhook_url",
    "broker",
    "dsn",
    "connection_string",
    "address",
];

fn is_secret_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    SECRET_KEY_EXACT.contains(&k.as_str()) || SECRET_KEY_PARTS.iter().any(|p| k.contains(p))
}

fn is_url_key(key: &str) -> bool {
    URL_KEYS.contains(&key.to_ascii_lowercase().as_str())
}

pub fn env_name(parts: &[&str]) -> String {
    let mut out = String::from("VOLTRUS");
    for p in parts {
        out.push('_');
        let mut last_us = true;
        for c in p.chars() {
            if c.is_ascii_alphanumeric() {
                out.push(c.to_ascii_uppercase());
                last_us = false;
            } else if !last_us {
                out.push('_');
                last_us = true;
            }
        }
        while out.ends_with('_') {
            out.pop();
        }
    }
    out
}

fn placeholder(name: &str) -> String {
    format!("${{{name}}}")
}

/// Variables referenced by `s`.
pub fn references(s: &str) -> Vec<String> {
    PLACEHOLDER
        .captures_iter(s)
        .map(|c| c[1].to_string())
        .collect()
}

struct Redactor<'a> {
    base: &'a str,
    captured: BTreeMap<String, String>,
}

impl Redactor<'_> {
    fn whole(&mut self, path: &[String], secret: &str) -> String {
        let refs: Vec<&str> = std::iter::once(self.base)
            .chain(path.iter().map(String::as_str))
            .collect();
        let name = env_name(&refs);
        self.captured.insert(name.clone(), secret.to_string());
        placeholder(&name)
    }

    fn url(&mut self, path: &[String], s: &str) -> String {
        let mut out = s.to_string();
        if let Some(c) = URL_USERINFO.captures(&out.clone()) {
            let pw = c[2].to_string();
            if !PLACEHOLDER.is_match(&pw) {
                let mut p = path.to_vec();
                p.push("password".into());
                let ph = self.whole(&p, &pw);
                out = format!(
                    "{}:{}@{}",
                    &c[1],
                    ph,
                    &out[c.get(0).map_or(0, |m| m.end())..]
                );
            }
        }
        let snapshot = out.clone();
        let mut result = String::with_capacity(out.len());
        let mut last = 0;
        for c in URL_QUERY.captures_iter(&snapshot) {
            let value = c.get(3).unwrap();
            if value.as_str().is_empty() || PLACEHOLDER.is_match(value.as_str()) {
                continue;
            }
            let mut p = path.to_vec();
            p.push(format!("query_{}", c[2].to_ascii_lowercase()));
            let ph = self.whole(&p, value.as_str());
            result.push_str(&snapshot[last..value.start()]);
            result.push_str(&ph);
            last = value.end();
        }
        result.push_str(&snapshot[last..]);
        result
    }

    fn walk(&mut self, v: &mut Value, path: &mut Vec<String>, forced: bool) {
        match v {
            Value::Mapping(m) => {
                let keys: Vec<Value> = m.keys().cloned().collect();
                for k in keys {
                    let Some(name) = k.as_str().map(str::to_string) else {
                        continue;
                    };
                    path.push(name.clone());
                    let secret = forced || is_secret_key(&name);
                    if let Some(child) = m.get_mut(&k) {
                        self.walk_child(child, path, secret, is_url_key(&name));
                    }
                    path.pop();
                }
            }
            Value::Sequence(items) => {
                for (i, item) in items.iter_mut().enumerate() {
                    path.push(i.to_string());
                    self.walk_child(item, path, forced, false);
                    path.pop();
                }
            }
            _ => {}
        }
    }

    fn walk_child(
        &mut self,
        child: &mut Value,
        path: &mut Vec<String>,
        secret: bool,
        url_key: bool,
    ) {
        match child {
            Value::String(s) if !s.is_empty() => {
                if secret {
                    if !PLACEHOLDER.is_match(s) {
                        *s = self.whole(path, s);
                    }
                } else if url_key {
                    *s = self.url(path, s);
                }
            }
            Value::Mapping(_) | Value::Sequence(_) => self.walk(child, path, secret),
            _ => {}
        }
    }
}

/// Top-level fields of a section that hold a secret as a whole.
fn forced_fields(section: &str, entity: &Mapping) -> Vec<&'static str> {
    match section {
        "webhooks" => vec!["url"],
        "notifications"
            if entity.get("channel_type").and_then(Value::as_str) == Some("webhook") =>
        {
            vec!["config"]
        }
        _ => Vec::new(),
    }
}

/// Replace secrets in `entity` with `${ENV}` references. Returns the removed
/// values keyed by variable name.
pub fn redact(section: &str, key: &str, entity: &mut Mapping) -> BTreeMap<String, String> {
    let base = env_name(&[section, key]);
    let forced = forced_fields(section, entity);
    let mut r = Redactor {
        base: base.strip_prefix("VOLTRUS_").unwrap_or(&base),
        captured: BTreeMap::new(),
    };
    let keys: Vec<Value> = entity.keys().cloned().collect();
    for k in keys {
        let Some(name) = k.as_str().map(str::to_string) else {
            continue;
        };
        let force = forced.contains(&name.as_str());
        let secret = force || is_secret_key(&name);
        let mut path = vec![name.clone()];
        if let Some(child) = entity.get_mut(&k) {
            r.walk_child(child, &mut path, secret, is_url_key(&name));
        }
    }
    r.captured
}

struct Resolver<'a> {
    env: &'a dyn Fn(&str) -> Option<String>,
    fallback: &'a BTreeMap<String, String>,
    missing: Vec<String>,
}

impl Resolver<'_> {
    fn string(&mut self, s: &mut String) {
        if !PLACEHOLDER.is_match(s) {
            return;
        }
        let mut out = String::with_capacity(s.len());
        let mut last = 0;
        for c in PLACEHOLDER.captures_iter(s) {
            let m = c.get(0).unwrap();
            out.push_str(&s[last..m.start()]);
            match (self.env)(&c[1]).or_else(|| self.fallback.get(&c[1]).cloned()) {
                Some(v) => out.push_str(&v),
                None => {
                    self.missing.push(c[1].to_string());
                    out.push_str(m.as_str());
                }
            }
            last = m.end();
        }
        out.push_str(&s[last..]);
        *s = out;
    }

    fn walk(&mut self, v: &mut Value, forced: bool) {
        match v {
            Value::Mapping(m) => {
                let keys: Vec<Value> = m.keys().cloned().collect();
                for k in keys {
                    let Some(name) = k.as_str().map(str::to_string) else {
                        continue;
                    };
                    let secret = forced || is_secret_key(&name);
                    if let Some(child) = m.get_mut(&k) {
                        self.child(child, secret, is_url_key(&name));
                    }
                }
            }
            Value::Sequence(items) => {
                for item in items {
                    self.child(item, forced, false);
                }
            }
            _ => {}
        }
    }

    fn child(&mut self, child: &mut Value, secret: bool, url_key: bool) {
        match child {
            Value::String(s) if secret || url_key => self.string(s),
            Value::Mapping(_) | Value::Sequence(_) => self.walk(child, secret),
            _ => {}
        }
    }
}

/// Substitute `${ENV}` references in secret-bearing positions: the process
/// environment first, then `fallback` (the values of the entity being
/// replaced). Returns the variables that could not be resolved.
pub fn resolve(
    section: &str,
    entity: &mut Mapping,
    env: &dyn Fn(&str) -> Option<String>,
    fallback: &BTreeMap<String, String>,
) -> Vec<String> {
    let forced = forced_fields(section, entity);
    let mut r = Resolver {
        env,
        fallback,
        missing: Vec::new(),
    };
    let keys: Vec<Value> = entity.keys().cloned().collect();
    for k in keys {
        let Some(name) = k.as_str().map(str::to_string) else {
            continue;
        };
        let secret = forced.contains(&name.as_str()) || is_secret_key(&name);
        if let Some(child) = entity.get_mut(&k) {
            r.child(child, secret, is_url_key(&name));
        }
    }
    r.missing.sort();
    r.missing.dedup();
    r.missing
}

/// Placeholders left in secret-bearing positions of a doc entity, for
/// validation warnings.
pub fn pending_references(section: &str, entity: &Mapping) -> Vec<String> {
    let mut copy = entity.clone();
    let none: BTreeMap<String, String> = BTreeMap::new();
    resolve(section, &mut copy, &|_| None, &none)
}
