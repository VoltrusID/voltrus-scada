// Editor model of the Designer: the screen being edited, every edit as one
// undoable step, and the pure operations behind the toolbar (align,
// distribute, group, lock, z-order, duplicate). No React here; the tests in
// client/tests/designerModel.test.ts drive it directly.
//
// The document is the same ScadaLayoutState the 2D runtime loads and saves
// (scada2/engine/layout.ts), so a screen saved by the Designer is an ordinary
// screen. Designer-only facts live in each widget's config:
//   config.group    group id shared by the members of a group
//   config.locked   true: not moved, resized or marquee-selected
//   config.bindings property bindings (bindings.ts)

import type { ConnectionInstance, WidgetInstance } from '../scada2/widgets/types'
import type { ScadaLayoutState } from '../scada2/engine/widgetState'

export type Doc = ScadaLayoutState

export interface Box { x: number; y: number; width: number; height: number }

export type Patch = Partial<Omit<WidgetInstance, 'id' | 'config'>> & { config?: Record<string, any> }

export type Edit =
  | { type: 'add'; widgets: WidgetInstance[]; connections?: ConnectionInstance[] }
  | { type: 'remove'; ids: string[] }
  /** Field updates per widget id. A `config` patch is merged key by key; a key set to undefined is removed. */
  | { type: 'patch'; patches: Record<string, Patch> }
  | { type: 'order'; ids: string[]; to: 'front' | 'back' | 'forward' | 'backward' }
  | { type: 'addConnection'; connection: ConnectionInstance }
  | { type: 'removeConnection'; id: string }
  | { type: 'patchConnection'; id: string; patch: Partial<ConnectionInstance> }

export function applyEdit(doc: Doc, e: Edit): Doc {
  switch (e.type) {
    case 'add':
      if (e.widgets.length === 0 && !e.connections?.length) return doc
      return { widgets: [...doc.widgets, ...e.widgets], connections: [...doc.connections, ...(e.connections ?? [])] }
    case 'remove': {
      const gone = new Set(e.ids)
      if (!doc.widgets.some(w => gone.has(w.id))) return doc
      return {
        widgets: doc.widgets.filter(w => !gone.has(w.id)),
        connections: doc.connections.filter(c => !gone.has(c.fromWidgetId) && !gone.has(c.toWidgetId)),
      }
    }
    case 'patch': {
      let changed = false
      const widgets = doc.widgets.map(w => {
        const p = e.patches[w.id]
        if (!p) return w
        const next = patchWidget(w, p)
        if (next !== w) changed = true
        return next
      })
      return changed ? { ...doc, widgets } : doc
    }
    case 'order': return reorder(doc, e.ids, e.to)
    case 'addConnection': return { ...doc, connections: [...doc.connections, e.connection] }
    case 'removeConnection': {
      if (!doc.connections.some(c => c.id === e.id)) return doc
      return { ...doc, connections: doc.connections.filter(c => c.id !== e.id) }
    }
    case 'patchConnection':
      return { ...doc, connections: doc.connections.map(c => (c.id === e.id ? { ...c, ...e.patch } : c)) }
  }
}

function patchWidget(w: WidgetInstance, p: Patch): WidgetInstance {
  let changed = false
  const next: WidgetInstance = { ...w }
  for (const [k, v] of Object.entries(p)) {
    if (k === 'config') continue
    if ((w as any)[k] !== v) { (next as any)[k] = v; changed = true }
    if (v === undefined) delete (next as any)[k]
  }
  if (p.config) {
    const config = { ...w.config }
    for (const [k, v] of Object.entries(p.config)) {
      if (v === undefined) { if (k in config) { delete config[k]; changed = true } } else if (!sameValue(config[k], v)) { config[k] = v; changed = true }
    }
    next.config = config
  }
  return changed ? next : w
}

const sameValue = (a: unknown, b: unknown) => a === b || (typeof a === 'object' && typeof b === 'object' && JSON.stringify(a) === JSON.stringify(b))

function reorder(doc: Doc, ids: string[], to: 'front' | 'back' | 'forward' | 'backward'): Doc {
  const sel = new Set(ids)
  const ws = doc.widgets
  let out: WidgetInstance[]
  if (to === 'front' || to === 'back') {
    const moved = ws.filter(w => sel.has(w.id))
    const rest = ws.filter(w => !sel.has(w.id))
    out = to === 'front' ? [...rest, ...moved] : [...moved, ...rest]
  } else {
    out = [...ws]
    // Step each selected widget past its nearest unselected neighbour, keeping the selection's own order.
    const idx = out.map((w, i) => (sel.has(w.id) ? i : -1)).filter(i => i >= 0)
    if (to === 'forward') {
      for (const i of idx.reverse()) if (i + 1 < out.length && !sel.has(out[i + 1].id)) [out[i], out[i + 1]] = [out[i + 1], out[i]]
    } else {
      for (const i of idx) if (i > 0 && !sel.has(out[i - 1].id)) [out[i], out[i - 1]] = [out[i - 1], out[i]]
    }
  }
  return out.every((w, i) => w === ws[i]) ? doc : { ...doc, widgets: out }
}

