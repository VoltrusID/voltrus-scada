// The operator layout of a screen on a phone (docs/designer.md, phase 5).
//
// It is generated from the screen, not drawn: process areas become sections,
// components are read top-to-bottom, left-to-right inside each section, an
// equipment symbol becomes a row (thumbnail, label, state, primary value),
// trends and tables become full-width cards, and decoration (pipes, lines,
// labels without bindings, unbound drawings) is dropped.
//
// An engineer adjusts it in the Designer's Mobile pane. Adjustments are kept
// per component in `config.mobile = {hide?, order?, span?}`, written only for
// what was changed, so generating again (after the screen was edited) keeps
// them. The key rides in the row's config, so it goes through every save
// path like any other property; the server checks its shape
// (collab/src/mobile.rs). This file has no React and is shared by the
// operator view and the Designer pane.

import type { WidgetInstance } from '../widgets/types'

/** What a component becomes on the phone. */
export type MobileKind =
  /** An equipment symbol: thumbnail, label, state, primary value; tap opens its faceplate. */
  | 'equipment'
  /** A bound value (gauge, number, indicator…): label, value, state. */
  | 'value'
  /** Trends, tables, charts, add-on widgets: a full-width card with the widget drawn in it. */
  | 'card'
  /** Operator controls (slider, toggle, push button, number input), drawn as they are. */
  | 'control'
  /** A label: its (bound) text. Unbound labels are decoration unless shown on purpose. */
  | 'note'

/** The stored adjustment of one component (`config.mobile`). */
export interface MobileOverride {
  /** true: hidden on phones; false: shown although the generator drops it (a plain label). */
  hide?: boolean
  /** Position inside its section (for a process area: the section's position among sections). */
  order?: number
  /** Columns of the phone's two-column grid: 1 half width, 2 full width (the default). */
  span?: 1 | 2
}

export const MOBILE_KEY = 'mobile'

export interface MobileItem {
  id: string
  kind: MobileKind
  span: 1 | 2
  /** Not shown to operators (dropped as decoration, or hidden by the engineer). */
  hidden: boolean
  /** The generator drops it (decoration, or fixed `visible: false`). */
  dropped: boolean
  /** The component carries a `config.mobile` adjustment. */
  adjusted: boolean
}

export interface MobileSection {
  /** The process area's widget id, or null for components outside any area. */
  id: string | null
  title: string
  hidden: boolean
  adjusted: boolean
  items: MobileItem[]
}

export interface OperatorLayout {
  sections: MobileSection[]
}

/** What the generator needs to know about symbols (from the symbol library). */
export interface SymbolInfo {
  binding?: 'state' | 'level' | 'value' | 'none'
  category?: string
}

export interface GenerateOptions {
  /** Library facts about a symbol key; unknown keys are treated as equipment when bound. */
  symbol?: (key: string) => SymbolInfo | undefined
}

/** Widget types drawn by a library symbol. */
export const SYMBOL_TYPES = new Set(['symbol', 'pump', 'motor', 'valve', 'tank_level', 'horizontal_tank'])
/** Widget types that become a full-width card. */
export const CARD_TYPES = new Set(['trend', 'multi_trend', 'data_table', 'bar_chart', 'pie_chart', 'annunciator', 'pid_faceplate'])
/** Operator controls. */
export const CONTROL_TYPES = new Set(['slider', 'number_input', 'toggle', 'push_button'])
/** Built-in widget types that show one bound value. */
const VALUE_TYPES = new Set([
  'gauge', 'indicator', 'bar', 'status_light', 'on_off', 'big_number', 'meter', 'semicircle_gauge', 'donut',
  'led_display', 'progress_bar',
])
/** Config keys naming a tag besides the primary binding (equipment symbols). */
const TAG_KEYS = ['faultSensorId', 'runSensorId', 'manualSensorId', 'interlockSensorId']

const isObj = (v: unknown): v is Record<string, any> => !!v && typeof v === 'object' && !Array.isArray(v)

