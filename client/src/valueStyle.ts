// One style map for every bound value (dashboard cards, SCADA widgets,
// faceplates): alarm state and data quality are shown the same way
// everywhere, with no per-widget setting. Colours are theme variables, so
// both themes and their contrast rules apply (tests/contrast.test.ts).
//
// Precedence: no reading, then offline, then bad and uncertain quality (the
// server evaluates alarms on good values only), then critical, then warning.

import { qualityFamily, qualityLabel } from './quality.ts'

export type ValueState = 'normal' | 'warn' | 'crit' | 'offline' | 'bad' | 'uncertain' | 'nodata'

export interface ValueStyle {
  state: ValueState
  /** Text colour for the value; undefined keeps the widget's own colour. */
  color?: string
  /** Subtle fill or frame colour for the value's box; undefined: none. */
  tint?: string
  /** One-character marker shown next to the value (✕ bad, ? uncertain, ! alarm). */
  marker: string
  /** Short label for a badge or a screen reader; empty when normal. */
  label: string
  /** Longer explanation for a tooltip; empty when normal. */
  title: string
}

/** The theme variable of each state, shared with the contrast test. */
export const STATE_COLORS: Record<Exclude<ValueState, 'normal'>, string> = {
  crit: 'var(--crit)',
  warn: 'var(--warn)',
  bad: 'var(--hmi-stale)',
  uncertain: 'var(--hmi-stale)',
  offline: 'var(--text-muted)',
  nodata: 'var(--text-muted)',
}

const tint = (c: string, pct: number) => `color-mix(in srgb, ${c} ${pct}%, transparent)`

/** Alarm level of a status as any server or client path spells it. */
export function alarmLevel(status: string | undefined | null): 'crit' | 'warn' | 'offline' | 'normal' {
  switch (status) {
    case 'crit': case 'critical': return 'crit'
    case 'warn': case 'warning': return 'warn'
    case 'offline': return 'offline'
    default: return 'normal'
  }
}

export function valueStyle(data: { value?: number; status?: string; quality?: string } | undefined | null): ValueStyle {
  if (!data || data.value == null) {
    return { state: 'nodata', color: STATE_COLORS.nodata, marker: '', label: 'NO DATA', title: 'No reading yet' }
  }
  const level = alarmLevel(data.status)
  if (level === 'offline' || Number.isNaN(data.value)) {
    return { state: 'offline', color: STATE_COLORS.offline, marker: '', label: 'OFFLINE', title: 'Device offline' }
  }
  const q = qualityFamily(data.quality)
  if (q === 'bad') {
    return { state: 'bad', color: STATE_COLORS.bad, tint: tint(STATE_COLORS.bad, 10), marker: '✕', label: 'BAD', title: qualityLabel(data.quality) }
  }
  if (q === 'uncertain') {
    return { state: 'uncertain', color: STATE_COLORS.uncertain, tint: tint(STATE_COLORS.uncertain, 8), marker: '?', label: 'UNCERTAIN', title: qualityLabel(data.quality) }
  }
  if (level === 'crit') {
    return { state: 'crit', color: STATE_COLORS.crit, tint: tint(STATE_COLORS.crit, 14), marker: '!', label: 'CRITICAL', title: 'Critical alarm limit exceeded' }
  }
  if (level === 'warn') {
    return { state: 'warn', color: STATE_COLORS.warn, tint: tint(STATE_COLORS.warn, 14), marker: '!', label: 'WARNING', title: 'Warning alarm limit exceeded' }
  }
  return { state: 'normal', marker: '', label: '', title: '' }
}

/** States that frame a SCADA widget at runtime (no reading yet stays quiet). */
export function framesWidget(s: ValueStyle): boolean {
  return s.state === 'crit' || s.state === 'warn' || s.state === 'bad' || s.state === 'uncertain' || s.state === 'offline'
}
