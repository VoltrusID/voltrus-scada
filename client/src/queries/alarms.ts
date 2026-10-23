import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { Alarm, AlarmSummary, AlarmAuditEntry, AlarmAnalyticsEntry, AlarmJournalEntry } from '../types'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export function useActiveAlarmCount(enabled: boolean) {
  return useQuery({
    queryKey: ['alarms', 'active', 'count'],
    queryFn: async () => {
      const r = await fetchJson<{ count: number }>('/api/v1/alarms/active/count')
      return r
    },
    enabled,
    refetchInterval: 5_000,
    staleTime: 4_000,
  })
}

export function useAlarms(
  status: string = 'all',
  enabled: boolean = true,
  filters?: { priority?: string; state?: string; shelved?: boolean },
) {
  return useQuery({
    queryKey: queryKeys.alarms(status),
    queryFn: () => {
      const params = new URLSearchParams({ status, limit: '50' })
      if (filters?.priority) params.set('priority', filters.priority)
      if (filters?.state) params.set('state', filters.state)
      if (filters?.shelved !== undefined) params.set('shelved', String(filters.shelved))
      return fetchJson<Alarm[]>(`/api/v1/alarms?${params.toString()}`)
    },
    enabled,
    staleTime: 5_000,
  })
}

/* ── Alarm journal (ISA-18.2 lifecycle history) ────────────────── */

export function useAcknowledgeAlarm() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/alarms/${id}/acknowledge`, { method: 'POST' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['alarms'] })
      qc.invalidateQueries({ queryKey: queryKeys.alarmSummary })
      qc.invalidateQueries({ queryKey: queryKeys.alarmAnalytics })
      qc.invalidateQueries({ queryKey: ['alarmJournal'] })
    },
  })
}

export function useAcknowledgeAllAlarms() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async () => {
      await fetchOk('/api/v1/alarms', { method: 'POST' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['alarms'] })
      qc.invalidateQueries({ queryKey: queryKeys.alarmSummary })
      qc.invalidateQueries({ queryKey: queryKeys.alarmAnalytics })
      qc.invalidateQueries({ queryKey: ['alarmJournal'] })
    },
  })
}

export function useAlarmCooldown() {
  return useQuery({
    queryKey: ['alarmCooldown'],
    queryFn: async () => {
      const r = await fetchJson<{ secs: number }>('/api/v1/settings/alarm-cooldown')
      return r.secs
    },
    staleTime: 60_000,
  })
}

export function useUpdateAlarmCooldown() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (secs: number) => {
      await fetchOk('/api/v1/settings/alarm-cooldown', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ secs }),
      })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['alarmCooldown'] })
    },
  })
}

/* ── Alarm summary ─────────────────────────────────────────────── */

export function useAlarmSummary() {
  return useQuery({
    queryKey: queryKeys.alarmSummary,
    queryFn: () => fetchJson<AlarmSummary>('/api/v1/alarms/summary'),
    staleTime: 5_000,
    refetchInterval: 10_000,
  })
}

/* ── Alarm audit trail ─────────────────────────────────────────── */

export function useAlarmAudit(params?: {
  alarm_id?: number
  from?: number
  to?: number
}) {
  return useQuery({
    queryKey: queryKeys.alarmAudit(params?.alarm_id),
    queryFn: () => {
      const qs = new URLSearchParams()
      if (params?.alarm_id) qs.set('alarm_id', String(params.alarm_id))
      if (params?.from) qs.set('from', String(params.from))
      if (params?.to) qs.set('to', String(params.to))
      return fetchJson<AlarmAuditEntry[]>(`/api/v1/alarms/audit?${qs.toString()}`)
    },
    staleTime: 10_000,
  })
}

/* ── Alarm analytics ───────────────────────────────────────────── */

export function useAlarmAnalytics(params?: {
  from?: number
  to?: number
}) {
  return useQuery({
    queryKey: queryKeys.alarmAnalytics,
    queryFn: () => {
      const qs = new URLSearchParams()
      if (params?.from) qs.set('from', String(params.from))
      if (params?.to) qs.set('to', String(params.to))
      return fetchJson<AlarmAnalyticsEntry[]>(`/api/v1/alarms/analytics?${qs.toString()}`)
    },
    staleTime: 30_000,
  })
}

/* ── Alarm journal (per-alarm ISA-18.2 history) ────────────────── */

export function useAlarmJournal(alarmId: number | null) {
  return useQuery({
    queryKey: queryKeys.alarmJournal(alarmId ?? undefined),
    queryFn: () => fetchJson<AlarmJournalEntry[]>(`/api/v1/alarms/${alarmId}/journal`),
    enabled: alarmId != null,
    staleTime: 10_000,
  })
}

/* ── Shelf / Unshelf mutations ──────────────────────────────────── */

export function useShelfAlarm() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, duration_secs, reason }: {
      id: number
      duration_secs: number
      reason: string
    }) => {
      await fetchOk(`/api/v1/alarms/${id}/shelf`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ duration_secs, reason }),
      })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['alarms'] })
      qc.invalidateQueries({ queryKey: queryKeys.alarmSummary })
      qc.invalidateQueries({ queryKey: ['alarmJournal'] })
    },
  })
}

export function useUnshelfAlarm() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/alarms/${id}/unshelf`, { method: 'POST' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['alarms'] })
      qc.invalidateQueries({ queryKey: queryKeys.alarmSummary })
      qc.invalidateQueries({ queryKey: ['alarmJournal'] })
    },
  })
}
