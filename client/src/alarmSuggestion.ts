// When the tag editor offers the server's suggested alarm on-delay.

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
 * Whether to offer the suggested on-delay as a "Use N s" action. The editor
 * never writes the suggestion into the form by itself: only this explicit
 * action does, so opening and saving a tag keeps its stored delay.
 */
export function offerOnDelay(form: { on_delay_secs: number }, suggestion: { on_delay_secs: number }): boolean {
  return suggestion.on_delay_secs > 0 && (form.on_delay_secs ?? 0) !== suggestion.on_delay_secs
}