// ── History ──

export interface History {
  past: Doc[]
  present: Doc
  future: Doc[]
  /** Edits with the same key in a row (typing in one field, nudging with arrows) are one undo step. */
  lastKey: string | null
  /** The document as last loaded or saved; `present !== saved` means unsaved changes. */
  saved: Doc
}

export type HistoryAction =
  | { type: 'edit'; edit: Edit; coalesce?: string }
  | { type: 'undo' }
  | { type: 'redo' }
  | { type: 'load'; doc: Doc }
  | { type: 'saved'; doc?: Doc }

export const HISTORY_LIMIT = 200

const EMPTY: Doc = { widgets: [], connections: [] }
export const emptyHistory: History = { past: [], present: EMPTY, future: [], lastKey: null, saved: EMPTY }

export function historyReducer(h: History, a: HistoryAction): History {
  switch (a.type) {
    case 'undo':
      if (h.past.length === 0) return h
      return { ...h, past: h.past.slice(0, -1), present: h.past[h.past.length - 1], future: [h.present, ...h.future], lastKey: null }
    case 'redo':
      if (h.future.length === 0) return h
      return { ...h, past: [...h.past, h.present], present: h.future[0], future: h.future.slice(1), lastKey: null }
    case 'load':
      return { past: [], present: a.doc, future: [], lastKey: null, saved: a.doc }
    case 'saved':
      return { ...h, saved: a.doc ?? h.present, lastKey: null }
    case 'edit': {
      const next = applyEdit(h.present, a.edit)
      if (next === h.present) return h
      const key = a.coalesce ?? null
      if (key && key === h.lastKey) return { ...h, present: next, future: [] }
      return { ...h, past: [...h.past, h.present].slice(-HISTORY_LIMIT), present: next, future: [], lastKey: key }
    }
  }
}

export const isDirty = (h: History) => h.present !== h.saved

// ── Selection helpers ──

export const groupOf = (w: WidgetInstance): string | null => (typeof w.config?.group === 'string' && w.config.group ? w.config.group : null)
export const isLocked = (w: WidgetInstance) => w.config?.locked === true

/** Clicking one member of a group selects the whole group. */
export function expandToGroups(doc: Doc, ids: string[]): string[] {
  const groups = new Set(doc.widgets.filter(w => ids.includes(w.id)).map(groupOf).filter(Boolean))
  if (groups.size === 0) return ids
  const out = new Set(ids)
  for (const w of doc.widgets) if (groups.has(groupOf(w))) out.add(w.id)
  // Keep stacking order, so the result is stable.
  return doc.widgets.filter(w => out.has(w.id)).map(w => w.id)
}

export function boundsOf(boxes: Box[]): Box | null {
  if (boxes.length === 0) return null
  const x = Math.min(...boxes.map(b => b.x))
  const y = Math.min(...boxes.map(b => b.y))
  const r = Math.max(...boxes.map(b => b.x + b.width))
  const btm = Math.max(...boxes.map(b => b.y + b.height))
  return { x, y, width: r - x, height: btm - y }
}

export const intersects = (a: Box, b: Box) => a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height

/** Widgets a marquee touches (locked ones are skipped). */
export function marqueeSelect(doc: Doc, box: Box): string[] {
  return expandToGroups(doc, doc.widgets.filter(w => !isLocked(w) && intersects(w, box)).map(w => w.id))
}

const pick = (doc: Doc, ids: string[]) => doc.widgets.filter(w => ids.includes(w.id))

// ── Arrange operations: each returns the patches for one 'patch' edit ──

export type AlignMode = 'left' | 'hcenter' | 'right' | 'top' | 'vmiddle' | 'bottom'

/** Align to the selection's bounding box (one widget: nothing to align to). Locked widgets stay. */
export function alignPatches(doc: Doc, ids: string[], mode: AlignMode): Record<string, Patch> {
  const ws = pick(doc, ids)
  const b = boundsOf(ws)
  if (!b || ws.length < 2) return {}
  const out: Record<string, Patch> = {}
  for (const w of ws) {
    if (isLocked(w)) continue
    let { x, y } = w
    if (mode === 'left') x = b.x
    if (mode === 'right') x = b.x + b.width - w.width
    if (mode === 'hcenter') x = Math.round(b.x + (b.width - w.width) / 2)
    if (mode === 'top') y = b.y
    if (mode === 'bottom') y = b.y + b.height - w.height
    if (mode === 'vmiddle') y = Math.round(b.y + (b.height - w.height) / 2)
    if (x !== w.x || y !== w.y) out[w.id] = { x, y }
  }
  return out
}

