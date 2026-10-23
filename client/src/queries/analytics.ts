import { useQuery } from '@tanstack/react-query'
import { fetchJson } from './fetch'

export interface AggregatedMetric {
  metric: string
  min: number
  max: number
  avg: number
  stddev: number
  count: number
  first_ts: number
  last_ts: number
}

export interface ComparisonResult {
  metric: string
  current: AggregatedMetric
  previous: AggregatedMetric
  change_percent: number
}

export function useAnalyticsAggregate(
  deviceId: string | undefined,
  metric: string | undefined,
  from: number | undefined,
  to: number | undefined,
  interval: number = 3600,
) {
  const params = new URLSearchParams()
  if (deviceId) params.set('device_id', deviceId)
  if (metric) params.set('metric', metric)
  if (from) params.set('from', String(from))
  if (to) params.set('to', String(to))
  params.set('interval', String(interval))
  const qs = params.toString()
  return useQuery({
    queryKey: ['analytics', 'aggregate', deviceId, metric, from, to, interval],
    queryFn: () => fetchJson<[number, AggregatedMetric][]>(`/api/v1/analytics/aggregate?${qs}`),
    enabled: !!deviceId && !!metric && !!from && !!to,
  })
}

export function useAnalyticsCompare(
  deviceId: string | undefined,
  metric: string | undefined,
  period1From: number | undefined,
  period1To: number | undefined,
  period2From: number | undefined,
  period2To: number | undefined,
) {
  const params = new URLSearchParams()
  if (deviceId) params.set('device_id', deviceId)
  if (metric) params.set('metric', metric)
  if (period1From) params.set('period1_from', String(period1From))
  if (period1To) params.set('period1_to', String(period1To))
  if (period2From) params.set('period2_from', String(period2From))
  if (period2To) params.set('period2_to', String(period2To))
  const qs = params.toString()
  return useQuery({
    queryKey: ['analytics', 'compare', deviceId, metric, period1From, period1To, period2From, period2To],
    queryFn: () => fetchJson<ComparisonResult>(`/api/v1/analytics/compare?${qs}`),
    enabled: !!deviceId && !!metric && !!period1From && !!period1To && !!period2From && !!period2To,
  })
}
