# Push relay protocol (v1)

The Voltrus push relay (`https://push.voltrus.id`, repository `voltrus-push-relay`)
lets a site licensed for Integrator or Enterprise push alarms to paired phones,
and lets those phones reach the site from outside its network. Three parties
speak this protocol:

* **the site**: the Enterprise add-on's relay connector (`enterprise/src/mobile/`),
  which registers a `MobilePushSender` (`src/edition.rs`) in Core;
* **the relay**: a Cloudflare Worker with one Durable Object per site;
* **the app**: Voltrus SCADA for Android and iOS (`voltrus-scada-mobile`).

Core itself never connects to the relay.

## 1. The site's connection

The site keeps one WebSocket open:

```
GET wss://push.voltrus.id/v1/sites/connect
X-Voltrus-Activation: <the site's signed activation file, base64 (standard)>
X-Voltrus-Site-Secret: <secret from an earlier hello; absent the first time>
X-Voltrus-Version: <SCADA version>
```

The relay accepts the connection when:

1. the activation file's Ed25519 signature verifies against one of the activation
   public keys (the same list as `TRUSTED_PUBLIC_KEYS` in the Enterprise
   `licensing/activation.rs`; the file is `VOLACT\x01` followed by the JSON
   activation, see `verify_activation_signature`);
2. its `tier` is `integrator` or `enterprise`;
3. `expires_at` is absent or less than 14 days in the past;
4. the licence key is not revoked in the licence database;
5. the site secret matches the one stored for the site. A site without a stored
   secret gets a new one. A connection without the secret, or with a wrong one, is
   still accepted (and given a new secret) when its activation was issued later
   than the one the stored secret belongs to: reactivating the licence is how a
   reinstalled site recovers. Otherwise it is refused.

The **site id** is `activation.site_id` when present, otherwise the first 32 hex
characters of SHA-256 of the licence key. It is stable across reactivations.

A refused connection is answered before the upgrade with an HTTP error and JSON
`{"error": "...", "code": "..."}`; codes: `activation_invalid` (400/401),
`licence_tier` (403), `licence_expired` (403), `licence_revoked` (403),
`site_secret` (401). The site retries a refused connection after an hour; a
dropped connection with backoff from 1 s to 60 s.

Only one connection per site is kept: a new one closes the previous one
(close code 4000, "replaced").

All frames are JSON text. The first frame after the upgrade comes from the relay:

```json
{"type": "hello", "site_id": "…", "site_secret": "…", "limits": {"devices": 50, "pushes_per_hour": 600}}
```

`site_secret` is present only when a new one was issued; the site stores it and
sends it on every later connection.

### Site to relay

**push**: send one notification to each target.

```json
{"type": "push", "id": "<site-chosen id>", "push": { MobileAlarmPush }}
```

`push` is `MobileAlarmPush` from `src/edition.rs` as serialised by serde
(`event`, `alarm_id`, `site_name`, `source_key`, `source_name`, `severity`,
`priority`, `message`, `value`, `unit`, `actor`, `ts`, `details`, `held_back`,
`targets: [{device_id, platform, push_token, lang}]`). When `details` is false
the site blanks `source_key`, `source_name`, `message`, sets `value` to 0 and
`unit` to "" before sending, so alarm details never leave the site.

The relay answers:

```json
{"type": "push_result", "id": "<same id>", "deliveries": [{"device_id": 7, "error": null, "unregistered": false}]}
```

`deliveries` lists the targets that failed or are no longer registered (it may
be empty). A push over the site's hourly quota is answered with every target
failing, `error: "quota"`. Delivery receipts that arrive later are reported as:

```json
{"type": "unregistered", "devices": [{"device_id": 7, "push_token": "ExponentPushToken[…]"}]}
```

The site then clears that push token (`Storage::clear_mobile_push_token`).

**response**: the answer to a forwarded request (below).

```json
{"type": "response", "id": "<request id>", "status": 200, "headers": {"content-type": "application/json"}, "body": "<text>"}
```

**ping**: `{"type": "ping"}` every 30 s; the relay answers `{"type": "pong"}`.

### Relay to site

**request**: a phone's HTTP request, forwarded.

```json
{"type": "request", "id": "<relay id>", "method": "POST", "path": "/api/v1/alarms/12/acknowledge",
 "query": "status=active", "headers": {"authorization": "Bearer vdt_…", "content-type": "application/json", "accept-language": "id"}, "body": "<text or null>"}
```

The site runs it against its own router in process, as if it arrived on its HTTP
port from the relay, and answers with a `response` frame within 15 s. Only these
headers are forwarded: `authorization`, `content-type`, `accept-language`. Only
these routes are forwarded; both the relay and the site check the list and answer
anything else with 403 `{"code": "path_not_allowed"}`:

| Method | Path |
|---|---|
| POST | `/api/v1/mobile/pair` |
| GET, PATCH, DELETE | `/api/v1/mobile/device` |
| GET | `/api/v1/mobile/summary` |
| GET | `/api/v1/alarms` |
| GET | `/api/v1/alarms/{id}/journal` |
| POST | `/api/v1/alarms/{id}/acknowledge` |

Request bodies are limited to 64 KB, response bodies to 1 MB (larger: 502
`response_too_large`).

## 2. The phone's side

The app reaches a site through the relay with plain HTTPS:

```
<METHOD> https://push.voltrus.id/v1/sites/{site_id}/api/v1/…
Authorization: Bearer vdt_…
```

The relay answers itself with:

* 503 `{"code": "site_offline"}` when the site is not connected,
* 504 `{"code": "site_timeout"}` when the site did not answer within 15 s,
* 403 `{"code": "path_not_allowed"}`,
* 429 `{"code": "rate_limited"}` above 120 requests a minute per site and
  client address.

Otherwise it returns the site's status, `content-type` and body unchanged.

The app tries the site's direct addresses first (`urls` in the pairing payload)
with a 3 s timeout, and the relay after them. Pairing through the relay works the
same way: `POST …/v1/sites/{site_id}/api/v1/mobile/pair`.

## 3. Notifications

The relay sends pushes through the Expo push service (one access token, held by
the relay as a secret). For each target it builds one Expo message:

| `event` | Title | Body | Kind |
|---|---|---|---|
| `raised`, details on | `{site_name}: {priority}` (localised) | `{source_name}: {message} ({value} {unit})` | alert |
| `raised`, details off | `{site_name}` | "New alarm" / "Alarm baru" | alert |
| `summary` | `{site_name}` | "{held_back} more alarms" / "{held_back} alarm lagi" | alert |
| `acknowledged`, `resolved`, `shelved` | none | none | data only |
| `unshelved` | as `raised` | | alert |

Alert messages carry `sound: "default"`, `priority: "high"`,
`channelId: "alarms-critical"` for priority `critical` and `"alarms"` otherwise,
`categoryId: "alarm"` (the app registers an Acknowledge action for it), and on
iOS `interruptionLevel: "time-sensitive"`.

Every message carries `data`:

```json
{"v": 1, "site_id": "…", "alarm_id": 12, "event": "raised", "priority": "warning", "ts": 1764000000}
```

Data-only messages (`_contentAvailable: true`, no title or body) tell the app to
remove that alarm's notification.

Texts are Indonesian when the target's `lang` is `id`, English otherwise.

## 4. What the relay stores

Per site (D1): site id, licence key hash, tier, site name, site secret hash,
activation `issued_at`, first and last connection time. Per hour: push count.
Nothing about alarms is stored; push tokens pass through for each push and are
kept only while Expo receipts are pending (at most a day).
