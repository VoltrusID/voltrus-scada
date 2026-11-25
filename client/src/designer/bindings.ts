// Property bindings: any widget property can follow a tag, an expression or a
// state-to-value map instead of holding a fixed value. Bindings live in the
// widget's `config.bindings`, keyed by property name, so they travel with the
// screen through the REST rows, MCP and the project file unchanged.
//
//   config.bindings = {
//     label:   { kind: 'expr', expr: '"P-101A " + text({wtp/intake_flow}, 0) + " m³/h"' },
//     visible: { kind: 'tag',  tag: 'wtp/p101a_run' },
//     color:   { kind: 'map',  expr: '{wtp/p101b_fault}', cases: [{ op: '>=', v: 1, out: '#f04646' }], else: '#8b939e' },
//   }
//
// Tags are named by project ref (`device/key`), resolved to the live value in
// the browser. The widget's primary value tag stays in `sensorId`/`sensor_bindings`.

import { compile, escapeRef, ExprError, refsOf, safeEvaluate, truthy, type Value } from './expr.ts'

export type Binding =
  | { kind: 'tag'; tag: string }
  | { kind: 'expr'; expr: string }
  | { kind: 'map'; expr: string; cases: MapCase[]; else?: unknown }

export interface MapCase { op: '==' | '>=' | '<=' | '>' | '<'; v: number; out: unknown }

export type PropType = 'string' | 'number' | 'boolean' | 'color' | 'select' | 'tag'

export const BINDINGS_KEY = 'bindings'

/** Properties every widget has besides its own schema. */
export const COMMON_PROPS: { key: string; label: string; type: PropType; default: unknown }[] = [
  { key: 'visible', label: 'Visible', type: 'boolean', default: true },
  { key: 'opacity', label: 'Opacity', type: 'number', default: 1 },
]

interface TagLike { id: number; key: string; device_id?: string | null }

/** A tag's base project ref: `device/key`. Use refsById for the ref of a tag that shares it with others. */
export const tagRef = (s: TagLike): string => `${s.device_id ?? ''}/${s.key}`

/**
 * Ref of every tag, numbered like the project exporter does: tags are taken in
 * id order, the first with a given `device/key` gets the plain ref and the next
 * ones `device/key#2`, `#3`... So a ref means the same tag in the browser, in
 * the project file and on the server.
 */
export function refsById(sensors: TagLike[]): Map<number, string> {
  const counts = new Map<string, number>()
  const out = new Map<number, string>()
  for (const s of [...sensors].sort((a, b) => a.id - b.id)) {
    const base = tagRef(s)
    const n = (counts.get(base) ?? 0) + 1
    counts.set(base, n)
    out.set(s.id, n === 1 ? base : `${base}#${n}`)
  }
  return out
}

export function refIndex(sensors: TagLike[]): Map<string, number> {
  const m = new Map<string, number>()
  for (const [id, r] of refsById(sensors)) m.set(r, id)
  return m
}

// One index per sensors array, shared by every widget on a screen.
const indexCache = new WeakMap<TagLike[], Map<string, number>>()

/** refIndex, built once per sensors array. */
export function sharedRefIndex(sensors: TagLike[]): Map<string, number> {
  let m = indexCache.get(sensors)
  if (!m) { m = refIndex(sensors); indexCache.set(sensors, m) }
  return m
}

export function bindingsOf(config: Record<string, any> | undefined): Record<string, Binding> {
  const b = config?.[BINDINGS_KEY]
  return b && typeof b === 'object' && !Array.isArray(b) ? b : {}
}

export const hasBindings = (config: Record<string, any> | undefined) => Object.keys(bindingsOf(config)).length > 0

/** Config with one property bound (or unbound with `null`). Returns a new config object. */
export function withBinding(config: Record<string, any>, prop: string, binding: Binding | null): Record<string, any> {
  const next = { ...bindingsOf(config) }
  if (binding) next[prop] = binding
  else delete next[prop]
  const out = { ...config }
  if (Object.keys(next).length) out[BINDINGS_KEY] = next
  else delete out[BINDINGS_KEY]
  return out
}

/** Problems in a binding, for the property editor. Empty means fine. */
export function bindingErrors(b: Binding, known: Set<string> | Map<string, number>): string[] {
  const errs: string[] = []
  const has = (r: string) => known.has(r)
  if (b.kind === 'tag') {
    if (!b.tag) errs.push('Pick a tag')
    else if (!has(b.tag)) errs.push(`Unknown tag ${b.tag}`)
    return errs
  }
  const c = compile(b.expr)
  if (c instanceof ExprError) return [c.message]
  for (const r of refsOf(c)) if (!has(r)) errs.push(`Unknown tag {${r}}`)
  if (b.kind === 'map') b.cases.forEach((x, i) => { if (typeof x.v !== 'number' || !Number.isFinite(x.v)) errs.push(`Case ${i + 1} needs a number`) })
  return errs
}

