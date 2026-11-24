//! Where the server's own HTTP requests to user-configured URLs may go:
//! outbound webhooks (and their test button), alarm notification webhooks and
//! flow `http_out` nodes. Every such request goes through [`client`], which
//! follows no redirects and resolves host names through [`PolicyResolver`],
//! so a name that resolves to a refused address is refused too.
//!
//! Always refused: link-local addresses (169.254.0.0/16 with the cloud
//! metadata service at 169.254.169.254, fe80::/10), unspecified and multicast
//! addresses. Loopback (127.0.0.0/8, ::1) and this machine's own interface
//! addresses (LAN and public) only with `allow_loopback`. IPv4 addresses
//! carried in IPv6 (mapped, compatible, NAT64, 6to4) are judged as IPv4.
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
    /// This machine: 127.0.0.0/8, ::1 and its own interface addresses
    /// (default false).
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

/// The IPv4 address an IPv6 address carries, when it is one of the forms
/// that reach an IPv4 host: IPv4-mapped `::ffff:a.b.c.d`, IPv4-compatible
/// `::a.b.c.d`, NAT64 `64:ff9b::a.b.c.d` and 6to4 `2002:aabb:ccdd::/48`.
fn embedded_v4(v6: Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(v4) = v6.to_ipv4_mapped() {
        return Some(v4);
    }
    let seg = v6.segments();
    let low = Ipv4Addr::from((u32::from(seg[6]) << 16) | u32::from(seg[7]));
    if seg[..6] == [0, 0, 0, 0, 0, 0] && v6 != Ipv6Addr::LOCALHOST && !v6.is_unspecified() {
        return Some(low); // IPv4-compatible
    }
    if seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return Some(low); // NAT64 well-known prefix
    }
    if seg[0] == 0x2002 {
        return Some(Ipv4Addr::from(
            (u32::from(seg[1]) << 16) | u32::from(seg[2]),
        )); // 6to4
    }
    None
}

/// Why `ip` is refused under `policy`, or `None` when it is allowed.
pub fn refusal(ip: IpAddr, policy: OutboundPolicy) -> Option<&'static str> {
    // An IPv4 address carried in IPv6 is judged as that IPv4 address.
    let ip = match ip {
        IpAddr::V6(v6) => embedded_v4(v6).map(IpAddr::V4).unwrap_or(ip),
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

/// This machine's own addresses (every interface, LAN and public ones
/// included), read at most once a minute.
fn own_addresses() -> Vec<IpAddr> {
    static CACHE: std::sync::Mutex<Option<(std::time::Instant, Vec<IpAddr>)>> =
        std::sync::Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, addrs)) = cache.as_ref()
        && at.elapsed() < Duration::from_secs(60)
    {
        return addrs.clone();
    }
    let nets = sysinfo::Networks::new_with_refreshed_list();
    let addrs: Vec<IpAddr> = nets
        .values()
        .flat_map(|data| data.ip_networks().iter().map(|n| n.addr))
        .collect();
    *cache = Some((std::time::Instant::now(), addrs.clone()));
    addrs
}

/// [`refusal`] on this machine: without `allow_loopback`, the machine's own
/// interface addresses are refused like loopback (a request to its LAN or
/// public address reaches the same services as one to 127.0.0.1).
pub fn refusal_here(ip: IpAddr, policy: OutboundPolicy) -> Option<&'static str> {
    refusal(ip, policy).or_else(|| {
        let ip = match ip {
            IpAddr::V6(v6) => embedded_v4(v6).map(IpAddr::V4).unwrap_or(ip),
            v4 => v4,
        };
        (!policy.allow_loopback && own_addresses().contains(&ip))
            .then_some("this machine's own address is not allowed (outbound_http.allow_loopback)")
    })
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
    match refusal_here(ip, policy()) {
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
                .filter(|a| refusal_here(a.ip(), policy).is_none())
                .collect();
            if allowed.is_empty() {
                let why = addrs
                    .first()
                    .and_then(|a| refusal_here(a.ip(), policy))
                    .unwrap_or("no address");
                return Err(format!("{host}: {why}").into());
            }
            Ok(Box::new(allowed.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// The HTTP client for user-configured URLs: no redirects, no proxy from
/// the environment (`HTTP_PROXY`/`HTTPS_PROXY` would resolve the host
/// itself, past [`PolicyResolver`]), host names resolved through
/// [`PolicyResolver`]. Call [`check_url`] before each request for URLs with
/// an IP address as host.
pub fn client(timeout: Option<Duration>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .dns_resolver(Arc::new(PolicyResolver));
    if let Some(t) = timeout {
        builder = builder.timeout(t);
    }
    // Never fall back to a default client: it would have none of the rules.
    builder
        .build()
        .expect("the HTTP client for outbound requests could not be built")
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
    fn ipv4_carried_in_ipv6_is_judged_as_ipv4() {
        let default = OutboundPolicy::default();
        for a in [
            "::169.254.169.254",  // IPv4-compatible
            "64:ff9b::a9fe:a9fe", // NAT64 of 169.254.169.254
            "2002:a9fe:a9fe::1",  // 6to4 of 169.254.169.254
            "64:ff9b::7f00:1",    // NAT64 of 127.0.0.1
            "2002:7f00:1::1",     // 6to4 of 127.0.0.1
        ] {
            assert!(refusal(ip(a), default).is_some(), "{a}");
        }
        assert!(refusal(ip("2001:db8::1"), default).is_none());
    }

    /// This machine's address on a non-loopback interface, found by asking
    /// the OS which source address it would use (no packet is sent).
    fn lan_ip() -> Option<IpAddr> {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
        socket.connect("10.255.255.255:9").ok()?;
        let ip = socket.local_addr().ok()?.ip();
        (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
    }

    #[test]
    fn this_machines_own_addresses_count_as_loopback() {
        let Some(own) = lan_ip() else {
            return; // no non-loopback interface here
        };
        let default = OutboundPolicy::default();
        assert!(refusal_here(own, default).is_some(), "{own}");
        let local = OutboundPolicy {
            allow_private_networks: true,
            allow_loopback: true,
        };
        if refusal(own, local).is_none() {
            assert!(refusal_here(own, local).is_none());
        }
    }

    #[tokio::test]
    async fn the_client_follows_no_redirect() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let router = axum::Router::new().route(
                "/hook",
                axum::routing::post(|| async {
                    (
                        axum::http::StatusCode::FOUND,
                        [("location", "http://169.254.169.254/")],
                        "",
                    )
                }),
            );
            axum::serve(listener, router).await.unwrap();
        });
        // check_url is not asked here (loopback is refused by default); the
        // client alone must hand back the redirect instead of following it.
        let resp = client(Some(Duration::from_secs(5)))
            .post(format!("http://127.0.0.1:{port}/hook"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 302);
    }

    #[test]
    fn urls_with_refused_ip_hosts_fail_the_check() {
        assert!(
            check_url("http://2130706433/").is_err(),
            "decimal 127.0.0.1"
        );
        assert!(check_url("http://169.254.169.254/latest/meta-data/").is_err());
        assert!(check_url("http://[::1]:8080/").is_err());
        assert!(check_url("ftp://example.com/").is_err());
        assert!(check_url("https://example.com/hook").is_ok());
    }
}
