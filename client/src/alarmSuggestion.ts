// When the tag editor pre-fills the server's suggested alarm on-delay.

import { highLimits } from './quality.ts'

interface TagLimits {
  warn_val: number
  crit_val: number
  source_config: string
  on_delay_secs: number
}

/** True when the tag has a high or a low alarm limit set. */
export function hasAlarmLimits(tag: TagLimits): boolean {
  let cfg: Record<string, unknown> = {}
  try { cfg = JSON.parse(tag.source_config || '{}') } catch { /* not JSON: no low limits */ }
  const high = highLimits(tag, cfg)
  return high.warn != null || high.crit != null || typeof cfg.low_warn === 'number' || typeof cfg.low_crit === 'number'
}

/**
 * The on-delay to pre-fill, or null to leave the field alone: only a tag with
 * alarm limits and no delay yet gets the suggestion, and only a non-zero one.
 */
export function prefillOnDelay(tag: TagLimits, suggestion: { on_delay_secs: number }): number | null {
  if ((tag.on_delay_secs ?? 0) > 0) return null
  if (!hasAlarmLimits(tag)) return null
  return suggestion.on_delay_secs > 0 ? suggestion.on_delay_secs : null
}
