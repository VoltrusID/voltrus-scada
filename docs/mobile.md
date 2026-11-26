# The Voltrus SCADA phone app

Operators pair their phones with a site and then see its active alarms, acknowledge
them, and watch a few key tags. With an Integrator or Enterprise licence the phone
also gets a push notification for every new alarm, wherever it is.

| | Core | Integrator, Enterprise |
|---|:-:|:-:|
| Pair phones, alarm list, acknowledge, key tags, on the site's network (LAN, VPN, or a public address) | yes | yes |
| Push notification for new alarms, cleared when someone acknowledges | | yes |
| Use from outside the site's network, through the Voltrus push relay | | yes |

A site takes up to 50 paired phones.

The HMI itself also works in a phone's browser: screens show as the phone
operator layout, and it can be installed on the home screen with the site's name
([designer.md](designer.md#phase-5-mobile-layouts-and-pwa-built)). The browser
does not receive alarm notifications; that is this app's job.

## Pairing a phone

Open **Data → Phone app** in the HMI and click **Show QR code**. In the app, tap
**Add site** and scan the code. The code works once and for 10 minutes. The phone
then acts as you: it sees what you see, and it can acknowledge alarms when your
role allows it (operator, admin). A viewer's phone only watches.

On the site's network the app can also sign in with a username and password
instead of a code.

The QR code holds the addresses the phone tries, in this order:

1. the addresses an admin lists under **Settings, Other addresses of this site**
   (for example the address on the plant VPN),
2. the address the HMI page was opened with, unless that is `localhost` or
   another loopback address,
3. the Voltrus push relay, when the site is licensed and connected to it.

If the page is open on the server itself and no other address is listed, the code
has nowhere to point the phone to: open the HMI from another computer, or add the
site's address in the settings.

Under **My phones** each user sees and removes their own phones. Admins see every
phone under **All paired phones**. A removed phone stops working at once, and so
does the phone of a user who is disabled or deleted. The audit log records every
pairing and removal (`mobile_pair`, `mobile_unpair`).

## Settings (admins)

* **Site name on the phone.** Without one, the licence's site name is used, then
  the white-label title, then "Voltrus SCADA". The app also shows the white-label
  logo and accent colour.
* **Show the alarm text in notifications.** Off: the notification only says that
  the site has a new alarm, so tag names and values do not pass through Apple's
  or Google's push services; the app fetches the details when it opens.
* **Key tags.** Up to 24 tags shown with live values on the phone's site screen.
* **Other addresses of this site.** Up to 4.

## How pushes are sent

The server watches new alarms and their state changes. For each new alarm it sends
one push to every paired phone that has notifications on and whose lowest
priority (set in the app: info, warning or critical) is at or below the alarm's.
When an alarm is acknowledged, shelved or resolved, the phones get an update that
clears the notification.

Alarm floods are spread out: after 20 new-alarm pushes in a minute, the rest of
that minute become one "N more alarms" push, and a site sends at most 600
pushes an hour. A standby node of a hot-standby pair sends nothing.

Core does not contact Voltrus servers. The connection to the push relay ([protocol](mobile-relay.md)) belongs
to the Enterprise add-on, which registers a `MobilePushSender`
(`src/edition.rs`); without one, nothing is pushed. The relay checks the site's
licence and passes the pushes to Apple and Google through Expo's push service.
Push notifications are not guaranteed delivery: keep Telegram, email or a
siren for alarms that must not be missed.

## API

A paired phone sends `Authorization: Bearer vdt_…` and is treated as its user:
the same routes, the same roles. The token does not expire; removing the phone
revokes it. A refused token is answered with 401 and `"code": "device_revoked"`.

| Route | Who | |
|---|---|---|
| `POST /api/v1/mobile/pairings` | a signed-in person (not an API key or a phone) | A QR code: `link`, `payload` (`site`, `urls`, `relay`, `code`), `expires_at` |
| `POST /api/v1/mobile/pair` | anyone with a code | `{code, name, platform, push_token?, lang?, min_priority?}` → `{token, device, username, role, site, urls, relay?}` (201). An unknown, used or expired code: 401 `pairing_invalid`. A full site: 409 `device_limit` |
| `POST /api/v1/mobile/devices` | a signed-in person | Pair the phone that signed in with this session (same body without `code`) |
| `GET /api/v1/mobile/devices` | any user | One's own phones; `?all=true` every phone (admins) |
| `DELETE /api/v1/mobile/devices/{id}` | owner or admin | Remove a phone |
| `GET`, `PATCH`, `DELETE /api/v1/mobile/device` | a phone | The phone itself: read; change `push_token` (`null` turns pushes off), `min_priority`, `lang`, `name`; remove itself |
| `GET /api/v1/mobile/summary` | any user | Site name, branding (`logo` is a `data:` URL or a link), the user's rights, alarm counts, key tag values, and `urls` and `relay` as in the QR code, so a phone learns new addresses and a later licence |
| `GET`, `PUT /api/v1/mobile/settings` | admin, demo, integration | The settings above |

The app uses the existing routes for the rest: `GET /api/v1/alarms`,
`POST /api/v1/alarms/{id}/acknowledge` (recorded with origin `mobile`; when an
add-on requires an electronic signature for it, the phone gets 403
`signature_not_possible` and the alarm is acknowledged at the HMI).

The QR code is the text `voltrus-scada://pair?d=` followed by the payload as
base64url JSON:

```json
{"v": 1, "site": "Pabrik A", "urls": ["http://10.8.0.5:3000"],
 "relay": {"url": "https://push.voltrus.id", "site_id": "…"}, "code": "…"}
```
