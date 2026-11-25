// The symbol file format `voltrus-symbol/1` (symbols/<key>.json, schema in
// symbols/symbol.schema.json): one ISA-101 symbol as data, drawn by the web
// (dataSymbols.tsx) and meant for the native console (lyon tessellates the
// same path strings). Plain TypeScript: validation and option logic are used by
// node tests too.

import { PAINTS, type Paint } from './symbolStyle.ts'

export type PortSide = 'left' | 'right' | 'top' | 'bottom'
export type SymbolBinding = 'state' | 'level' | 'value' | 'none'

/** `{ option: value }` or `{ option: [values] }`: all must match the symbol's config. */
export type Condition = Record<string, string | boolean | (string | boolean)[]>

export interface SymbolLayer {
  d: string
  paint: Paint
  /** Base stroke width instead of the paint's (pipes are 3). */
  width?: number
  if?: Condition
}

export interface SymbolText {
  x: number
  y: number
  /** Fixed text, or */
  text?: string
  /** a config key (`tagFunction`), or `$level` (the level in %, hidden when unknown or `showPercent` is off). */
  from?: string
  size: number
  weight?: number
  font?: 'ui' | 'mono'
  /** Sits on a body: contrasting colour when the body is filled. */
  onBody?: boolean
  if?: Condition
}

export interface SymbolOption {
  key: string
  label: string
  type: 'select' | 'boolean' | 'string'
  values?: string[]
  default: string | boolean
}

export interface SymbolFile {
  format?: string
  key: string
  name: string
  category: string
  keywords?: string[]
  viewBox: [number, number]
  size: [number, number]
  binding: SymbolBinding
  stretch?: boolean
  /** False: kept for saved screens, not offered in the palette. */
  palette?: boolean
  ports?: Partial<Record<PortSide, [number, number]>>
  anchors?: { id: string; x: number; y: number; dir: PortSide }[]
  layers: SymbolLayer[]
  /** Level fill area (level binding), clipped to `clip` when given. */
  level?: { x: number; y: number; w: number; h: number; clip?: string }
  text?: SymbolText[]
  options?: SymbolOption[]
  /** Label of the secondary run tag field (`runSensorId`), when the symbol animates from one. */
  runTag?: string
}

export const SYMBOL_FORMAT = 'voltrus-symbol/1'

/** ISA-101 categories, in palette order. */
export const ISA_CATEGORIES = ['Rotating equipment', 'Valves', 'Vessels', 'Heat transfer', 'Separation', 'Instruments', 'Piping'] as const

const BINDINGS: SymbolBinding[] = ['state', 'level', 'value', 'none']
const SIDES: PortSide[] = ['left', 'right', 'top', 'bottom']

/** Does a layer, text or option condition hold for this config (missing keys take the option's default)? */
export function matches(cond: Condition | undefined, config: Record<string, any>, options: SymbolOption[] = []): boolean {
  if (!cond) return true
  for (const [k, want] of Object.entries(cond)) {
    const opt = options.find(o => o.key === k)
    const have = config[k] ?? opt?.default
    const list = Array.isArray(want) ? want : [want]
    if (!list.some(w => String(w) === String(have))) return false
  }
  return true
}

