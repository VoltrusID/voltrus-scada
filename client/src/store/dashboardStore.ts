import { createContext, useContext, useRef, useCallback, useSyncExternalStore, useMemo } from 'react'
import type { DataPoint, SensorData } from '../types'
import type { Transport } from '../api/websocket'

interface DashboardState {
  dataMap: Record<string, DataPoint[]>
  currentValues: Record<string, SensorData>
  historyMap: Record<string, DataPoint[]> | null
  historyReady: boolean
  wsConnected: boolean
  /** What carries the live data: the WebSocket, or the SSE fallback. */
  liveTransport: Transport
  systemInfo: { hostname: string; os: string; cpu_usage: number; memory_used_mb: number; memory_total_mb: number; memory_percent: number } | null
}

const initialState: DashboardState = {
  dataMap: {},
  currentValues: {},
  historyMap: null,
  historyReady: false,
  wsConnected: false,
  liveTransport: 'ws',
  systemInfo: null,
}

// Stable empty array fallback to avoid useSyncExternalStore infinite loops
const EMPTY_DATA: DataPoint[] = []

type Listener = () => void

function createDashboardStore() {
  let state = initialState
  const listeners = new Set<Listener>()

  const getState = () => state

  const setState = (updater: (prev: DashboardState) => DashboardState) => {
    const next = updater(state)
    if (next !== state) {
      state = next
      listeners.forEach(l => l())
    }
  }

  const subscribe = (listener: Listener) => {
    listeners.add(listener)
    return () => listeners.delete(listener)
  }

  // Properly immutable merge — returns new state only when data actually changed
  const mergeSensorData = (updates: { id: number | string; points: DataPoint[]; value: SensorData }[]) => {
    if (updates.length === 0) return
    setState(prev => {
      let nextDataMap: Record<string, DataPoint[]> | undefined
      let nextValues: Record<string, SensorData> | undefined

      for (const { id, points, value } of updates) {
        const k = String(id)
        const existingPoints = nextDataMap?.[k] ?? prev.dataMap[k]
        if (points !== existingPoints) {
          if (!nextDataMap) nextDataMap = { ...prev.dataMap }
          nextDataMap[k] = points
        }
        const existingValue = nextValues?.[k] ?? prev.currentValues[k]
        if (value !== existingValue) {
          if (!nextValues) nextValues = { ...prev.currentValues }
          nextValues[k] = value
        }
      }

      if (!nextDataMap && !nextValues) return prev
      return {
        ...prev,
        ...(nextDataMap ? { dataMap: nextDataMap } : {}),
        ...(nextValues ? { currentValues: nextValues } : {}),
      }
    })
  }

  const setHistoryMap = (map: Record<string, DataPoint[]> | null) => {
    setState(prev => prev.historyMap === map ? prev : { ...prev, historyMap: map })
  }

  const setHistoryReady = (ready: boolean) => {
    setState(prev => prev.historyReady === ready ? prev : { ...prev, historyReady: ready })
  }

  const setWsConnected = (connected: boolean) => {
    setState(prev => prev.wsConnected === connected ? prev : { ...prev, wsConnected: connected })
  }

  const setLiveTransport = (transport: Transport) => {
    setState(prev => prev.liveTransport === transport ? prev : { ...prev, liveTransport: transport })
  }

  const setSystemInfo = (info: DashboardState['systemInfo']) => {
    setState(prev => prev.systemInfo === info ? prev : { ...prev, systemInfo: info })
  }

  const clearData = () => {
    setState(prev => {
      const empty = Object.keys(prev.dataMap).length === 0 && Object.keys(prev.currentValues).length === 0
      return empty ? prev : initialState
    })
  }

  return {
    getState,
    subscribe,
    setState,
    mergeSensorData,
    setHistoryMap,
    setHistoryReady,
    setWsConnected,
    setLiveTransport,
    setSystemInfo,
    clearData,
  }
}

type Store = ReturnType<typeof createDashboardStore>

const DashboardStoreContext = createContext<Store | null>(null)

export const DashboardStoreProvider = DashboardStoreContext.Provider

function useStore() {
  const store = useContext(DashboardStoreContext)
  if (!store) throw new Error('useDashboardStore must be used within DashboardStoreProvider')
  return store
}

// Generic selector hook using useSyncExternalStore
// CRITICAL: selectors must return stable references when data hasn't changed.
// `paused` keeps returning the last value (an off-screen card need not follow
// live data) once there is one; the first render after it clears reads the
// store again.
export function useDashboardSelector<T>(selector: (state: DashboardState) => T, paused = false): T {
  const store = useStore()
  const selectorRef = useRef(selector)
  selectorRef.current = selector
  const pausedRef = useRef(paused)
  pausedRef.current = paused
  const lastRef = useRef<{ value: T } | null>(null)

  const getSnapshot = useCallback(() => {
    if (pausedRef.current && lastRef.current && lastRef.current.value !== undefined) return lastRef.current.value
    const value = selectorRef.current(store.getState())
    lastRef.current = { value }
    return value
  }, [store])
  const getServerSnapshot = useCallback(() => selectorRef.current(initialState), [])

  return useSyncExternalStore(store.subscribe, getSnapshot, getServerSnapshot)
}

// Public API — all return stable references for useSyncExternalStore compatibility
export function useSensorDataPoints(sensorId: number | string): DataPoint[] {
  const k = String(sensorId)
  return useDashboardSelector(s => s.dataMap[k] ?? EMPTY_DATA)
}