/** The component's adjustment, read defensively: anything malformed counts as not set. */
export function mobileOverride(config: Record<string, any> | undefined): MobileOverride {
  const m = config?.[MOBILE_KEY]
  if (!isObj(m)) return {}
  const out: MobileOverride = {}
  if (typeof m.hide === 'boolean') out.hide = m.hide
  if (typeof m.order === 'number' && Number.isFinite(m.order)) out.order = m.order
  if (m.span === 1 || m.span === 2) out.span = m.span
  return out
}

const hasBindings = (config: Record<string, any> | undefined) => isObj(config?.bindings) && Object.keys(config!.bindings).length > 0
const bound = (w: WidgetInstance) => w.sensorId != null || (w.sensorIds?.length ?? 0) > 0

/**
 * What the component becomes on the phone, `'area'` for a process area (a
 * section) or `'decoration'` for what is dropped.
 */
export function classify(w: WidgetInstance, opts: GenerateOptions = {}): MobileKind | 'area' | 'decoration' {
  const t = w.type as string
  const c = w.config ?? {}
  if (t === 'area') return 'area'
  if (t === 'pipe') return 'decoration'
  if (t === 'label') return bound(w) ? 'value' : hasBindings(c) ? 'note' : 'decoration'
  if (SYMBOL_TYPES.has(t)) {
    const key = t === 'symbol' ? String(c.symbol ?? '') : t
    const info = opts.symbol?.(key)
    // Pipes, junctions, arrows and connectors are lines, even with a flow tag.
    if (info?.category === 'Piping' || /^pipe/.test(key)) return 'decoration'
    const tagged = bound(w) || TAG_KEYS.some(k => c[k] != null) || (typeof c.equipment === 'string' && c.equipment !== '')
      || (typeof c.faceplate === 'string' && c.faceplate !== '')
    return tagged ? 'equipment' : 'decoration'
  }
  if (CARD_TYPES.has(t)) return 'card'
  if (CONTROL_TYPES.has(t)) return 'control'
  if (VALUE_TYPES.has(t)) return bound(w) || hasBindings(c) ? 'value' : 'decoration'
  // Add-on and custom widgets draw themselves: a card.
  return 'card'
}

/** What a dropped component becomes when an engineer shows it anyway. */
function shownKind(w: WidgetInstance): MobileKind | null {
  return w.type === 'label' ? 'note' : null
}

interface Box { x: number; y: number; width: number; height: number; id: string }

/**
 * Reading order: top to bottom, left to right. Components whose vertical
 * centre lies within the first component of a row are on that row.
 */
export function readingOrder<T extends Box>(items: T[]): T[] {
  const sorted = [...items].sort((a, b) => a.y - b.y || a.x - b.x || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
  const rows: { bottom: number; items: T[] }[] = []
  for (const it of sorted) {
    const row = rows[rows.length - 1]
    const cy = it.y + it.height / 2
    if (row && cy <= row.bottom) row.items.push(it)
    else rows.push({ bottom: it.y + Math.max(it.height, 1), items: [it] })
  }
  return rows.flatMap(r => r.items.sort((a, b) => a.x - b.x || a.y - b.y || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0)))
}

const contains = (a: Box, x: number, y: number) => x >= a.x && x <= a.x + a.width && y >= a.y && y <= a.y + a.height

/**
 * Put stored positions (`order`) over the generated sequence. Components with
 * an order come first, sorted by it; each component without one is put
 * right after the component that precedes it in the generated sequence, so
 * a component added to the screen later lands where the generator would put
 * it, next to its neighbours.
 */
export function mergeOrder<T extends { id: string | null }>(generated: T[], orderOf: (t: T) => number | undefined): T[] {
  if (!generated.some(g => orderOf(g) != null)) return generated
  const index = new Map(generated.map((g, i) => [g.id, i]))
  const out = generated
    .filter(g => orderOf(g) != null)
    .sort((a, b) => orderOf(a)! - orderOf(b)! || index.get(a.id)! - index.get(b.id)!)
  generated.forEach((g, i) => {
    if (orderOf(g) != null) return
    const at = i === 0 ? 0 : out.indexOf(generated[i - 1]) + 1
    out.splice(at, 0, g)
  })
  return out
}

