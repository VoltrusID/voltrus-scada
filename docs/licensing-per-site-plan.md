# Per-site licence and annual support: plan

Status: core groundwork shipped (SCADA-4). Issuer and add-on work below is
planned, not built. Items marked **DECISION** are for the owner; no final prices
are set here.

## 1. Rule that does not change

Core never limits a Core feature on licence state (see [open-core.md](open-core.md)).
That covers the new fields too: a lapsed support contract blocks nothing in Core
and nothing already licensed. Support state is informational.

## 2. The new model

| Part | What it is |
|---|---|
| Per-site deployment licence | Unlocks the add-on tier (Integrator or Enterprise) for **one site / project**. Priced per project. |
| Annual support and maintenance | Recurring, 20-25% of the licence price per year. Covers updates and support for that site. |
| Managed multi-site monitoring (optional) | For fleet owners: Voltrus-run monitoring across their sites, sold separately on top of per-site licences. |

Core stays free, unlimited, no key.

## 3. Core groundwork (shipped in this change)

* `LicenseStatus::Valid` gains `site: Option<LicenseSite{id,name}>` and
  `support_until: Option<i64>` (Unix seconds, `None` = no contract). Both are
  `#[serde(default)]`, and omitted from JSON when `None`, so old statuses
  deserialize and old clients see the old shape. `LicenseStatus` now derives
  `Deserialize`.
* `SupportState` (`active`, `lapsed`, `none`) is derived against now:
  `active` while `now < support_until`, `lapsed` from that second on, `none`
  without a contract or when not `Valid`.
* `license_activation` table and `LicenseActivationRow` gain `site_id`,
  `site_name`, `support_until` (idempotent `ALTER TABLE` migration; old rows read
  back NULL).
* `GET /api/license/status` adds `support` (always) plus `site` / `support_until`
  when present. `GET /api/v1/system/features` adds a `license` object
  `{tier, site, support_until, support}`.
* `Extension::resolve_license` is the hook: the add-on fills `site` and
  `support_until` from the signed blob; Core only reports them.
* Web footer and native navbar tooltip / sign-in card show the site name and
  `Support until <date>` or `Support lapsed <date> — renew to keep receiving
  updates`; nothing when there is no contract.

## 4. Signed activation blob: fields to add

Added to the blob signed by the issuer and verified by the add-on:

| Field | Type | Semantics |
|---|---|---|
| `site_id` | string | Stable id of the licensed site (issuer-side UUID). |
| `site_name` | string | Display name, shown in the UI. Not security relevant. |
| `support_until` | int, Unix s, optional | End of support contract. Absent/NULL = none (lifetime, grandfathered). |