/** Every tag ref a widget's bindings read. */
export function bindingRefs(config: Record<string, any> | undefined): string[] {
  const out = new Set<string>()
  for (const b of Object.values(bindingsOf(config))) {
    if (!b || typeof b !== 'object') continue
    if (b.kind === 'tag') { if (b.tag) out.add(b.tag) } else if (typeof b.expr === 'string') {
      const c = compile(b.expr)
      if (!(c instanceof ExprError)) refsOf(c, out)
    }
  }
  return [...out]
}

function coerce(v: unknown, type: PropType | undefined): unknown {
  if (type === 'boolean') return typeof v === 'boolean' ? v : truthy(v as Value)
  if (type === 'number') {
    const n = typeof v === 'number' ? v : v == null || v === '' ? NaN : Number(v)
    return Number.isFinite(n) ? n : undefined
  }
  if (type === 'string') return v == null ? '' : typeof v === 'number' ? String(Math.round(v * 1000) / 1000) : String(v)
  return v
}

const matches = (x: number, c: MapCase) =>
  c.op === '==' ? x === c.v : c.op === '>=' ? x >= c.v : c.op === '<=' ? x <= c.v : c.op === '>' ? x > c.v : x < c.v

/**
 * The value a binding gives now. `undefined` when it cannot be evaluated (the
 * property keeps its fixed value). Never throws: a binding is saved data, and a
 * bad one must not take the screen down.
 */
export function evaluateBinding(b: Binding, read: (ref: string) => number | null, type?: PropType): unknown {
  try {
    return evaluateUnsafe(b, read, type)
  } catch {
    return undefined
  }
}

function evaluateUnsafe(b: Binding, read: (ref: string) => number | null, type?: PropType): unknown {
  if (!b || typeof b !== 'object') return undefined
  if (b.kind === 'tag') {
    const v = read(b.tag)
    return v == null ? undefined : coerce(v, type)
  }
  if (typeof b.expr !== 'string') return undefined
  const c = compile(b.expr)
  if (c instanceof ExprError) return undefined
  const v = safeEvaluate(c, read)
  if (b.kind === 'expr') return v == null ? undefined : coerce(v, type)
  if (b.kind !== 'map') return undefined
  const x = typeof v === 'boolean' ? (v ? 1 : 0) : typeof v === 'number' ? v : NaN
  const cases = Array.isArray(b.cases) ? b.cases : []
  const hit = Number.isNaN(x) ? undefined : cases.find(c => c && typeof c.v === 'number' && Number.isFinite(c.v) && matches(x, c))
  const out = hit ? hit.out : b.else
  return out === undefined ? undefined : coerce(out, type)
}

/**
 * The config a widget is drawn with: every bound property replaced by its
 * current value. `types` gives the property types (from the widget schema).
 * Returns the same object when nothing is bound.
 */
export function applyBindings(
  config: Record<string, any>,
  read: (ref: string) => number | null,
  types: Record<string, PropType> = {},
): Record<string, any> {
  const bs = bindingsOf(config)
  const keys = Object.keys(bs)
  if (keys.length === 0) return config
  const out: Record<string, any> = { ...config }
  for (const k of keys) {
    // Tag-picker fields hold tag ids, not values: they are never bound (the editor offers no binding for them).
    if (k === BINDINGS_KEY || types[k] === 'tag') continue
    const v = evaluateBinding(bs[k], read, types[k] ?? (k === 'visible' ? 'boolean' : k === 'opacity' ? 'number' : undefined))
    if (v !== undefined) out[k] = v
  }
  return out
}

/** Whether a widget is drawn at runtime: its bound `visible` when that has a value, else the fixed one. */
export function shownAtRuntime(config: Record<string, any> | undefined, read: (ref: string) => number | null): boolean {
  const b = bindingsOf(config).visible
  if (b) {
    const v = evaluateBinding(b, read, 'boolean')
    if (v !== undefined) return v !== false
  }
  return config?.visible !== false
}

/** Reader over the live value store, by ref. */
export function liveReader(index: Map<string, number>, values: Record<string, { value: number } | undefined>) {
  return (ref: string): number | null => {
    const id = index.get(ref)
    if (id == null) return null
    const v = values[String(id)]?.value
    return typeof v === 'number' ? v : null
  }
}

/** One-line description of a binding for the property grid. */
export function describeBinding(b: Binding): string {
  if (b.kind === 'tag') return escapeRef(b.tag)
  if (b.kind === 'expr') return b.expr
  return `map ${b.expr} (${b.cases.length} case${b.cases.length === 1 ? '' : 's'})`
}
