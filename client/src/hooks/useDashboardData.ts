import { useRef, useEffect, useCallback } from 'react'
import { SensorConfig, SensorData, DataPoint, LIVE_PTS } from '../types'
import { expandCompactData, alarmStatus } from '../utils'
import { useWSData } from './useWSData'
import { useSystemInfo, fetchJson } from '../queries'
import { useDashboardStoreActions, useHistoryReady } from '../store/dashboardStore'

export function useDashboardData(
  authed: boolean,
  mode: 'live' | 'history' | 'system' | 'flows' | 'tags' | 'assets' | 'scripts',
  activeScreen: string,
  sensors: SensorConfig[],
  screenSensors: SensorConfig[],
  historyRange: string,
  historyDetail: 'auto' | 'full',
  historyAutoAgg: string,
  turbo: boolean,
) {
  const initialLoadDone = useRef(false)
  const prevAuthed = useRef(authed)
  const historyReady = useHistoryReady()

  // Reset initialLoadDone when auth state changes (logout/login cycle)
  useEffect(() => {
    if (prevAuthed.current && !authed) {
      initialLoadDone.current = false
    }
    prevAuthed.current = authed
  }, [authed])

  const actions = useDashboardStoreActions()
  const { data: systemInfo } = useSystemInfo(authed && mode === 'system')

  // Sync systemInfo to store
  useEffect(() => {
    actions.setSystemInfo(systemInfo ?? null)
  }, [systemInfo, actions])

  // Real-time data via WebSocket (replaces SSE)
  useWSData(authed, sensors)

  // ── Initial data load ────────────────────────────────────────────
  useEffect(() => {
    if (!authed || sensors.length === 0 || initialLoadDone.current) return
    initialLoadDone.current = true
    fetchJson<{ data: Record<string, any> }>('/api/v1/history?range=30m').then(result => {
      if (!result.data) return
      const expanded = expandCompactData(result.data)
      const updates: { id: string; points: DataPoint[]; value: SensorData }[] = []
      for (const s of sensors) {
        const pts = expanded[s.key]
        if (pts) {
          const trimmed = pts.length > LIVE_PTS ? pts.slice(-LIVE_PTS) : pts
          const last = trimmed[trimmed.length - 1]
          const fakeValue: SensorData = {
            id: s.id, key: s.key,
            value: last?.value ?? s.base_val,
            warn: s.warn_val, crit: s.crit_val,
            source_type: s.source_type,
            status: last ? alarmStatus(s, last.value) : 'normal',
          }
          updates.push({ id: String(s.id), points: trimmed, value: fakeValue })
        }
      }
      if (updates.length > 0) actions.mergeSensorData(updates)
      actions.setHistoryReady(true)
    }).catch(() => {}).finally(() => {
      if (!historyReady) actions.setHistoryReady(true)
    })
  }, [authed, sensors, actions, historyReady])

  // ── History fetch ────────────────────────────────────────────────
  useEffect(() => {
    if (mode !== 'history') { actions.setHistoryMap(null); return }
    const dp = activeScreen === 'all' ? '' : `&screen=${activeScreen}`
    const detailParam = historyDetail === 'full' ? '&detail=full' : ''
    const aggParam = historyDetail === 'auto' ? `&agg=${historyAutoAgg}` : ''
    fetchJson<{ data: Record<string, any> }>(`/api/v1/history?range=${historyRange}${dp}${detailParam}${aggParam}`).then(result => {
      if (!result.data) return
      const expanded = expandCompactData(result.data)
      const remapped: Record<string, DataPoint[]> = {}
      for (const s of screenSensors) {
        const pts = expanded[s.key]
        if (pts) remapped[String(s.id)] = pts
      }
      actions.setHistoryMap(remapped)
    })
  }, [mode, historyRange, historyDetail, activeScreen, screenSensors, historyAutoAgg, actions])

  return {
    historyReady,
    systemInfo,
  }
}
