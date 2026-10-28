import { useEffect, useRef, useCallback } from 'react'
import type { SensorConfig, SensorData } from '../types'
import { type TagUpdate, onWsReady } from '../api/websocket'
import { useDashboardStoreActions } from '../store/dashboardStore'
import { alarmStatus } from '../utils'

const LIVE_PTS = 300

export function useWSData(authed: boolean, sensors: SensorConfig[]) {
  const actions = useDashboardStoreActions()
  const sensorsRef = useRef(sensors)
  sensorsRef.current = sensors
  const prevKeysRef = useRef<string[]>([])

  const handleBatch = useCallback((updates: TagUpdate[]) => {
    const sensorMap = new Map<string, SensorConfig>()
    for (const s of sensorsRef.current) {
      sensorMap.set(s.key, s)
      sensorMap.set(String(s.id), s)
    }

    const state = actions.getState()
    // Accumulate per-tag within a batch so a burst of samples for one tag does
    // not collapse to only the last point. Preserve the server's sample
    // timestamp (seconds -> ms) instead of clobbering it with Date.now().
    const dataUpdates = new Map<string, { id: string; points: { value: number; ts: number }[]; value: SensorData }>()

    for (const u of updates) {
      const sensor = sensorMap.get(u.tag_id)
      if (!sensor) continue

      const k = String(sensor.id)
      const ts = u.ts && u.ts > 1e12 ? u.ts : u.ts ? u.ts * 1000 : Date.now()
      const isOffline = u.value == null || Number.isNaN(u.value)
      const value: SensorData = {
        id: sensor.id,
        key: sensor.key,
        value: u.value,
        warn: sensor.warn_val,
        crit: sensor.crit_val,
        source_type: sensor.source_type,
        // WS quality is comm quality (always good/normal); widgets read status as the alarm level.
        status: isOffline ? (u.quality || 'normal') : alarmStatus(sensor, u.value),
      }

      const prevArr = dataUpdates.has(k) ? dataUpdates.get(k)!.points : (state.dataMap[k] ?? [])
      const appended = isOffline ? prevArr : [...prevArr, { value: u.value, ts }]
      const trimmed = appended.length > LIVE_PTS ? appended.slice(-LIVE_PTS) : appended
      dataUpdates.set(k, { id: k, points: trimmed, value })
    }

    if (dataUpdates.size > 0) {
      actions.mergeSensorData(Array.from(dataUpdates.values()))
    }
  }, [actions])

  // Subscribe to sensor tag keys via WebSocket
  useEffect(() => {
    if (!authed || sensors.length === 0) return

    const keys = sensors.map(s => s.key)
    const prevKeys = prevKeysRef.current
    prevKeysRef.current = keys

    return onWsReady(ws => {
      const removed = prevKeys.filter(k => !keys.includes(k))
      if (removed.length > 0) ws.unsubscribe(removed)
      ws.subscribe(keys)
    })
  }, [authed, sensors])

  // Register batch handler and mark connected
  useEffect(() => {
    if (!authed) return

    return onWsReady(ws => {
      ws.onTagBatch(handleBatch)
      ws.onTransportChange(actions.setLiveTransport)
      actions.setLiveTransport(ws.getTransport())
      actions.setWsConnected(true)
    })

    // eslint-disable-next-line react-hooks/exhaustive-deps -- disconnect cleans up; reconnect re-registers
  }, [authed, handleBatch, actions])
}
