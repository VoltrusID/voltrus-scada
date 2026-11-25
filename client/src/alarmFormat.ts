// Alarm values and limits as an operator reads them: with the tag's decimals
// and unit, never the raw float (2.602250099182129). Mirrors
// `alarm_eval::format_reading` on the server, which writes the alarm message.

/** A tag's decimals kept to 0..6, or undefined when the tag is unknown. */
function shownDecimals(decimals: number | null | undefined): number | undefined {
  if (decimals == null || !Number.isFinite(decimals)) return undefined
  return Math.min(Math.max(Math.round(decimals), 0), 6)
}

/** A number with the tag's decimals; without them, as many as its size needs. */
export function formatAlarmNumber(v: number, decimals?: number | null): string {
  if (!Number.isFinite(v)) return String(v)
  const d = shownDecimals(decimals)
  if (d !== undefined) return v.toFixed(d)
  const a = Math.abs(v)
  return v.toFixed(a >= 100 ? 0 : a >= 10 ? 1 : 2)
}

/** A value with its unit: `2.60 m`, `85.0%`, `21.5°C`. */
export function formatAlarmReading(v: number, decimals?: number | null, unit?: string | null): string {
  const n = formatAlarmNumber(v, decimals)
  const u = (unit ?? '').trim()
  if (!u) return n
  return u.startsWith('%') || u.startsWith('°') ? `${n}${u}` : `${n} ${u}`
}

/**
 * An alarm message written before the server rounded its numbers ("reached
 * 2.602250099182129m"): every number with five or more decimals is rounded
 * to the tag's decimals. Messages written since are left as they are.
 */
export function tidyAlarmMessage(message: string, decimals?: number | null): string {
  return message.replace(/-?\d+\.\d{5,}/g, m => formatAlarmNumber(Number(m), decimals))
}
