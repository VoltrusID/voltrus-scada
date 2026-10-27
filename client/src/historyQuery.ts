import type { DataPoint } from './types'

export interface HistoryQuery {
  range: string
  /** Limit the answer to one screen's tags. Omitted or `all`: every tag. */
  screen?: string
  /** Limit the answer to one tag key. */
  sensor?: string
  detail?: 'auto' | 'full'
  /** `sampling`, `avg` or `minmax`; only sent with auto detail. */
  agg?: string
}

/** The `GET /api/v1/history` URL for a query. */
export function historyUrl(q: HistoryQuery): string {
  const params = new URLSearchParams({ range: q.range })
  if (q.screen && q.screen !== 'all') params.set('screen', q.screen)
  if (q.sensor) params.set('sensor', q.sensor)
  if (q.detail === 'full') params.set('detail', 'full')
  else if (q.agg) params.set('agg', q.agg)
  return `/api/v1/history?${params}`
}

export interface BandPoint { time: number; lo: number; hi: number }

/**
 * A series' min/max band as chart data: epoch seconds, ascending and unique,
 * without points that lack either edge.
 */
export function bandPoints(points: DataPoint[]): BandPoint[] {
  const byTime = new Map<number, BandPoint>()
  for (const p of points) {
    const { lo, hi } = p
    if (lo === null || lo === undefined || !Number.isFinite(lo)) continue
    if (hi === null || hi === undefined || !Number.isFinite(hi)) continue
    const time = p.ts / 1000
    byTime.set(time, { time, lo, hi })
  }
  return [...byTime.values()].sort((a, b) => a.time - b.time)
}