/** Equal gaps between neighbours, first and last staying where they are. Needs three or more. */
export function distributePatches(doc: Doc, ids: string[], axis: 'h' | 'v'): Record<string, Patch> {
  const ws = pick(doc, ids)
  if (ws.length < 3) return {}
  const pos = axis === 'h' ? 'x' : 'y'
  const size = axis === 'h' ? 'width' : 'height'
  const sorted = [...ws].sort((a, b) => a[pos] - b[pos])
  const first = sorted[0], last = sorted[sorted.length - 1]
  const span = last[pos] + last[size] - first[pos]
  const total = sorted.reduce((s, w) => s + w[size], 0)
  const gap = (span - total) / (sorted.length - 1)
  const out: Record<string, Patch> = {}
  let at = first[pos] + first[size] + gap
  for (const w of sorted.slice(1, -1)) {
    const v = Math.round(at)
    if (v !== w[pos] && !isLocked(w)) out[w.id] = { [pos]: v }
    at += w[size] + gap
  }
  return out
}

/** Same width and/or height as the first selected widget. */
export function matchSizePatches(doc: Doc, ids: string[], dim: 'width' | 'height' | 'both'): Record<string, Patch> {
  const ws = ids.map(id => doc.widgets.find(w => w.id === id)).filter(Boolean) as WidgetInstance[]
  if (ws.length < 2) return {}
  const ref = ws[0]
  const out: Record<string, Patch> = {}
  for (const w of ws.slice(1)) {
    if (isLocked(w)) continue
    const p: Patch = {}
    if (dim !== 'height' && w.width !== ref.width) p.width = ref.width
    if (dim !== 'width' && w.height !== ref.height) p.height = ref.height
    if (Object.keys(p).length) out[w.id] = p
  }
  return out
}

/** Move by (dx, dy), kept on the stage. Locked widgets stay. */
export function movePatches(doc: Doc, ids: string[], dx: number, dy: number, stage: { w: number; h: number }): Record<string, Patch> {
  const out: Record<string, Patch> = {}
  const ws = pick(doc, ids).filter(w => !isLocked(w))
  const b = boundsOf(ws)
  if (!b) return out
  // Clamp the whole selection together so its shape is kept at the stage edge.
  const cdx = Math.max(-b.x, Math.min(stage.w - (b.x + b.width), dx))
  const cdy = Math.max(-b.y, Math.min(stage.h - (b.y + b.height), dy))
  if (cdx === 0 && cdy === 0) return out
  for (const w of ws) out[w.id] = { x: w.x + cdx, y: w.y + cdy }
  return out
}

let seq = 0
export const newId = (p: string) => `${p}_${Date.now().toString(36)}${(seq++).toString(36)}${Math.random().toString(36).slice(2, 6)}`

/** Put the selection in one new group (two or more widgets). Members of other groups move into it. */
export function groupPatches(doc: Doc, ids: string[], groupId = newId('g')): Record<string, Patch> {
  if (ids.length < 2) return {}
  return Object.fromEntries(pick(doc, ids).map(w => [w.id, { config: { group: groupId } }]))
}

export function ungroupPatches(doc: Doc, ids: string[]): Record<string, Patch> {
  return Object.fromEntries(pick(doc, ids).filter(groupOf).map(w => [w.id, { config: { group: undefined } }]))
}

export function lockPatches(doc: Doc, ids: string[], locked: boolean): Record<string, Patch> {
  return Object.fromEntries(pick(doc, ids).filter(w => isLocked(w) !== locked).map(w => [w.id, { config: { locked: locked ? true : undefined } }]))
}

/**
 * Copies of widgets, offset, with new ids. Groups among the copies get new
 * group ids so a pasted group is its own group. Connections between two copied
 * widgets are copied too.
 */
export function duplicate(
  widgets: WidgetInstance[], connections: ConnectionInstance[], offset: number,
): { widgets: WidgetInstance[]; connections: ConnectionInstance[] } {
  const ids = new Map(widgets.map(w => [w.id, newId('w')]))
  const groups = new Map<string, string>()
  const out = widgets.map(w => {
    const config = structuredClone(w.config ?? {})
    if (config.group) {
      if (!groups.has(config.group)) groups.set(config.group, newId('g'))
      config.group = groups.get(config.group)
    }
    delete config.locked
    const copy: WidgetInstance = { ...w, id: ids.get(w.id)!, x: w.x + offset, y: w.y + offset, config, sensorIds: [...(w.sensorIds ?? [])] }
    delete copy.name
    return copy
  })
  const conns = connections
    .filter(c => ids.has(c.fromWidgetId) && ids.has(c.toWidgetId))
    .map(c => ({ ...c, id: newId('c'), fromWidgetId: ids.get(c.fromWidgetId)!, toWidgetId: ids.get(c.toWidgetId)!, waypoints: c.waypoints.map(([x, y]) => [x + offset, y + offset] as [number, number]) }))
  return { widgets: out, connections: conns }
}

/** Bind the widget's primary value to a tag (drop a tag on it, or pick in the property editor). */
export function bindPrimaryPatch(tagId: number | null): Patch {
  // A new tag drops a name loaded from the server, so the saved name follows the tag.
  return { sensorId: tagId, sensorIds: tagId != null ? [tagId] : [], name: undefined }
}
