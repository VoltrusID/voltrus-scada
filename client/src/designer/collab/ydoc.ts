// The screen as a Yjs document (docs/collaboration.md), and the way between
// it and the Designer's model. The server (collab/src/document.rs) and the
// native console use the same layout:
//
//   widgets:     Y.Map id -> Y.Map { type, name, x, y, width, height, sensors, zi,
//                                    config: Y.Map { key -> value, bindings: Y.Map { prop -> binding } } }
//   connections: Y.Map id -> Y.Map { from, to, type, waypoints }
//
// Stacking order is (zi, id); config.z is never in the document. Every model
// edit (model.ts) still runs on plain Docs: writeDiff turns the difference of
// two Docs into field writes, so concurrent edits to other fields merge.

import * as Y from 'yjs'
import type { ConnectionInstance, ScadaWidgetType, WidgetInstance } from '../../scada2/widgets/types'
import type { ScadaConnectionItem, ScadaEquipmentItem } from '../../types'
import type { Doc } from '../model'
import { keyBetween, nKeysBetween } from './fractional.ts'

export const WIDGETS = 'widgets'
export const CONNECTIONS = 'connections'
export const BINDINGS = 'bindings'

export const widgetsOf = (d: Y.Doc) => d.getMap<Y.Map<any>>(WIDGETS)
export const connectionsOf = (d: Y.Doc) => d.getMap<Y.Map<any>>(CONNECTIONS)

export function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true
  if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) return false
  if (Array.isArray(a) !== Array.isArray(b)) return false
  if (Array.isArray(a)) {
    const bb = b as unknown[]
    return a.length === bb.length && a.every((v, i) => deepEqual(v, bb[i]))
  }
  const ka = Object.keys(a as object).filter(k => (a as any)[k] !== undefined)
  const kb = Object.keys(b as object).filter(k => (b as any)[k] !== undefined)
  return ka.length === kb.length && ka.every(k => deepEqual((a as any)[k], (b as any)[k]))
}

const plain = (v: unknown): any => (v instanceof Y.AbstractType ? v.toJSON() : v)
const num = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : 0)

/** The equipment name a save writes: label, else the loaded name, else a fallback. */
export const savedName = (w: WidgetInstance, nameOf: (w: WidgetInstance) => string) =>
  String(w.config?.label || w.name || nameOf(w))

const sensorsOf = (w: WidgetInstance) => w.sensorIds ?? (w.sensorId != null ? [w.sensorId] : [])

function readWidget(id: string, m: Y.Map<any>): { w: WidgetInstance; zi: string } {
  const cfgRaw = m.get('config')
  const config: Record<string, any> = cfgRaw instanceof Y.Map ? cfgRaw.toJSON() : (cfgRaw && typeof cfgRaw === 'object' ? { ...cfgRaw } : {})
  delete config.z
  // An empty bindings map means none (it is always there in the document).
  const b = config[BINDINGS]
  if (b && typeof b === 'object' && !Array.isArray(b) && Object.keys(b).length === 0) delete config[BINDINGS]
  const sensors = plain(m.get('sensors'))
  const ids: number[] = Array.isArray(sensors) ? sensors : []
  const name = m.get('name')
  const w: WidgetInstance = {
    id,
    type: String(m.get('type') ?? '') as ScadaWidgetType,
    sensorId: ids[0] ?? null,
    sensorIds: ids,
    x: num(m.get('x')),
    y: num(m.get('y')),
    width: num(m.get('width')),
    height: num(m.get('height')),
    config,
    ...(typeof name === 'string' && name ? { name } : {}),
  }
  const zi = m.get('zi')
  return { w, zi: typeof zi === 'string' ? zi : '' }
}

function readConnection(id: string, m: Y.Map<any>): ConnectionInstance {
  const wp = plain(m.get('waypoints'))
  return {
    id,
    fromWidgetId: String(m.get('from') ?? ''),
    toWidgetId: String(m.get('to') ?? ''),
    connectionType: String(m.get('type') ?? 'flow') as ConnectionInstance['connectionType'],
    waypoints: Array.isArray(wp) ? wp : [],
  }
}

