import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  COMMUNITY_LICENSE,
  FEATURE_FLAG_KEYS,
  hasFeature,
  normalizeLicense,
  siteLabel,
  supportLabel,
  supportState,
} from '../src/edition/license.ts'
import { mergeTabs } from '../src/edition/tabs.ts'
import { ENTERPRISE_NS, registerLocales } from '../src/edition/locales.ts'
import community from '../src/edition/community.ts'

test('the licence model has exactly the 11 contract flags', () => {
  assert.deepEqual([...FEATURE_FLAG_KEYS].sort(), [
    'cfr_part_11', 'chat_notifications', 'ha', 'iec_62443', 'multi_site', 'pdf_reporting', 'rtsp_video',
    'sso', 'sso_saml', 'theme_full', 'white_label',
  ])
})

test('community licence has every flag off', () => {
  assert.equal(COMMUNITY_LICENSE.status, 'Community')
  assert.equal(COMMUNITY_LICENSE.tier, 'community')
  for (const k of FEATURE_FLAG_KEYS) assert.equal(COMMUNITY_LICENSE.feature_flags[k], false)
})

test('the Community stub exports a null module', () => {
  assert.equal(community, null)
})

test('normalizeLicense degrades garbage to Community', () => {
  for (const raw of [undefined, null, 42, 'Valid', [], {}]) {
    const info = normalizeLicense(raw)
    assert.equal(info.status, 'Community')
    assert.equal(info.tier, 'community')
    assert.equal(hasFeature(info, 'sso'), false)
  }
})

test('normalizeLicense accepts an enterprise status and keeps only contract flags', () => {
  const info = normalizeLicense({
    status: 'Valid',
    tier: 'enterprise',
    license_key: 'VLT-1234-5678',
    expires_at: null,
    days_remaining: null,
    grace: false,
    feature_flags: { sso: true, ha: true, white_label: 'yes', analytics: true, oee: true },
  })
  assert.equal(info.status, 'Valid')
  assert.equal(info.tier, 'enterprise')
  assert.equal(info.license_key, 'VLT-1234-5678')
  assert.equal(info.expires_at, null)
  assert.equal(info.grace, false)
  assert.equal(hasFeature(info, 'sso'), true)
  assert.equal(hasFeature(info, 'ha'), true)
  // Only literal `true` counts; legacy flags are dropped.
  assert.equal(hasFeature(info, 'white_label'), false)
  assert.deepEqual(Object.keys(info.feature_flags).sort(), [...FEATURE_FLAG_KEYS].sort())
})

test('normalizeLicense rejects legacy tiers and statuses it does not know', () => {
  const info = normalizeLicense({ status: 'Engineering', tier: 'professional', feature_flags: { pdf_reporting: true } })
  assert.equal(info.status, 'Community')
  assert.equal(info.tier, 'community')
  // Flags are still read as sent; the server decides what they mean.
  assert.equal(hasFeature(info, 'pdf_reporting'), true)
})

test('normalizeLicense keeps the monthly-key expiry fields', () => {
  const info = normalizeLicense({ status: 'Valid', tier: 'integrator', expires_at: '2027-01-01T00:00:00Z', days_remaining: 12, grace: true, feature_flags: {} })
  assert.equal(info.expires_at, '2027-01-01T00:00:00Z')
  assert.equal(info.days_remaining, 12)
  assert.equal(info.grace, true)
})

test('hasFeature treats a missing licence as Community', () => {
  assert.equal(hasFeature(null, 'ha'), false)
  assert.equal(hasFeature(undefined, 'ha'), false)
})

test('mergeTabs appends extra tabs into named groups and creates new ones', () => {
  const core = [
    { label: 'Data', items: [{ id: 'sources' }] },
    { label: 'Admin', items: [{ id: 'users' }] },
  ]
  const merged = mergeTabs(core, [
    { id: 'hub', group: 'Data' },
    { id: 'license', group: 'Admin' },
    { id: 'extra', group: 'Plugins' },
  ])
  assert.deepEqual(merged.map(g => [g.label, g.items.map(i => i.id)]), [
    ['Data', ['sources', 'hub']],
    ['Admin', ['users', 'license']],
    ['Plugins', ['extra']],
  ])
  // Input is not mutated.
  assert.deepEqual(core[0].items, [{ id: 'sources' }])
})

test('mergeTabs never lets an extra tab shadow a core id or an earlier extra', () => {
  const merged = mergeTabs(
    [{ label: 'Data', items: [{ id: 'sources' }] }],
    [{ id: 'sources', group: 'Data' }, { id: 'hub', group: 'Data' }, { id: 'hub', group: 'Data' }],
  )
  assert.deepEqual(merged[0].items.map(i => i.id), ['sources', 'hub'])
})

test('mergeTabs with no extra tabs returns the core groups unchanged', () => {
  const core = [{ label: 'Data', items: [{ id: 'sources' }] }]
  assert.deepEqual(mergeTabs(core, []), core)
})

test('registerLocales adds each language under the enterprise namespace', () => {
  const calls: unknown[][] = []
  registerLocales({ addResourceBundle: (...args) => { calls.push(args) } }, {
    en: { license: { title: 'License' } },
    de: { license: { title: 'Lizenz' } },
  })
  assert.deepEqual(calls, [
    ['en', ENTERPRISE_NS, { license: { title: 'License' } }, true, true],
    ['de', ENTERPRISE_NS, { license: { title: 'Lizenz' } }, true, true],
  ])
})

test('registerLocales is a no-op without bundles', () => {
  registerLocales({ addResourceBundle: () => assert.fail('must not be called') }, undefined)
})

test('normalizeLicense tolerates a status without site or support fields', () => {
  const info = normalizeLicense({ status: 'Valid', tier: 'enterprise', feature_flags: {}, license_key: 'k' })
  assert.equal(info.site, undefined)
  assert.equal(info.support_until, undefined)
  assert.equal(supportState(info), 'none')
  assert.equal(supportLabel(info), null)
  assert.equal(siteLabel(info), null)
})

test('normalizeLicense parses site and support, ignoring malformed values', () => {
  const info = normalizeLicense({
    status: 'Valid', tier: 'integrator', feature_flags: {},
    site: { id: 's1', name: 'Plant A' }, support_until: 1_900_000_000, support: 'active',
  })
  assert.deepEqual(info.site, { id: 's1', name: 'Plant A' })
  assert.equal(info.support_until, 1_900_000_000)
  assert.equal(siteLabel(info), 'Plant A')
  const bad = normalizeLicense({ site: { id: 1 }, support_until: 'soon', support: 'gold' })
  assert.equal(bad.site, undefined)
  assert.equal(bad.support_until, undefined)
  assert.equal(bad.support, undefined)
})

test('supportLabel covers active, lapsed and none', () => {
  const until = Date.UTC(2027, 0, 15) / 1000
  const base = { status: 'Valid' as const, tier: 'integrator' as const, feature_flags: COMMUNITY_LICENSE.feature_flags }
  assert.equal(supportLabel({ ...base, support_until: until }, until - 1), 'Support until 2027-01-15')
  assert.equal(
    supportLabel({ ...base, support_until: until }, until),
    'Support lapsed 2027-01-15 — renew to keep receiving updates',
  )
  assert.equal(supportLabel({ ...base, support_until: null }, until), null)
  assert.equal(supportLabel(COMMUNITY_LICENSE, until), null)
  // The server's derived state wins over the local clock.
  assert.equal(supportState({ ...base, support_until: until, support: 'lapsed' }, until - 1), 'lapsed')
})
