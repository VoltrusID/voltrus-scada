// Last-known values for the phone view (docs/designer.md, phase 5).
//
// While the operator view is live it keeps a small snapshot of the screen in
// this browser: its rows, the tags it shows (name, unit, decimals) and their
// last values with the time each was read. When the connection drops, or the
// installed app opens with no server to reach, the view shows that snapshot
// with every value's age and an offline banner. A value is never shown as
// live once the live stream is gone. Signing out deletes the snapshots.
// No React here.

import type { ScadaConnectionItem, ScadaEquipmentItem, SensorConfig, SensorData } from '../../types'

export const STORAGE_KEY = 'voltrus.operator.lastKnown.v1'
/** Screens kept, most recent first. */
export const MAX_SCREENS = 5
/** A snapshot larger than this (JSON characters) is not kept. */
export const MAX_CHARS = 400_000
/** A live value read longer ago than this shows its age. */
export const AGE_AFTER_MS = 60_000

export interface LastValue { value: number; ts_ms?: number; quality?: string; status?: string }

export interface ScreenSnapshot {
  screen: string
  screenName: string
  /** When the snapshot was taken (ms). */
  savedAt: number
  equipment: ScadaEquipmentItem[]
  connections: ScadaConnectionItem[]
  sensors: Pick<SensorConfig, 'id' | 'key' | 'name' | 'unit' | 'decimals' | 'device_id' | 'min_val' | 'max_val'>[]
  values: Record<string, LastValue>
}

interface Stored { v: 1; user: string; screens: ScreenSnapshot[] }

type Storage = Pick<globalThis.Storage, 'getItem' | 'setItem' | 'removeItem'>

function read(storage: Storage): Stored | null {
  try {
    const raw = storage.getItem(STORAGE_KEY)
    if (!raw) return null
    const s = JSON.parse(raw)
    if (!s || s.v !== 1 || !Array.isArray(s.screens) || typeof s.user !== 'string') return null
    return s as Stored
  } catch {
    return null
  }
}

/** The values a snapshot keeps: the tags the screen shows. */
export function pickValues(ids: Iterable<number>, current: Record<string, SensorData>): Record<string, LastValue> {
  const out: Record<string, LastValue> = {}
  for (const id of ids) {
    const d = current[String(id)]
    if (!d || typeof d.value !== 'number' || !Number.isFinite(d.value)) continue
    out[String(id)] = { value: d.value, ts_ms: d.ts_ms, quality: d.quality, status: d.status }
  }
  return out
}

/** Keep `snap` for `user`, newest first, at most [`MAX_SCREENS`]. False when it was too large or storage refused it. */
export function saveSnapshot(storage: Storage, user: string, snap: ScreenSnapshot): boolean {
  const prev = read(storage)
  const others = prev && prev.user === user ? prev.screens.filter(s => s.screen !== snap.screen) : []
  const next: Stored = { v: 1, user, screens: [snap, ...others].slice(0, MAX_SCREENS) }
  try {
    const text = JSON.stringify(next)
    if (JSON.stringify(snap).length > MAX_CHARS) return false
    storage.setItem(STORAGE_KEY, text)
    return true
  } catch {
    return false
  }
}

/** The snapshot of a screen (or the most recent one when `screen` is omitted), for `user` when given. */
export function loadSnapshot(storage: Storage, screen?: string, user?: string): ScreenSnapshot | null {
  const s = read(storage)
  if (!s || (user !== undefined && s.user !== user)) return null
  return (screen ? s.screens.find(x => x.screen === screen) : s.screens[0]) ?? null
}

/** Delete every snapshot (sign-out). */
export function clearSnapshots(storage: Storage): void {
  try { storage.removeItem(STORAGE_KEY) } catch { /* storage unavailable */ }
}

/** Values of a snapshot as the live store holds them, so the same widgets can draw them. */
export function asSensorData(snap: ScreenSnapshot): Record<string, SensorData> {
  const out: Record<string, SensorData> = {}
  for (const [id, v] of Object.entries(snap.values)) {
    const s = snap.sensors.find(x => String(x.id) === id)
    out[id] = { id: Number(id), key: s?.key ?? '', value: v.value, warn: 0, crit: 0, source_type: '', status: v.status ?? 'ok', quality: v.quality, ts_ms: v.ts_ms }
  }
  return out
}

/** Tag ids a screen's widgets read: their bindings, the tag keys of symbols, faceplate maps. */
export function tagIdsOf(widgets: { sensorId: number | null; sensorIds?: number[]; config?: Record<string, any> }[]): Set<number> {
  const out = new Set<number>()
  const add = (v: unknown) => {
    const n = typeof v === 'number' ? v : typeof v === 'string' && v.trim() !== '' ? Number(v) : NaN
    if (Number.isInteger(n)) out.add(n)
  }
  for (const w of widgets) {
    add(w.sensorId)
    for (const id of w.sensorIds ?? []) add(id)
    const c = w.config ?? {}
    for (const k of ['faultSensorId', 'runSensorId', 'manualSensorId', 'interlockSensorId']) add(c[k])
    if (c.faceplateMap && typeof c.faceplateMap === 'object') for (const v of Object.values(c.faceplateMap)) add(v)
  }
  return out
}

let user = ''

/** Who is signed in: snapshots are kept per user, and only that user's are shown. */
export function setSnapshotUser(name: string): void { user = name }
export function snapshotUser(): string { return user }

/** Age of a reading in words: "just now", "45 s", "12 min", "3 h", "2 d". */
export function ageParts(ms: number): { n: number; unit: 'now' | 's' | 'min' | 'h' | 'd' } {
  const s = Math.max(0, Math.floor(ms / 1000))
  if (s < 5) return { n: 0, unit: 'now' }
  if (s < 60) return { n: s, unit: 's' }
  if (s < 3600) return { n: Math.floor(s / 60), unit: 'min' }
  if (s < 86400) return { n: Math.floor(s / 3600), unit: 'h' }
  return { n: Math.floor(s / 86400), unit: 'd' }
}

/**
 * Whether a value may be shown as live: the live stream is up and the value
 * is not older than [`AGE_AFTER_MS`]. Otherwise it is shown with its age.
 */
export function isLiveValue(live: boolean, d: { ts_ms?: number } | undefined, now: number): boolean {
  if (!live || !d) return false
  return d.ts_ms == null || now - d.ts_ms <= AGE_AFTER_MS
}
