// The server stores and compares timestamps in Unix seconds; JavaScript dates are milliseconds.

/** Unix seconds for a `datetime-local`/date input value, or undefined when it does not parse. */
export function unixSecsFromInput(value: string): number | undefined {
  const ms = new Date(value).getTime()
  return Number.isNaN(ms) ? undefined : Math.floor(ms / 1000)
}

/** Date and time of a Unix-seconds timestamp in the browser's locale. */
export function fmtUnixSecs(secs: number): string {
  return new Date(secs * 1000).toLocaleString(undefined, {
    year: 'numeric', month: 'short', day: '2-digit',
    hour: '2-digit', minute: '2-digit', hour12: false,
  })
}

/** Whether a Unix-seconds timestamp lies in the past. */
export function isPastUnixSecs(secs: number, nowMs: number = Date.now()): boolean {
  return secs < nowMs / 1000
}

/** Time of day (HH:MM:SS, 24 h) of a Unix-seconds timestamp in the browser's locale. */
export function fmtUnixSecsClock(secs: number): string {
  return new Date(secs * 1000).toLocaleTimeString('en-US', {
    hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit',
  })
}

/** Audit-log `from`/`to` query values (Unix seconds) for two `datetime-local` inputs; empty or invalid inputs are omitted. */
export function auditRangeParams(from: string, to: string): { from?: number; to?: number } {
  const out: { from?: number; to?: number } = {}
  const f = from ? unixSecsFromInput(from) : undefined
  const t = to ? unixSecsFromInput(to) : undefined
  if (f !== undefined) out.from = f
  if (t !== undefined) out.to = t
  return out
}
