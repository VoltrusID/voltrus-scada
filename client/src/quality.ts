// Per-value data quality as the server reports it (`data_quality` on live
// updates, `quality` on /api/v1/data): good, uncertain, uncertain_stale, bad,
// bad_comm_failure, bad_device_error, bad_config_error. Absent means good, so
// older servers keep looking as they did.

export type QualityFamily = 'good' | 'uncertain' | 'bad'

export function qualityFamily(quality: string | undefined | null): QualityFamily {
  if (!quality || quality === 'good') return 'good'
  if (quality.startsWith('bad')) return 'bad'
  if (quality.startsWith('uncertain')) return 'uncertain'
  return 'good'
}

const LABELS: Record<string, string> = {
  uncertain: 'Uncertain value',
  uncertain_stale: 'Stale: not updated for several poll cycles',
  bad: 'Bad value',
  bad_comm_failure: 'Bad: device not reachable, last known value',
  bad_device_error: 'Bad: the device reported an error for this item',
  bad_config_error: 'Bad: the tag is configured wrong',
}

/** A short explanation for a tooltip, empty for a good value. */
export function qualityLabel(quality: string | undefined | null): string {
  if (qualityFamily(quality) === 'good') return ''
  return LABELS[quality as string] ?? 'Value quality is not good'
}

/** The one-character marker shown next to a value that is not good. */
export function qualityMarker(quality: string | undefined | null): string {
  switch (qualityFamily(quality)) {
    case 'bad': return '✕'
    case 'uncertain': return '?'
    default: return ''
  }
}

interface HighLimitTag {
  warn_val: number
  crit_val: number
  source_config: string
}

/**
 * High alarm limits as the server reads them: `warn_enabled` / `crit_enabled` in
 * source_config switch a limit on or off explicitly (so 0 and negative limits work);
 * a tag without the flag keeps the old rule that only a limit above 0 is set.
 */
export function highLimits(tag: HighLimitTag, cfg?: Record<string, unknown>): { warn: number | null; crit: number | null } {
  let c = cfg
  if (!c) {
    try { c = JSON.parse(tag.source_config || '{}') } catch { c = {} }
  }
  const pick = (v: number, flag: unknown) => (typeof flag === 'boolean' ? (flag ? v : null) : v > 0 ? v : null)
  return { warn: pick(tag.warn_val, c?.warn_enabled), crit: pick(tag.crit_val, c?.crit_enabled) }
}
