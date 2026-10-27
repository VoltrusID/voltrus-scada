// Alarm-flood rule of the alarm panel: more than FLOOD_THRESHOLD alarms raised in the last
// FLOOD_WINDOW_SECS. Counts the raises themselves (`triggered_at`, Unix seconds), not the
// 24-hour per-source trigger counts of the analytics.

export const FLOOD_WINDOW_SECS = 300
export const FLOOD_THRESHOLD = 10

export interface FloodAlarm {
  triggered_at: number
}

export interface Flood {
  /** Alarms raised inside the window (a lower bound when `capped`). */
  count: number
  /** The list was cut at `limit` and every row is inside the window, so more may exist. */
  capped: boolean
}

/** The flood in the newest-first alarm list at `nowSecs`, or null when there is none. */
export function detectFlood(alarms: FloodAlarm[], nowSecs: number, limit: number): Flood | null {
  const since = nowSecs - FLOOD_WINDOW_SECS
  const count = alarms.filter(a => a.triggered_at > since).length
  if (count <= FLOOD_THRESHOLD) return null
  return { count, capped: count >= limit }
}
