import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export interface ShiftHandover {
  id: number
  author_id: number
  shift_date: string
  shift_period: string
  what_happened: string
  whats_pending: string
  what_to_watch: string
  created_at: number
}

export interface ShiftAutoReport {
  shift_date: string
  shift_period: string
  period_start: number
  period_end: number
  alarms: any[]
  annotations: any[]
  handovers: ShiftHandover[]
  security_events: any[]
}

export function useShiftHandovers(params: { shift_date?: string; shift_period?: string } = {}) {
  const sp = new URLSearchParams()
  if (params.shift_date) sp.set('shift_date', params.shift_date)
  if (params.shift_period) sp.set('shift_period', params.shift_period)
  const qs = sp.toString()
  return useQuery({
    queryKey: queryKeys.shiftHandoversFiltered(params.shift_date, params.shift_period),
    queryFn: () => fetchJson<ShiftHandover[]>(`/api/v1/shift-handover${qs ? '?' + qs : ''}`),
    staleTime: 5_000,
  })
}

export function useCreateShiftHandover() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (data: { shift_date: string; shift_period: string; what_happened: string; whats_pending: string; what_to_watch: string }) => {
      const res = await fetchOk('/api/v1/shift-handover', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
      })
      return res.json()
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.shiftHandovers })
    },
  })
}

export function useShiftAutoReport() {
  return useQuery({
    queryKey: queryKeys.shiftAutoReport,
    queryFn: () => fetchJson<ShiftAutoReport>('/api/v1/shift-handover/auto-report'),
    staleTime: 15_000,
    refetchInterval: 30_000,
  })
}