Semantics of `support_until` (proposal for the add-on's policy):

* Lapse never disables a feature and never blocks Core.
* The add-on keeps working on **every version released on or before
  `support_until`** (perpetual-fallback model). Each build embeds its release
  date; the add-on compares it with `support_until`.
* A build released after `support_until` runs as the lower of what the key
  entitles it to; the UI shows `Support lapsed ...` and asks to renew. **DECISION:**
  whether a post-lapse build degrades to Community add-on features (strict) or
  only shows the notice and keeps working (soft). Recommend soft for the first
  release, strict later only if renewals lag.
* Renewal re-issues the blob with a later `support_until`; no reinstall.
* Blobs without the fields stay valid exactly as before.

## 5. Issuer (voltrus-console, D1 `license_bindings` and activation worker)

1. Add columns: `site_id TEXT`, `site_name TEXT`, `support_until INTEGER`
   (NULL allowed), plus an index on `site_id`.
2. Binding: per-site binding **alongside** the machine fingerprint, not instead
   of it: one key = one site, with a `device_limit` per site (number of machines
   that may activate for that site, e.g. 2 for a primary plus standby). Replacing
   fingerprints outright would break hot-standby and reinstalls. **DECISION:** the
   default machine count per site.
3. Activation worker: include the three fields in the signed blob; reject
   activation for a different `site_id` once bound; allow the owner to rename
   `site_name` from the console without re-binding.
4. Renewal: support is a **Polar subscription** per site (annual). Webhook
   `subscription.active` / `order.paid` sets `support_until` = period end;
   `subscription.canceled` / `revoked` leaves `support_until` untouched (it simply
   lapses). The add-on re-fetches the blob on its existing refresh cycle.
5. Licence purchase (one-time per site) and support (subscription) are two Polar
   products per tier; the first-year support is bundled at checkout.
6. Admin: console view per site: licence, binding count, `support_until`, renewal
   link.

## 6. Grandfathering lifetime buyers

* Keep perpetual validity: lifetime keys stay `Valid` forever, `support_until`
  NULL, UI shows no support line. Nothing they own is taken away.
* Upgrade credit (proposal, **DECISION** for the owner to confirm):
  * Credit = 100% of the lifetime price actually paid, one-time, applied to the
    first per-site licence **plus** its first-year support (not to later renewals).
  * Usable for any one site, any tier at or above the tier bought; a downgrade
    keeps the credit as store credit for support only.
  * Valid 12 months from the launch of per-site licensing, then lapses.
  * A lifetime holder who never converts keeps the lifetime key as is, and is not
    entitled to updates beyond what the key was sold with. **DECISION:** whether
    that means "all versions" (generous) or "up to the last version before
    per-site launch" (recommended: version fence, announced upfront).
* Monthly subscribers: keep the plan until they cancel; offer conversion at the
  per-site price with the unused month credited.

## 7. Pricing copy (voltrus-landing)

Files: `src/data/products.ts` (`scadaTiers`: today Integrator 249 lifetime / 19
monthly, Enterprise 999 / 79; `scadaPaidFrom` derives from `scadaTiers[1].lifetime`),
the pricing page and the generated `public/pricing.md` / `llms.txt` (whatever
`sync-pricing` emits; edit the source, not the generated file).

Proposed shape of change: `lifetime` becomes `perSite` (one-time), add
`supportYearly`, keep `monthly` only while monthly subscribers exist. Copy:
"per site, one-time, plus optional annual support".

Options for the owner (numbers are **placeholders to choose from, not decisions**):

| | Option A: keep today's numbers per site | Option B: raise to project pricing | Option C: tiered by device bands |
|---|---|---|---|
| Integrator licence | 249 per site | **DECISION** | **DECISION** per band |
| Enterprise licence | 999 per site | **DECISION** | **DECISION** per band |
| Support / year | 20% (~50 / ~200) | 25% | 20-25% |
| Fits | existing customers, cheap to launch | integrators billing projects | big vs small sites |

Core copy must keep saying "no limits, no key". Any price copy uses `scadaPaidFrom`.

## 8. Rollout

1. Ship Core (this change) in the next release; nothing visible without a blob
   carrying the new fields.
2. Add-on: parse the fields, pass them through `Extension::resolve_license`,
   persist into `license_activation`, version-vs-`support_until` check (policy
   per section 4).
3. Issuer: migration, signed blob, Polar products and webhooks; staging
   end-to-end with a test key (active, lapsed, none).
4. Landing: switch pricing copy once the issuer can sell it; keep the old
   checkout links alive until then.
5. Grandfather: email lifetime and monthly buyers with their options and the
   credit deadline; give each lifetime buyer a console page showing their credit.
6. Comms: changelog and a plain statement: Core is unchanged and free; nothing
   already bought stops working; support lapse only ends updates and support.
7. Success checks: no support ticket where a lapsed site lost a feature;
   renewal rate after the first anniversary.

## 9. Open decisions

Prices (section 7); soft vs strict after lapse (4); machines per site (5);
credit rule details and lifetime version fence (6); whether managed multi-site
monitoring is priced per site or per fleet.
