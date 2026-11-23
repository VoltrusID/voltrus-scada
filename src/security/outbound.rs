//! Where the server's own HTTP requests to user-configured URLs may go:
//! outbound webhooks (and their test button), alarm notification webhooks and
//! flow `http_out` nodes. Every such request goes through [`client`], which
//! follows no redirects and resolves host names through [`PolicyResolver`],
//! so a name that resolves to a refused address is refused too.
//!
//! Always refused: link-local addresses (169.254.0.0/16 with the cloud
//! metadata service at 169.254.169.254, fe80::/10), unspecified and multicast
//! addresses. Loopback (127.0.0.0/8, ::1) only with `allow_loopback`.
//! Private ranges (10/8, 172.16/12, 192.168/16, 100.64/10, fc00::/7) are
//! allowed by default, since a plant posts to hosts on its own network; set
//! `allow_private_networks: false` on a server reachable from the internet
//! (a public demo) so its users cannot reach the network behind it.
//!
//! Config (`outbound_http:` in config.yaml), applied with [`set_policy`] at
//! startup.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::Deserialize;

/// `outbound_http:` in the config file.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub struct OutboundPolicy {
    /// Private and carrier-grade NAT ranges (default true).
    #[serde(default = "default_true")]
    pub allow_private_networks: bool,
    /// This machine: 127.0.0.0/8 and ::1 (default false).
    #[serde(default)]
    pub allow_loopback: bool,
}

fn default_true() -> bool {
    true
}

impl Default for OutboundPolicy {
    fn default() -> Self {
        Self {
            allow_private_networks: true,
            allow_loopback: false,
        }
    }
}

static POLICY: RwLock<OutboundPolicy> = RwLock::new(OutboundPolicy {
    allow_private_networks: true,
    allow_loopback: false,
});

/// Apply the config's policy (called once at startup).
pub fn set_policy(policy: OutboundPolicy) {
    *POLICY.write().unwrap_or_else(|e| e.into_inner()) = policy;
}

/// The policy in force.
pub fn policy() -> OutboundPolicy {
    *POLICY.read().unwrap_or_else(|e| e.into_inner())
}

fn is_private_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_private() || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10
}

/// Why `ip` is refused under `policy`, or `None` when it is allowed.
pub fn refusal(ip: IpAddr, policy: OutboundPolicy) -> Option<&'static str> {
    // An IPv4 address written as IPv6 (::ffff:a.b.c.d) is judged as IPv4.
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    };
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_link_local() {
                Some("a link-local address (cloud metadata) is never allowed")
            } else if v4.is_unspecified() || v4.is_broadcast() || v4.is_multicast() {
                Some("not a unicast address")
            } else if v4.is_loopback() {
                (!policy.allow_loopback)
                    .then_some("loopback is not allowed (outbound_http.allow_loopback)")
            } else if is_private_v4(v4) {
                (!policy.allow_private_networks).then_some(
                    "private networks are not allowed (outbound_http.allow_private_networks)",
                )
            } else {
                None
            }
        }
        IpAddr::V6(v6) => {
            let seg0 = v6.segments()[0];
            if (seg0 & 0xffc0) == 0xfe80 {
                Some("a link-local address is never allowed")
            } else if v6.is_unspecified() || v6.is_multicast() {
                Some("not a unicast address")
            } else if v6 == Ipv6Addr::LOCALHOST {
                (!policy.allow_loopback)
                    .then_some("loopback is not allowed (outbound_http.allow_loopback)")
            } else if (seg0 & 0xfe00) == 0xfc00 {
                (!policy.allow_private_networks).then_some(
                    "private networks are not allowed (outbound_http.allow_private_networks)",
                )
            } else {
                None
            }
        }
    }
}

/// Check a URL before a request: http or https, and when the host is an IP
/// address, an allowed one. Host names are checked when they resolve.
pub fn check_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("URL must start with http:// or https://".into());
    }
    let host = parsed.host_str().ok_or("URL has no host")?;
    // The parser normalises IPv4 forms (decimal, octal) to dotted quads and
    // keeps IPv6 in brackets.
    let Ok(ip) = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
    else {
        return Ok(());
    };
    match refusal(ip, policy()) {
        Some(why) => Err(format!("{ip}: {why}")),
        None => Ok(()),
    }
}

/// Resolves host names with the system resolver and drops the addresses the
/// policy refuses; a name left with none fails to resolve.
#[derive(Debug, Default)]
pub struct PolicyResolver;

impl reqwest::dns::Resolve for PolicyResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let policy = policy();
            let addrs: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let allowed: Vec<SocketAddr> = addrs
                .iter()
                .copied()
                .filter(|a| refusal(a.ip(), policy).is_none())
                .collect();
            if allowed.is_empty() {
                let why = addrs
                    .first()
                    .and_then(|a| refusal(a.ip(), policy))
                    .unwrap_or("no address");
                return Err(format!("{host}: {why}").into());
            }
            Ok(Box::new(allowed.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// The HTTP client for user-configured URLs: no redirects, host names
/// resolved through [`PolicyResolver`]. Call [`check_url`] before each
/// request for URLs with an IP address as host.
pub fn client(timeout: Option<Duration>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(Arc::new(PolicyResolver));
    if let Some(t) = timeout {
        builder = builder.timeout(t);
    }
    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn metadata_and_link_local_are_always_refused() {
        let open = OutboundPolicy {
            allow_private_networks: true,
            allow_loopback: true,
        };
        for a in [
            "169.254.169.254",
            "169.254.1.1",
            "fe80::1",
            "::ffff:169.254.169.254",
            "0.0.0.0",
        ] {
            assert!(refusal(ip(a), open).is_some(), "{a}");
        }
    }

    #[test]
    fn loopback_and_private_follow_the_policy() {
        let default = OutboundPolicy::default();
        assert!(refusal(ip("127.0.0.1"), default).is_some());
        assert!(refusal(ip("::1"), default).is_some());
        assert!(refusal(ip("192.168.1.10"), default).is_none());
        assert!(refusal(ip("10.0.0.5"), default).is_none());
        assert!(refusal(ip("203.0.113.8"), default).is_none());

        let strict = OutboundPolicy {
            allow_private_networks: false,
            allow_loopback: false,
        };
        for a in [
            "192.168.1.10",
            "10.0.0.5",
            "172.16.0.1",
            "100.64.0.1",
            "fd00::1",
        ] {
            assert!(refusal(ip(a), strict).is_some(), "{a}");
        }
        assert!(refusal(ip("203.0.113.8"), strict).is_none());

        let local = OutboundPolicy {
            allow_private_networks: true,
            allow_loopback: true,
        };
        assert!(refusal(ip("127.0.0.1"), local).is_none());
    }

    #[test]
    fn urls_with_refused_ip_hosts_fail_the_check() {
        assert!(check_url("http://169.254.169.254/latest/meta-data/").is_err());
        assert!(check_url("http://[::1]:8080/").is_err());
        assert!(check_url("ftp://example.com/").is_err());
        assert!(check_url("https://example.com/hook").is_ok());
    }
}