/** Order of the stored override, for [`mergeOrder`]. */
const storedOrder = (w: WidgetInstance | undefined) => (w ? mobileOverride(w.config).order : undefined)

const areaTitle = (w: WidgetInstance) => String(w.config?.title || w.config?.label || w.name || '').trim()

/**
 * The operator layout of a screen, with what is hidden still listed (the
 * Designer pane shows it; the operator view skips `hidden`).
 */
export function generateOperatorLayout(doc: { widgets: WidgetInstance[] }, opts: GenerateOptions = {}): OperatorLayout {
  const byId = new Map(doc.widgets.map(w => [w.id, w]))
  const areas = doc.widgets.filter(w => w.type === 'area')
  // Each component goes to the smallest area that holds its centre.
  const members = new Map<string | null, WidgetInstance[]>([[null, []]])
  for (const a of areas) members.set(a.id, [])
  for (const w of doc.widgets) {
    if (w.type === 'area') continue
    const cx = w.x + w.width / 2
    const cy = w.y + w.height / 2
    let best: WidgetInstance | null = null
    for (const a of areas) {
      if (contains(a, cx, cy) && (!best || a.width * a.height < best.width * best.height)) best = a
    }
    members.get(best?.id ?? null)!.push(w)
  }

  const itemOf = (w: WidgetInstance): MobileItem | null => {
    const kind = classify(w, opts)
    if (kind === 'area') return null
    const o = mobileOverride(w.config)
    const adjusted = w.config?.[MOBILE_KEY] !== undefined
    if (kind === 'decoration') {
      // Only a plain label can be shown anyway (as a note); other decoration is not listed.
      const as = shownKind(w)
      return as ? { id: w.id, kind: as, span: o.span ?? 2, hidden: o.hide !== false, dropped: true, adjusted } : null
    }
    // A fixed `visible: false` is hidden at runtime too (a bound one is decided live).
    const dropped = w.config?.visible === false
    return { id: w.id, kind, span: o.span ?? 2, hidden: o.hide ?? dropped, dropped, adjusted }
  }

  const sectionOf = (id: string | null): MobileSection => {
    const ws = readingOrder(members.get(id) ?? [])
    const items = ws.map(itemOf).filter((x): x is MobileItem => x != null)
    const area = id ? byId.get(id) : undefined
    const o = area ? mobileOverride(area.config) : {}
    return {
      id,
      title: area ? areaTitle(area) : '',
      hidden: o.hide === true,
      adjusted: area?.config?.[MOBILE_KEY] !== undefined,
      items: mergeOrder(items, it => storedOrder(byId.get(it.id))),
    }
  }

  const sections = mergeOrder(readingOrder(areas).map(a => sectionOf(a.id)), s => storedOrder(byId.get(s.id!)))
  const loose = sectionOf(null)
  return { sections: loose.items.length > 0 ? [loose, ...sections] : sections }
}

/** The sections and items an operator sees: nothing hidden, no empty section. */
export function visibleLayout(layout: OperatorLayout): OperatorLayout {
  return {
    sections: layout.sections
      .filter(s => !s.hidden)
      .map(s => ({ ...s, items: s.items.filter(i => !i.hidden) }))
      .filter(s => s.items.length > 0),
  }
}

// ── Adjusting (the Designer's Mobile pane) ──
//
// Each function returns config patches per widget id (`{config: {mobile}}`,
// `undefined` removing the key), the shape of the Designer's `patch` edit.

export type MobilePatches = Record<string, { config: Record<string, any> }>

/** The override with `change` applied; undefined when nothing is left. */
function withOverride(w: WidgetInstance, change: Partial<Record<keyof MobileOverride, unknown>>): MobileOverride | undefined {
  const next: Record<string, unknown> = { ...mobileOverride(w.config) }
  for (const [k, v] of Object.entries(change)) {
    if (v === undefined) delete next[k]
    else next[k] = v
  }
  return Object.keys(next).length > 0 ? (next as MobileOverride) : undefined
}

