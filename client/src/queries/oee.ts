import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { fetchJson, fetchOk } from './fetch'

export interface DowntimeEvent {
  id: number
  device_id: string
  state: string
  started_at: number
  ended_at: number | null
  duration_secs: number | null
  reason: string | null
}

export interface OeeMetrics {
  availability: number
  performance: number
  quality: number
  oee: number
  total_running_secs: number
  total_planned_secs: number
  downtime_count: number
}

export interface ShiftConfig {
  id: number
  name: string
  start_hour: number
  start_minute: number
  duration_hours: number
  days_mask: number
  created_at: number
}

export function useOeeMetrics(deviceId: string | undefined, from?: number, to?: number) {
  const params = new URLSearchParams()
  if (deviceId) params.set('device_id', deviceId)
  if (from) params.set('from', String(from))
  if (to) params.set('to', String(to))
  const qs = params.toString()
  return useQuery({
    queryKey: ['oee', 'metrics', deviceId, from, to],
    queryFn: () => fetchJson<OeeMetrics>(`/api/v1/oee/metrics?${qs}`),
    enabled: !!deviceId,
  })
}

export function useDowntimeEvents(deviceId?: string, from?: number, to?: number) {
  const params = new URLSearchParams()
  if (deviceId) params.set('device_id', deviceId)
  if (from) params.set('from', String(from))
  if (to) params.set('to', String(to))
  const qs = params.toString()
  return useQuery({
    queryKey: ['oee', 'downtime', deviceId, from, to],
    queryFn: () => fetchJson<DowntimeEvent[]>(`/api/v1/oee/downtime?${qs}`),
  })
}

export function useCreateDowntimeEvent() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (data: { device_id: string; state: string }) => {
      const r = await fetchOk('/api/v1/oee/downtime', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
      })
      return r.json()
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['oee'] })
    },
  })
}

export function useCloseDowntimeEvent() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, reason }: { id: number; reason?: string }) => {
      const r = await fetchOk(`/api/v1/oee/downtime/${id}/close`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ reason }),
      })
      return r.json()
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['oee'] })
    },
  })
}

export function useShifts() {
  return useQuery({
    queryKey: ['oee', 'shifts'],
    queryFn: () => fetchJson<ShiftConfig[]>('/api/v1/oee/shifts'),
  })
}

export function useCreateShift() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (data: { name: string; start_hour: number; start_minute: number; duration_hours: number; days_mask: number }) => {
      const r = await fetchOk('/api/v1/oee/shifts', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
      })
      return r.json()
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['oee', 'shifts'] })
    },
  })
}

export function useDeleteShift() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/oee/shifts/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['oee', 'shifts'] })
    },
  })
}