export function useSensorCurrentValue(sensorId: number | string, paused = false): SensorData | undefined {
  const k = String(sensorId)
  return useDashboardSelector(s => s.currentValues[k], paused)
}

interface Tracking {
  /** Keys the component has read; only changes to these re-render it. */
  keys: Set<string>
  /** It enumerated the record (Object.keys, JSON...): every change counts. */
  all: boolean
  /** The values the current snapshot was taken from. */
  base: Record<string, SensorData> | null
  proxy: Record<string, SensorData> | null
}

function trackingProxy(t: Tracking, live: () => Record<string, SensorData>): Record<string, SensorData> {
  // The target is a stand-in: reads always go to the latest values, so a key
  // read for the first time is never stale.
  return new Proxy({} as Record<string, SensorData>, {
    get(_, k) {
      if (typeof k !== 'string') return undefined
      t.keys.add(k)
      return live()[k]
    },
    has(_, k) {
      if (typeof k === 'string') t.keys.add(k)
      return k in live()
    },
    ownKeys() {
      t.all = true
      return Reflect.ownKeys(live())
    },
    getOwnPropertyDescriptor(_, k) {
      if (typeof k !== 'string') return undefined
      t.keys.add(k)
      const values = live()
      if (!(k in values)) return undefined
      return { value: values[k], writable: false, enumerable: true, configurable: true }
    },
    set() { return false },
    deleteProperty() { return false },
  })
}

/**
 * All live values, for components that look values up by id (widgets, SCADA
 * symbols, the twin). Unlike selecting `s.currentValues`, the component
 * re-renders only when a value it has read changes, not on every update of
 * any tag. The record is read-only, and a new object whenever it re-renders,
 * so it works as a memo or effect dependency.
 */
export function useTrackedCurrentValues(): Record<string, SensorData> {
  const store = useStore()
  const ref = useRef<Tracking>({ keys: new Set(), all: false, base: null, proxy: null })
  const getSnapshot = useCallback(() => {
    const t = ref.current
    const values = store.getState().currentValues
    if (t.proxy && t.base) {
      if (values === t.base) return t.proxy
      if (!t.all) {
        let changed = false
        for (const k of t.keys) {
          if (values[k] !== t.base[k]) { changed = true; break }
        }
        if (!changed) return t.proxy
      }
    }
    t.base = values
    t.proxy = trackingProxy(t, () => store.getState().currentValues)
    return t.proxy
  }, [store])
  return useSyncExternalStore(store.subscribe, getSnapshot, getSnapshot)
}

/** Live points of several tags; re-renders only when one of theirs changes. */
export function useSensorDataPointsMany(sensorIds: readonly (number | string)[]): Record<string, DataPoint[]> {
  const store = useStore()
  const keys = sensorIds.map(String).join(',')
  const cache = useRef<{ keys: string; picked: Record<string, DataPoint[]> } | null>(null)
  const getSnapshot = useCallback(() => {
    const map = store.getState().dataMap
    const ids = keys ? keys.split(',') : []
    const prev = cache.current
    if (prev && prev.keys === keys && ids.every(k => prev.picked[k] === (map[k] ?? EMPTY_DATA))) return prev.picked
    const picked: Record<string, DataPoint[]> = {}
    for (const k of ids) picked[k] = map[k] ?? EMPTY_DATA
    cache.current = { keys, picked }
    return picked
  }, [store, keys])
  return useSyncExternalStore(store.subscribe, getSnapshot, getSnapshot)
}

export function useHistoryDataPoints(sensorId: number | string): DataPoint[] | undefined {
  const k = String(sensorId)
  return useDashboardSelector(s => s.historyMap?.[k])
}

export function useHistoryReady(): boolean {
  return useDashboardSelector(s => s.historyReady)
}

export function useWsConnected(): boolean {
  return useDashboardSelector(s => s.wsConnected)
}

export function useLiveTransport(): Transport {
  return useDashboardSelector(s => s.liveTransport)
}

export function useChartData(sensorId: number | string, mode: 'live' | 'history', paused = false): DataPoint[] {
  const k = String(sensorId)
  return useDashboardSelector(s => {
    if (mode === 'history' && s.historyMap && s.historyMap[k]) {
      return s.historyMap[k]
    }
    return s.dataMap[k] ?? EMPTY_DATA
  }, paused)
}

export function useDetailData(sensorId: number | string, mode: 'live' | 'history' | 'flows'): DataPoint[] {
  const k = String(sensorId)
  return useDashboardSelector(s => {
    if (mode === 'history' && s.historyMap && s.historyMap[k]) {
      return s.historyMap[k]
    }
    return s.dataMap[k] ?? EMPTY_DATA
  })
}

// Convenience hook that maps DataPoint[] → number[] with stable memoization.
export function useChartValues(sensorId: number | string, mode: 'live' | 'history', paused = false): number[] {
  const raw = useChartData(sensorId, mode, paused)
  return useMemo(() => raw.map(d => d.value), [raw])
}

export function useDashboardStoreActions() {
  const store = useStore()
  return useRef({
    mergeSensorData: store.mergeSensorData,
    setHistoryMap: store.setHistoryMap,
    setHistoryReady: store.setHistoryReady,
    setWsConnected: store.setWsConnected,
    setLiveTransport: store.setLiveTransport,
    setSystemInfo: store.setSystemInfo,
    clearData: store.clearData,
    getState: store.getState,
  }).current
}

export function useDashboardSystemInfo() {
  return useDashboardSelector(s => s.systemInfo)
}

export { createDashboardStore, type DashboardState }