// SVG path data: commands and numbers only (what lyon's SVG path parser takes).
const PATH_TOKEN = /\s*(?:([MmLlHhVvCcSsQqTtAaZz])|([-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?))\s*,?/y
const ARGS: Record<string, number> = { M: 2, L: 2, H: 1, V: 1, C: 6, S: 4, Q: 4, T: 2, A: 7, Z: 0 }

/** Problems with a path string: unknown characters, a path not starting with M, wrong argument counts. */
export function pathProblems(d: string): string[] {
  const out: string[] = []
  PATH_TOKEN.lastIndex = 0
  let cmd: string | null = null
  let nums = 0
  const close = () => {
    if (!cmd) return
    const n = ARGS[cmd.toUpperCase()]
    if (n === 0 ? nums !== 0 : nums === 0 || nums % n !== 0) out.push(`'${cmd}' has ${nums} numbers`)
  }
  let i = 0
  while (i < d.length) {
    PATH_TOKEN.lastIndex = i
    const m = PATH_TOKEN.exec(d)
    if (!m || m[0].length === 0) { out.push(`unexpected '${d.slice(i, i + 8)}'`); break }
    i = PATH_TOKEN.lastIndex
    if (m[1]) {
      if (!cmd && m[1].toUpperCase() !== 'M') out.push('must start with M')
      close()
      cmd = m[1]
      nums = 0
    } else {
      if (!cmd) { out.push('number before the first command'); break }
      nums++
    }
  }
  close()
  if (!cmd) out.push('empty path')
  return out
}

/** Every problem of a symbol file, each naming its field. `file` is the file name (without `.json`) when known. */
export function validateSymbolFile(s: SymbolFile, file?: string): string[] {
  const p: string[] = []
  if (!/^[a-z][a-z0-9_]{1,63}$/.test(s.key ?? '')) p.push(`key '${s.key}' must be lowercase letters, digits and '_'`)
  if (file != null && file !== s.key) p.push(`key '${s.key}' must equal the file name '${file}'`)
  if (s.format != null && s.format !== SYMBOL_FORMAT) p.push(`format must be ${SYMBOL_FORMAT}`)
  if (!s.name?.trim()) p.push('name is empty')
  if (!s.category?.trim()) p.push('category is empty')
  if (!BINDINGS.includes(s.binding)) p.push(`binding '${s.binding}' is not one of ${BINDINGS.join(', ')}`)
  const [vw, vh] = s.viewBox ?? []
  if (!(vw > 0 && vh > 0)) p.push('viewBox must be two positive numbers')
  if (!(s.size?.[0] > 0 && s.size?.[1] > 0)) p.push('size must be two positive numbers')
  const inside = (x: number, y: number) => x >= 0 && y >= 0 && x <= vw && y <= vh
  for (const [side, pt] of Object.entries(s.ports ?? {})) {
    if (!SIDES.includes(side as PortSide)) p.push(`ports.${side} is not a side`)
    else if (!Array.isArray(pt) || pt.length !== 2 || !inside(pt[0], pt[1])) p.push(`ports.${side} is outside the viewBox`)
  }
  for (const [i, a] of (s.anchors ?? []).entries()) {
    if (!inside(a.x, a.y)) p.push(`anchors[${i}] '${a.id}' is outside the viewBox`)
    if (!SIDES.includes(a.dir)) p.push(`anchors[${i}].dir '${a.dir}' is not a side`)
  }
  if (!Array.isArray(s.layers) || s.layers.length === 0) p.push('layers is empty')
  const optKeys = new Set((s.options ?? []).map(o => o.key))
  const checkCond = (at: string, c?: Condition) => {
    for (const k of Object.keys(c ?? {})) if (!optKeys.has(k)) p.push(`${at}.if names '${k}', which is not an option`)
  }
  for (const [i, l] of (s.layers ?? []).entries()) {
    if (!PAINTS.includes(l.paint)) p.push(`layers[${i}].paint '${l.paint}' is not one of ${PAINTS.join(', ')}`)
    for (const e of pathProblems(l.d ?? '')) p.push(`layers[${i}].d: ${e}`)
    checkCond(`layers[${i}]`, l.if)
  }
  if (s.level) {
    if (s.binding !== 'level') p.push('level is only for binding level')
    if (s.level.clip) for (const e of pathProblems(s.level.clip)) p.push(`level.clip: ${e}`)
  } else if (s.binding === 'level') p.push('binding level needs a level area')
  for (const [i, t] of (s.text ?? []).entries()) {
    if (t.text == null && !t.from) p.push(`text[${i}] needs text or from`)
    if (t.from && t.from !== '$level' && !optKeys.has(t.from)) p.push(`text[${i}].from '${t.from}' is not an option`)
    checkCond(`text[${i}]`, t.if)
  }
  for (const [i, o] of (s.options ?? []).entries()) {
    if (o.type === 'select') {
      if (!o.values?.length) p.push(`options[${i}] '${o.key}' has no values`)
      else if (!o.values.includes(String(o.default))) p.push(`options[${i}] '${o.key}' default '${o.default}' is not one of its values`)
    } else if (o.type === 'boolean' && typeof o.default !== 'boolean') p.push(`options[${i}] '${o.key}' default must be true or false`)
  }
  return p
}
