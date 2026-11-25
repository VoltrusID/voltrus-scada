// Alarm values and limits as an operator reads them: with the tag's decimals
// and unit, never the raw float (2.602250099182129). Mirrors
// `alarm_eval::format_reading` and `format_limit` on the server, which write
// the alarm message: the same rounding (toFixed: nearest, halves away from
// zero), no "-0.00", and NaN / inf / -inf for non-finite numbers.

const MAX_DECIMALS = 6

/** A tag's decimals kept to 0..6, or undefined when the tag is unknown. */
function shownDecimals(decimals: number | null | undefined): number | undefined {
  if (decimals == null || !Number.isFinite(decimals)) return undefined
  return Math.min(Math.max(Math.round(decimals), 0), MAX_DECIMALS)
}

/** `v.toFixed(d)` without a negative zero, and with the server's spelling of non-finite numbers. */
function fixed(v: number, d: number): string {
  if (Number.isNaN(v)) return 'NaN'
  if (!Number.isFinite(v)) return v > 0 ? 'inf' : '-inf'
  const s = v.toFixed(d)
  return /^-0(\.0+)?$/.test(s) ? s.slice(1) : s
}

/** A number with the tag's decimals; without them, as many as its size needs. */
export function formatAlarmNumber(v: number, decimals?: number | null): string {
  const d = shownDecimals(decimals)
  if (d !== undefined) return fixed(v, d)
  const a = Math.abs(v)
  return fixed(v, a >= 100 ? 0 : a >= 10 ? 1 : 2)
}

/**
 * An alarm limit: the tag's decimals, or more when the limit needs them (up
 * to six), so a limit of 0.05 on a one-decimal tag reads 0.05, not 0.1.
 */
export function formatAlarmLimitNumber(v: number, decimals?: number | null): string {
  const d = shownDecimals(decimals)
  if (d === undefined || !Number.isFinite(v)) return formatAlarmNumber(v, decimals)
  for (let n = d; n <= MAX_DECIMALS; n++) {
    if (Number(v.toFixed(n)) === v) return fixed(v, n)
  }
  return fixed(v, MAX_DECIMALS)
}

function withUnit(n: string, unit?: string | null): string {
  const u = (unit ?? '').trim()
  if (!u) return n
  return u.startsWith('%') || u.startsWith('°') ? `${n}${u}` : `${n} ${u}`
}

/** A value with its unit: `2.60 m`, `85.0%`, `21.5°C`. */
export function formatAlarmReading(v: number, decimals?: number | null, unit?: string | null): string {
  return withUnit(formatAlarmNumber(v, decimals), unit)
}

/** An alarm limit with its unit, keeping the digits the limit needs. */
export function formatAlarmLimit(v: number, decimals?: number | null, unit?: string | null): string {
  return withUnit(formatAlarmLimitNumber(v, decimals), unit)
}

const NUM = String.raw`-?\d+(?:\.\d+)?(?:e[+-]?\d+)?`
// "<name> reached <value><unit> (<severity>: <limit><unit>)" and the
// "dropped to ... (<severity> low: ...)" form the server writes.
const GENERATED = new RegExp(String.raw`^(.* (?:reached|dropped to) )(${NUM})(.*? \(\w+(?: low)?: )(${NUM})(.*\))$`)
/** Float noise: five or more decimals. */
const NOISY = /\.\d{5,}/

/**
 * An alarm message written before the server rounded its numbers ("reached
 * 2.602250099182129m"): the value and the limit, when they carry float
 * noise, are rounded to the tag's decimals. Only those two positions of a
 * generated alarm message are touched; any other text (a custom message, a
 * tag name with digits in it) is left as it is, as are messages written since.
 */
export function tidyAlarmMessage(message: string, decimals?: number | null): string {
  const m = GENERATED.exec(message)
  if (!m) return message
  const [, head, value, middle, limit, tail] = m
  const tidy = (n: string) => (NOISY.test(n) ? formatAlarmNumber(Number(n), decimals) : n)
  return `${head}${tidy(value)}${middle}${tidy(limit)}${tail}`
}
