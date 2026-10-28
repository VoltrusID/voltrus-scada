# Open core: what is free and what is paid

Voltrus SCADA is open core. This repository is the **Core**, licensed under
[AGPL-3.0-or-later](../LICENSE). A separate, closed-source **Enterprise add-on**
adds features that integrators and regulated sites pay for. Official Voltrus
binaries are Core plus the add-on.

Pricing and plans: <https://voltrus.id/voltrus-scada>.

## What you get in Core

Everything one site needs to run, with no device, tag or retention limits, no
licence key and no calls to Voltrus servers:

| Area | In Core |
|---|---|
| Runtime | Single binary server, SQLite historian, compression and retention tiers, backup and restore |
| Protocols | Modbus TCP, MQTT (embedded broker), OPC UA, Siemens S7, EtherNet/IP, DNP3, BACnet/IP |
| HMI | Grid, canvas, 3D digital twin and 2D SCADA editor dashboards, widgets, screens and layouts |
| Alarms | Priorities, shelving, deadband, on/off delay, acknowledgement, analytics |
| Data | Historian and trends, SPC (control, histogram, run charts), annotations, CSV and Excel import and export |
| Automation | Flow engine, scripts, recipes, command queue with read-back, simulation mode |
| Integration | REST API (OpenAPI), Python and TypeScript SDKs, Prometheus, Grafana and BI endpoints, outbound webhooks, scheduled data egress |
| Operations | OEE and downtime, shift handover, asset inventory with NIS2 export, EDR and hardware compatibility tools, AI copilot (bring your own key) |
| Security | Local accounts (Argon2), role-based access control, API keys, rate limiting, audit log, CSRF and security headers |

## What the Enterprise add-on adds

Unlocked by a licence key bound to one site. Two plans:

| Feature | Integrator | Enterprise |
|---|:-:|:-:|
| White-label branding and theme editor | yes | yes |
| Scheduled PDF reports and SMS alarm notifications | yes | yes |
| Multi-site hub | yes | yes |
| Single sign-on (Active Directory, Azure AD, Okta, SAML 2.0, OIDC, LDAP) | | yes |
| 21 CFR Part 11 mode (electronic signatures, record checksums, policy) | | yes |
| IEC 62443 security audit mode (event log, thresholds, export) | | yes |
| Hot standby with automatic failover and config sync | | yes |
| RTSP video cameras | | yes |
| Support | standard | priority SLA |

Licence keys carry a tier and, for monthly plans, an expiry. A lapsed monthly key
keeps its features for a 7 day grace period and then the install falls back to
Core. **An invalid, expired or missing licence never limits Core.**
A per-site key may also carry the site it is issued for and an annual support
contract (`site`, `support_until`, derived `support`: `active`, `lapsed`, `none`
on `/api/license/status`); a lapsed contract is informational only. See
[licensing-per-site-plan.md](licensing-per-site-plan.md).

## How the split is built

The core is both a library and the `voltrus` binary (one Cargo package).
`src/edition.rs` defines the extension seam:

```rust
voltrus::run(std::sync::Arc::new(MyExtension)).await
```

An [`Extension`](../src/edition.rs) can contribute HTTP routes (public, any user,
operator, admin), start background tasks, handle CLI subcommands, validate bearer
tokens from an identity provider, add sign-in providers, supply SMS delivery and
resolve a licence into a `LicenseTier`. A sign-in provider (SAML, OIDC, LDAP) opens
the user's session with `api::auth::issue_session`, the function local login uses,
so every session has the same cookies and lifetime. The `voltrus` binary in this repository
runs with `edition::Community`: no add-on, no licence, nothing gated.

The Enterprise add-on is a separate crate that depends on this one and calls
`voltrus::run` with its own extension. It is not in this repository.

Things worth knowing:

* The **storage layer and data models are public**, including tables and
  columns used by add-on features (hub sites, cameras, report definitions,
  electronic signatures, security events). The behaviour built on them
  (handlers, rendering, signing, failover) lives in the add-on.
* `GET /api/license/status` is served by Core and always reports a tier and
  11 boolean flags (`white_label`, `theme_full`, `pdf_reporting`,
  `sms_notifications`, `multi_site`, `sso`, `sso_saml`, `cfr_part_11`,
  `iec_62443`, `ha`, `rtsp_video`). In Core all are `false`.
* The web client has the same seam: `client/src/edition/` loads an optional
  enterprise module (Vite alias `@voltrus/enterprise-client`) and falls back to a
  Community stub. Core UI works when the add-on endpoints return 404.
* Endpoints that exist only in the add-on are not in the OpenAPI document
  generated from this repository, so the SDKs generated from it cover Core only.
* The experimental Voltrus Gateway client integration (gRPC to a separate
  gateway process) is not part of Core.

## Licence and contributing

* Code in this repository: AGPL-3.0-or-later, see [LICENSE](../LICENSE) and
  [NOTICE](../NOTICE). The client SDKs under `sdks/` are MIT.
* If you run a modified version as a network service you must offer its source
  to its users (AGPL section 13). Plain use, including commercial use inside your
  company, needs nothing from us.
* Contributions need a DCO sign-off and a licence grant that keeps dual licensing
  possible, see [CONTRIBUTING.md](../CONTRIBUTING.md).
* "Voltrus" and the logo are trademarks of JoyoDigitama. The AGPL does not grant
  trademark rights. Forks must use a different name.