function patchFor(w: WidgetInstance, change: Partial<Record<keyof MobileOverride, unknown>>): MobilePatches {
  const next = withOverride(w, change)
  const cur = w.config?.[MOBILE_KEY]
  if (cur === undefined && next === undefined) return {}
  if (cur !== undefined && next !== undefined && JSON.stringify(cur) === JSON.stringify(next)) return {}
  return { [w.id]: { config: { [MOBILE_KEY]: next } } }
}

/** Hide or show a component (or a whole section, by its area id) on phones. */
export function setHiddenPatch(doc: { widgets: WidgetInstance[] }, id: string, hidden: boolean, opts: GenerateOptions = {}): MobilePatches {
  const w = doc.widgets.find(x => x.id === id)
  if (!w) return {}
  // What the generator would do: an area shows, decoration does not.
  const kind = classify(w, opts)
  const droppedByDefault = kind !== 'area' && (kind === 'decoration' || w.config?.visible === false)
  return patchFor(w, { hide: hidden === droppedByDefault ? undefined : hidden })
}

/** Half or full width. */
export function setSpanPatch(doc: { widgets: WidgetInstance[] }, id: string, span: 1 | 2): MobilePatches {
  const w = doc.widgets.find(x => x.id === id)
  return w ? patchFor(w, { span: span === 2 ? undefined : span }) : {}
}

/**
 * Move a component one place up (-1) or down (+1) inside its section, or a
 * section among the sections when `id` is a process area. Every component of
 * the section gets its position stored, or, when the result is the generated
 * order again, none does.
 */
export function movePatch(doc: { widgets: WidgetInstance[] }, id: string, delta: -1 | 1, opts: GenerateOptions = {}): MobilePatches {
  const layout = generateOperatorLayout(doc, opts)
  const byId = new Map(doc.widgets.map(w => [w.id, w]))
  const isArea = byId.get(id)?.type === 'area'
  let seq: string[]
  let fresh: string[]
  if (isArea) {
    seq = layout.sections.filter(s => s.id != null).map(s => s.id!)
    const plain = generateOperatorLayout({ widgets: doc.widgets.map(w => (w.type === 'area' ? withoutOrder(w) : w)) }, opts)
    fresh = plain.sections.filter(s => s.id != null).map(s => s.id!)
  } else {
    const section = layout.sections.find(s => s.items.some(i => i.id === id))
    if (!section) return {}
    seq = section.items.map(i => i.id)
    const members = new Set(seq)
    const plain = generateOperatorLayout({ widgets: doc.widgets.map(w => (members.has(w.id) ? withoutOrder(w) : w)) }, opts)
    fresh = plain.sections.find(s => s.id === section.id)?.items.map(i => i.id) ?? seq
  }
  const at = seq.indexOf(id)
  const to = at + delta
  if (at < 0 || to < 0 || to >= seq.length) return {}
  const next = [...seq]
  ;[next[at], next[to]] = [next[to], next[at]]
  const backToGenerated = next.length === fresh.length && next.every((x, i) => x === fresh[i])
  const out: MobilePatches = {}
  next.forEach((wid, i) => {
    const w = byId.get(wid)
    if (w) Object.assign(out, patchFor(w, { order: backToGenerated ? undefined : i }))
  })
  return out
}

function withoutOrder(w: WidgetInstance): WidgetInstance {
  const o = mobileOverride(w.config)
  if (o.order === undefined) return w
  const next = withOverride(w, { order: undefined })
  const config = { ...w.config }
  if (next) config[MOBILE_KEY] = next
  else delete config[MOBILE_KEY]
  return { ...w, config }
}

/** Remove every adjustment of the screen: the generated layout again. */
export function resetPatch(doc: { widgets: WidgetInstance[] }): MobilePatches {
  const out: MobilePatches = {}
  for (const w of doc.widgets) if (w.config?.[MOBILE_KEY] !== undefined) out[w.id] = { config: { [MOBILE_KEY]: undefined } }
  return out
}

/** How many components carry an adjustment. */
export const adjustedCount = (doc: { widgets: WidgetInstance[] }) => doc.widgets.filter(w => w.config?.[MOBILE_KEY] !== undefined).length
