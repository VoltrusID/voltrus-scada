import { useRef, useEffect, useState, useCallback } from 'react'
import { SensorConfig, SensorData, DataPoint, LIVE_PTS } from '../types'
import { expandCompactData, alarmStatus } from '../utils'
import { historyUrl } from '../historyQuery'
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
  /** The tag whose detail view is open: history then asks for that tag only. */
  detailSensor?: SensorConfig,
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
    fetchJson<{ data: Record<string, any> }>(historyUrl({ range: '30m', keyedBy: 'id' })).then(result => {
      if (!result.data) return
      const expanded = expandCompactData(result.data)
      const updates: { id: string; points: DataPoint[]; value: SensorData }[] = []
      for (const s of sensors) {
        const pts = expanded[String(s.id)]
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
  // Each change of range, screen or tag aborts the request before it, so a slow
  // answer for the old selection can never overwrite the new one.
  const [historyError, setHistoryError] = useState<string | null>(null)
  const [historyAttempt, setHistoryAttempt] = useState(0)
  const retryHistory = useCallback(() => setHistoryAttempt(n => n + 1), [])
  useEffect(() => {
    setHistoryError(null)
    if (mode !== 'history') { actions.setHistoryMap(null); return }
    // The detail chart needs one tag; the dashboard, the tags of its screen.
    const url = historyUrl({
      range: historyRange,
      screen: detailSensor ? undefined : activeScreen,
      sensor: detailSensor?.key,
      detail: historyDetail,
      agg: historyAutoAgg,
      keyedBy: 'id',
    })
    const wanted = detailSensor ? [detailSensor] : screenSensors
    const ctrl = new AbortController()
    fetchJson<{ data: Record<string, any> }>(url, { signal: ctrl.signal }).then(result => {
      if (ctrl.signal.aborted) return
      const expanded = expandCompactData(result.data ?? {})
      const remapped: Record<string, DataPoint[]> = {}
      for (const s of wanted) {
        const pts = expanded[String(s.id)]
        if (pts) remapped[String(s.id)] = pts
      }
      actions.setHistoryMap(remapped)
    }).catch((err: unknown) => {
      if (ctrl.signal.aborted) return
      setHistoryError(err instanceof Error && err.message ? err.message : 'History could not be loaded')
    })
    return () => ctrl.abort()
  }, [mode, historyRange, historyDetail, activeScreen, screenSensors, historyAutoAgg, detailSensor, actions, historyAttempt])

  return {
    historyReady,
    systemInfo,
    /** The last history request failed (null while loading or after success). */
    historyError,
    retryHistory,
  }
}
