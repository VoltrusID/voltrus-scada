// Licence status model shared by core and the enterprise module.
// Kept free of React and of the enterprise alias so node tests can import it.

export type LicenseTier = 'community' | 'integrator' | 'enterprise'

export type LicenseStatusName =
  | 'Community'
  | 'Valid'
  | 'Unactivated'
  | 'Expired'
  | 'InvalidFingerprint'
  | 'InvalidSignature'
  | 'Corrupted'

/** The only licence-gated features. Everything else is always on in core. */
export const FEATURE_FLAG_KEYS = [
  'white_label',
  'theme_full',
  'pdf_reporting',
  'chat_notifications',
  'multi_site',
  'sso',
  'sso_saml',
  'cfr_part_11',
  'iec_62443',
  'ha',
  'rtsp_video',
] as const

export type FeatureFlag = (typeof FEATURE_FLAG_KEYS)[number]
export type FeatureFlags = Record<FeatureFlag, boolean>

export type SupportState = 'active' | 'lapsed' | 'none'

/** The site a per-site licence key is issued for. */
export interface LicenseSite {
  id: string
  name: string
}

export interface LicenseInfo {
  status: LicenseStatusName
  tier: LicenseTier
  feature_flags: FeatureFlags
  license_key?: string
  /** RFC3339 period end for monthly keys, null/absent for lifetime keys. */
  expires_at?: string | null
  days_remaining?: number | null
  /** True while a lapsed monthly key is inside the 7 day grace window. */
  grace?: boolean
  message?: string
  /** Per-site licence: the site the key is issued for. */
  site?: LicenseSite | null
  /** End of the annual support contract, Unix seconds; null/absent = none. */
  support_until?: number | null
  /** Server-derived support state; absent on servers that predate it. */
  support?: SupportState
}

const STATUS_NAMES: readonly string[] = [
  'Community', 'Valid', 'Unactivated', 'Expired', 'InvalidFingerprint', 'InvalidSignature', 'Corrupted',
]
const TIERS: readonly string[] = ['community', 'integrator', 'enterprise']

export function communityFlags(): FeatureFlags {
  const flags = {} as FeatureFlags
  for (const k of FEATURE_FLAG_KEYS) flags[k] = false
  return flags
}

export const COMMUNITY_LICENSE: LicenseInfo = Object.freeze({
  status: 'Community',
  tier: 'community',
  feature_flags: Object.freeze(communityFlags()) as FeatureFlags,
}) as LicenseInfo

/**
 * Turn whatever /api/license/status returned (or failed to return) into a
 * LicenseInfo. Anything unrecognised degrades to Community: core is never
 * blocked by licence state.
 */
export function normalizeLicense(raw: unknown): LicenseInfo {
  if (typeof raw !== 'object' || raw === null) return COMMUNITY_LICENSE
  const r = raw as Record<string, unknown>

  const status = typeof r.status === 'string' && STATUS_NAMES.includes(r.status)
    ? (r.status as LicenseStatusName)
    : 'Community'
  const tier = typeof r.tier === 'string' && TIERS.includes(r.tier) ? (r.tier as LicenseTier) : 'community'

  const flags = communityFlags()
  const rawFlags = typeof r.feature_flags === 'object' && r.feature_flags !== null
    ? (r.feature_flags as Record<string, unknown>)
    : {}
  for (const k of FEATURE_FLAG_KEYS) flags[k] = rawFlags[k] === true

  const info: LicenseInfo = { status, tier, feature_flags: flags }
  if (typeof r.license_key === 'string') info.license_key = r.license_key
  if (typeof r.expires_at === 'string' || r.expires_at === null) info.expires_at = r.expires_at
  if (typeof r.days_remaining === 'number' || r.days_remaining === null) info.days_remaining = r.days_remaining
  if (typeof r.grace === 'boolean') info.grace = r.grace
  if (typeof r.message === 'string') info.message = r.message
  if (typeof r.site === 'object' && r.site !== null) {
    const s = r.site as Record<string, unknown>
    if (typeof s.id === 'string' && typeof s.name === 'string') info.site = { id: s.id, name: s.name }
  }
  if (typeof r.support_until === 'number' && Number.isFinite(r.support_until)) info.support_until = r.support_until
  else if (r.support_until === null) info.support_until = null
  if (r.support === 'active' || r.support === 'lapsed' || r.support === 'none') info.support = r.support
  return info
}

/** Support state: the server's word, else derived from support_until and `nowSec`. */
export function supportState(license: LicenseInfo, nowSec: number = Math.floor(Date.now() / 1000)): SupportState {
  if (license.support) return license.support
  if (typeof license.support_until !== 'number') return 'none'
  return nowSec < license.support_until ? 'active' : 'lapsed'
}

function isoDate(unixSec: number): string {
  return new Date(unixSec * 1000).toISOString().slice(0, 10)
}

/** One line about the support contract, or null when there is none. Never gates anything. */
export function supportLabel(license: LicenseInfo, nowSec?: number): string | null {
  const until = license.support_until
  if (typeof until !== 'number') return null
  switch (supportState(license, nowSec)) {
    case 'active': return `Support until ${isoDate(until)}`
    case 'lapsed': return `Support lapsed ${isoDate(until)} — renew to keep receiving updates`
    default: return null
  }
}

/** Site name for display, or null for keys that are not per-site. */
export function siteLabel(license: LicenseInfo): string | null {
  return license.site?.name || null
}

export function hasFeature(license: LicenseInfo | null | undefined, flag: FeatureFlag): boolean {
  return license?.feature_flags?.[flag] === true
}