/** Widget ids in stacking order: by (zi, id). */
export function orderOf(d: Y.Doc): string[] {
  const out: { id: string; zi: string }[] = []
  widgetsOf(d).forEach((m, id) => { if (m instanceof Y.Map) out.push({ id, zi: String(m.get('zi') ?? '') }) })
  out.sort((a, b) => (a.zi < b.zi ? -1 : a.zi > b.zi ? 1 : a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
  return out.map(o => o.id)
}

/**
 * The model of the document. Widgets and connections equal to the ones in
 * `prev` are `prev`'s objects, so React skips them.
 */
export function readDoc(d: Y.Doc, prev?: Doc): Doc {
  const prevW = new Map(prev?.widgets.map(w => [w.id, w]))
  const prevC = new Map(prev?.connections.map(c => [c.id, c]))
  const ws = widgetsOf(d)
  const widgets = orderOf(d).map(id => {
    const { w } = readWidget(id, ws.get(id)!)
    const p = prevW.get(id)
    return p && deepEqual(p, w) ? p : w
  })
  const connections: ConnectionInstance[] = []
  connectionsOf(d).forEach((m, id) => {
    if (!(m instanceof Y.Map)) return
    const c = readConnection(id, m)
    const p = prevC.get(id)
    connections.push(p && deepEqual(p, c) ? p : c)
  })
  connections.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
  const same = prev && widgets.length === prev.widgets.length && widgets.every((w, i) => w === prev.widgets[i])
    && connections.length === prev.connections.length && connections.every(c => prevC.get(c.id) === c)
  return same ? prev! : { widgets, connections }
}

/**
 * A config as the document holds it. `bindings` is always a map, empty when
 * nothing is bound, so two people binding a widget's first properties at once
 * write into one map instead of each creating their own (the server does the same).
 */
function configMap(config: Record<string, any>): Y.Map<any> {
  const m = new Y.Map<any>()
  for (const [k, v] of Object.entries(config)) {
    if (k === 'z' || v === undefined) continue
    if (k === BINDINGS && v && typeof v === 'object' && !Array.isArray(v)) {
      const b = new Y.Map<any>()
      for (const [p, bv] of Object.entries(v)) if (bv !== undefined) b.set(p, bv)
      m.set(k, b)
    } else m.set(k, v)
  }
  if (config[BINDINGS] === undefined) m.set(BINDINGS, new Y.Map<any>())
  return m
}

function widgetMap(w: WidgetInstance, zi: string, name: string): Y.Map<any> {
  const m = new Y.Map<any>()
  m.set('type', w.type)
  m.set('name', name)
  m.set('x', w.x)
  m.set('y', w.y)
  m.set('width', w.width)
  m.set('height', w.height)
  m.set('sensors', [...sensorsOf(w)])
  m.set('zi', zi)
  m.set('config', configMap(w.config ?? {}))
  return m
}

function connectionMap(c: ConnectionInstance): Y.Map<any> {
  const m = new Y.Map<any>()
  m.set('from', c.fromWidgetId)
  m.set('to', c.toWidgetId)
  m.set('type', c.connectionType)
  m.set('waypoints', c.waypoints.map(p => [...p]))
  return m
}

function writeConfig(m: Y.Map<any>, from: Record<string, any>, to: Record<string, any>) {
  let cfg = m.get('config')
  if (!(cfg instanceof Y.Map)) { m.set('config', configMap(to)); return }
  const keys = new Set([...Object.keys(from), ...Object.keys(to)])
  for (const k of keys) {
    if (k === 'z') continue
    const a = from[k], b = to[k]
    if (deepEqual(a, b)) continue
    const isObj = (v: unknown) => !!v && typeof v === 'object' && !Array.isArray(v)
    // No bindings counts as an empty map: binding by binding, never the whole map.
    if (k === BINDINGS && (isObj(a) || a === undefined) && (isObj(b) || b === undefined)) {
      const fa = a ?? {}, fb = b ?? {}
      let bm = cfg.get(BINDINGS)
      if (!(bm instanceof Y.Map)) { bm = new Y.Map<any>(); cfg.set(BINDINGS, bm) }
      for (const p of new Set([...Object.keys(fa), ...Object.keys(fb)])) {
        if (deepEqual(fa[p], fb[p])) continue
        if (fb[p] === undefined) bm.delete(p)
        else bm.set(p, fb[p])
      }
      continue
    }
    if (b === undefined) cfg.delete(k)
    else if (k === BINDINGS && isObj(b)) cfg.set(k, configMap({ [BINDINGS]: b }).get(BINDINGS))
    else cfg.set(k, b)
  }
}

/** Longest increasing subsequence of `xs`, as the set of its indexes. */
function lisIndexes(xs: number[]): Set<number> {
  const tails: number[] = []
  const prevIdx: number[] = new Array(xs.length).fill(-1)
  for (let i = 0; i < xs.length; i++) {
    let lo = 0, hi = tails.length
    while (lo < hi) { const mid = (lo + hi) >> 1; if (xs[tails[mid]] < xs[i]) lo = mid + 1; else hi = mid }
    if (lo > 0) prevIdx[i] = tails[lo - 1]
    tails[lo] = i
  }
  const out = new Set<number>()
  let k = tails.length ? tails[tails.length - 1] : -1
  while (k >= 0) { out.add(k); k = prevIdx[k] }
  return out
}

/**
 * Write the difference between `prev` and `next` (two model Docs, `prev`
 * being what the document showed) into the document, field by field, in one
 * transaction with `origin`. Only widgets whose place in the stacking order
 * changed get a new zi.
 */
export function writeDiff(d: Y.Doc, prev: Doc, next: Doc, nameOf: (w: WidgetInstance) => string, origin: unknown): void {
  if (prev === next) return
  d.transact(() => {
    const ws = widgetsOf(d)
    const prevW = new Map(prev.widgets.map(w => [w.id, w]))
    const nextIds = new Set(next.widgets.map(w => w.id))
    for (const w of prev.widgets) if (!nextIds.has(w.id)) ws.delete(w.id)
    // Stacking: widgets kept in place are the longest run already in order.
    const prevPos = new Map(prev.widgets.map((w, i) => [w.id, i]))
    const known = next.widgets.map((w, i) => ({ i, p: prevPos.get(w.id) })).filter(x => x.p != null && ws.get(next.widgets[x.i].id) instanceof Y.Map)
    const lis = lisIndexes(known.map(x => x.p!))
    const fixed = new Set(known.filter((_, k) => lis.has(k)).map(x => next.widgets[x.i].id))
    const newZi = new Map<string, string>()
    const ziOf = (id: string) => String(ws.get(id)?.get('zi') ?? '') || null
    for (let i = 0; i < next.widgets.length;) {
      if (fixed.has(next.widgets[i].id)) { i++; continue }
      let j = i
      while (j < next.widgets.length && !fixed.has(next.widgets[j].id)) j++
      const lo = i > 0 ? ziOf(next.widgets[i - 1].id) : null
      const hi = j < next.widgets.length ? ziOf(next.widgets[j].id) : null
      const keys = nKeysBetween(lo, hi, j - i)
      for (let k = i; k < j; k++) newZi.set(next.widgets[k].id, keys[k - i])
      i = j
    }
    for (const w of next.widgets) {
      const p = prevW.get(w.id)
      const m = ws.get(w.id)
      const name = savedName(w, nameOf)
      if (!p || !(m instanceof Y.Map)) {
        ws.set(w.id, widgetMap(w, newZi.get(w.id) ?? keyBetween(null, null), name))
        continue
      }
      const zi = newZi.get(w.id)
      if (zi && m.get('zi') !== zi) m.set('zi', zi)
      if (p === w) continue
      if (p.type !== w.type) m.set('type', w.type)
      for (const k of ['x', 'y', 'width', 'height'] as const) if (p[k] !== w[k]) m.set(k, w[k])
      if (!deepEqual(sensorsOf(p), sensorsOf(w))) m.set('sensors', [...sensorsOf(w)])
      if (m.get('name') !== name && savedName(p, nameOf) !== name) m.set('name', name)
      writeConfig(m, p.config ?? {}, w.config ?? {})
    }
    const cs = connectionsOf(d)
    const prevC = new Map(prev.connections.map(c => [c.id, c]))
    const nextC = new Set(next.connections.map(c => c.id))
    for (const c of prev.connections) if (!nextC.has(c.id)) cs.delete(c.id)
    for (const c of next.connections) {
      const p = prevC.get(c.id)
      const m = cs.get(c.id)
      if (!p || !(m instanceof Y.Map)) { cs.set(c.id, connectionMap(c)); continue }
      if (p === c) continue
      if (p.fromWidgetId !== c.fromWidgetId) m.set('from', c.fromWidgetId)
      if (p.toWidgetId !== c.toWidgetId) m.set('to', c.toWidgetId)
      if (p.connectionType !== c.connectionType) m.set('type', c.connectionType)
      if (!deepEqual(p.waypoints, c.waypoints)) m.set('waypoints', c.waypoints.map(q => [...q]))
    }
  }, origin)
}

/** Fill an empty document from rows, as the server does (zi from config.z). */
export function loadRows(d: Y.Doc, equipment: ScadaEquipmentItem[], connections: ScadaConnectionItem[], origin?: unknown): void {
  d.transact(() => {
    const parsed = equipment.map((eq, i) => {
      let config: Record<string, any> = {}
      try { const c = JSON.parse(eq.config || '{}'); if (c && typeof c === 'object' && !Array.isArray(c)) config = c } catch { /* bad row */ }
      const z = typeof config.z === 'number' ? config.z : i
      delete config.z
      if (config.rotation == null && eq.rotation) config.rotation = eq.rotation
      let sensors: number[] = []
      try { const s = JSON.parse(eq.sensor_bindings || '[]'); if (Array.isArray(s)) sensors = s } catch { /* bad row */ }
      return { eq, config, z, i, sensors }
    })
    const order = [...parsed].sort((a, b) => a.z - b.z || a.i - b.i)
    const keys = nKeysBetween(null, null, order.length)
    const ws = widgetsOf(d)
    order.forEach((p, k) => {
      const w: WidgetInstance = {
        id: p.eq.equipment_id, type: p.eq.equipment_type as ScadaWidgetType, sensorId: p.sensors[0] ?? null, sensorIds: p.sensors,
        x: p.eq.x, y: p.eq.y, width: p.eq.width, height: p.eq.height, config: p.config,
      }
      ws.set(w.id, widgetMap(w, keys[k], p.eq.equipment_name ?? ''))
    })
    const cs = connectionsOf(d)
    for (const c of connections) {
      let wp: [number, number][] = []
      try { const v = JSON.parse(c.waypoints || '[]'); if (Array.isArray(v)) wp = v } catch { /* bad row */ }
      cs.set(c.connection_id, connectionMap({ id: c.connection_id, fromWidgetId: c.from_equipment, toWidgetId: c.to_equipment, connectionType: c.connection_type as ConnectionInstance['connectionType'], waypoints: wp }))
    }
  }, origin)
}
