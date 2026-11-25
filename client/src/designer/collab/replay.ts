// Local edits field by field, so they can be written again into another
// document (a server restart) without touching what others changed:
//
//   [root, id]                            the whole object (created or deleted here)
//   [root, id, field]                     x, y, width, height, name, type, sensors, zi, from, to, waypoints, config
//   [root, id, 'config', key]             config.<key>
//   [root, id, 'config', 'bindings', p]   config.bindings.<p>

import * as Y from 'yjs'

export type FieldPath = string[]

export class Touched {
  readonly paths = new Map<string, FieldPath>()
  get size() { return this.paths.size }
  add(p: FieldPath) { this.paths.set(JSON.stringify(p), p) }
  addAll(o: Touched | null | undefined) { o?.paths.forEach((p, k) => this.paths.set(k, p)) }
}

/** Record what the events of one root map (observeDeep) changed. */
export function recordEvents(root: string, events: Y.YEvent<any>[], into: Touched): void {
  for (const e of events) {
    const base = [root, ...e.path.map(String)]
    if (e instanceof Y.YMapEvent) for (const k of e.keysChanged) into.add([...base, k].slice(0, 5))
    else if (base.length > 1) into.add(base.slice(0, 5))
  }
}

/** A Y.Map as captured (plain values stay as they are). */
class Nested {
  readonly entries: [string, unknown][]
  constructor(entries: [string, unknown][]) { this.entries = entries }
}

const snap = (v: unknown): unknown =>
  v instanceof Y.Map ? new Nested([...v.entries()].map(([k, x]) => [k, snap(x)])) : v instanceof Y.AbstractType ? v.toJSON() : v

const unsnap = (v: unknown): unknown => {
  if (!(v instanceof Nested)) return v
  const m = new Y.Map<any>()
  for (const [k, x] of v.entries) m.set(k, unsnap(x))
  return m
}

export interface ReplayOp { path: FieldPath; present: boolean; value?: unknown }

/** The touched fields as they are in `doc` now. Fields of objects gone here are left out. */
export function captureReplay(doc: Y.Doc, touched: Touched): ReplayOp[] {
  const out: ReplayOp[] = []
  for (const path of touched.paths.values()) {
    let m: unknown = doc.getMap(path[0])
    for (const k of path.slice(1, -1)) m = m instanceof Y.Map ? m.get(k) : undefined
    if (!(m instanceof Y.Map)) continue
    const key = path[path.length - 1]
    out.push(m.has(key) ? { path, present: true, value: snap(m.get(key)) } : { path, present: false })
  }
  return out
}

/** Objects the ops change. */
export const objectsOf = (ops: ReplayOp[]) => new Set(ops.map(o => `${o.path[0]}\u0000${o.path[1]}`)).size

/**
 * Write the ops into `doc`: whole objects this client created or deleted,
 * then single fields of objects that still exist there (an object someone
 * else deleted stays deleted). A missing config or bindings map is created.
 */
export function applyReplay(doc: Y.Doc, ops: ReplayOp[], origin: unknown): void {
  if (!ops.length) return
  doc.transact(() => {
    const whole = new Set<string>()
    for (const o of ops) {
      if (o.path.length !== 2) continue
      const root = doc.getMap<any>(o.path[0])
      whole.add(`${o.path[0]}\u0000${o.path[1]}`)
      if (o.present) root.set(o.path[1], unsnap(o.value))
      else root.delete(o.path[1])
    }
    for (const o of ops) {
      if (o.path.length < 3 || whole.has(`${o.path[0]}\u0000${o.path[1]}`)) continue
      let m = doc.getMap<any>(o.path[0]).get(o.path[1])
      if (!(m instanceof Y.Map)) continue
      for (const k of o.path.slice(2, -1)) {
        let next = m.get(k)
        if (!(next instanceof Y.Map)) { next = new Y.Map<any>(); m.set(k, next) }
        m = next
      }
      const key = o.path[o.path.length - 1]
      if (o.present) m.set(key, unsnap(o.value))
      else m.delete(key)
    }
  }, origin)
}
