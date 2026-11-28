// Protocol traffic of one device (GET /api/v1/devices/{id}/traffic, docs/traffic.md):
// types, the query string, merging polled frames, and hex-pane helpers.

export type TrafficValue = number | boolean | string | null

export interface TrafficField {
  /** Wireshark display-filter name, '' for a grouping node. */
  id: string
  label: string
  value: TrafficValue
  display: string
  offset: number
  len: number
  children?: TrafficField[]
}

export interface TrafficError {
  kind: 'exception' | 'malformed' | 'no_response'
  code?: number
  title: string
  explanation: string
}

export interface TrafficTagRef {
  key: string
  tag_id?: number
  tag_name?: string
  offset?: number
  len?: number
}

export interface TrafficFrame {
  seq: number
  ts_us: number
  dir: 'tx' | 'rx'
  proto: string
  summary: string
  function?: number
  range?: { space: string; start: number; count: number }
  fields?: TrafficField[]
  raw?: string
  pair_id: number | null
  latency_us: number | null
  error: TrafficError | null
  tag_refs?: TrafficTagRef[]
}

export interface TrafficResponse {
  device_id: string
  capturing: boolean
  capture_until_ms: number
  capacity: number
  kept: number
  evicted: number
  frames: TrafficFrame[]
  note?: string
}

export interface TrafficFilters {
  function: string
  errors: boolean
  space: string
  addrFrom: string
  addrTo: string
}

export const EMPTY_FILTERS: TrafficFilters = { function: '', errors: false, space: '', addrFrom: '', addrTo: '' }

/** Function codes offered in the filter (Modbus). */
export const MODBUS_FUNCTIONS: [number, string][] = [
  [1, 'Read Coils'], [2, 'Read Discrete Inputs'], [3, 'Read Holding Registers'], [4, 'Read Input Registers'],
  [5, 'Write Single Coil'], [6, 'Write Single Register'], [15, 'Write Multiple Coils'], [16, 'Write Multiple Registers'],
  [22, 'Mask Write Register'], [23, 'Read/Write Multiple Registers'], [43, 'Read Device Identification'],
]

/** Frames kept in the browser; the server keeps 2,000 per device. */
export const MAX_FRAMES = 2000

/** Query string for the filters, plus `after` when polling for newer frames. */
export function trafficQuery(f: TrafficFilters, after?: number): string {
  const p = new URLSearchParams()
  if (f.function) p.set('function', f.function)
  if (f.errors) p.set('errors', 'true')
  if (f.space) p.set('space', f.space)
  const from = f.addrFrom.trim()
  const to = f.addrTo.trim()
  if (/^\d+$/.test(from)) p.set('addr_from', from)
  if (/^\d+$/.test(to)) p.set('addr_to', to)
  if (after !== undefined) p.set('after', String(after))
  p.set('limit', '500')
  return p.toString()
}

/** Seconds of frames fetched again on every poll: a request gets its pair
 *  and latency when the answer comes, or "no response" after 5 s. */
const REVISIT_US = 7_000_000

/** The `after` for the next poll: just before the frames that may still change. */
export function nextAfter(frames: TrafficFrame[]): number | undefined {
  if (frames.length === 0) return undefined
  const newest = frames[frames.length - 1].ts_us
  const first = frames.find(f => f.ts_us >= newest - REVISIT_US) ?? frames[frames.length - 1]
  return first.seq - 1
}

/** Merge polled frames into the list: newer copies replace older ones, order by seq, keep the newest MAX_FRAMES. */
export function mergeFrames(old: TrafficFrame[], incoming: TrafficFrame[]): TrafficFrame[] {
  if (incoming.length === 0) return old
  const bySeq = new Map<number, TrafficFrame>()
  for (const f of old) bySeq.set(f.seq, f)
  for (const f of incoming) bySeq.set(f.seq, f)
  const merged = [...bySeq.values()].sort((a, b) => a.seq - b.seq)
  return merged.length > MAX_FRAMES ? merged.slice(merged.length - MAX_FRAMES) : merged
}

/** Raw hex as bytes. */
export function hexBytes(raw: string | undefined): number[] {
  const out: number[] = []
  if (!raw) return out
  for (let i = 0; i + 1 < raw.length; i += 2) out.push(parseInt(raw.slice(i, i + 2), 16))
  return out
}

/** The deepest field covering byte `i` (clicking a byte selects its field). */
export function fieldAt(fields: TrafficField[] | undefined, i: number): TrafficField | undefined {
  for (const f of fields ?? []) {
    if (i >= f.offset && i < f.offset + f.len) return fieldAt(f.children, i) ?? f
  }
  return undefined
}

/** Ids of the ancestors of `target` (to open them in the tree). */
export function pathTo(fields: TrafficField[] | undefined, target: TrafficField, trail: TrafficField[] = []): TrafficField[] | undefined {
  for (const f of fields ?? []) {
    if (f === target) return [...trail, f]
    const found = pathTo(f.children, target, [...trail, f])
    if (found) return found
  }
  return undefined
}

export function formatLatency(us: number | null): string {
  if (us == null) return ''
  if (us < 1000) return `${us} µs`
  if (us < 10_000) return `${(us / 1000).toFixed(2)} ms`
  return `${Math.round(us / 1000)} ms`
}

/** hh:mm:ss.mmm in local time. */
export function formatTime(tsUs: number): string {
  const d = new Date(Math.floor(tsUs / 1000))
  const pad = (n: number, w = 2) => String(n).padStart(w, '0')
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`
}
