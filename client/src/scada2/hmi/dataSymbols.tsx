import type { ConfigFieldDef } from '../widgets/types'
import type { SymbolDef, SymbolRenderProps } from './symbols'
import { ISA_CATEGORIES, matches, type SymbolFile, type SymbolText } from './symbolFile'
import { symbolStyle, type Paint } from './symbolStyle'

// The ISA-101 library as data: every symbols/<key>.json (repository root) is
// turned into a SymbolDef, so ports, faceplates, the 3D twin and saved rows
// work as for the symbols written in code. The files are the one source for
// the web and the native console (docs/designer.md, "Symbol files").

const files = import.meta.glob<SymbolFile>(['../../../../symbols/*.json', '!../../../../symbols/_*.json', '!../../../../symbols/*.schema.json'], { eager: true, import: 'default' })

const STATE_FIELDS: ConfigFieldDef[] = [
  { key: 'manualSensorId', label: 'Manual Mode Tag', type: 'tag', default: null },
  { key: 'interlockSensorId', label: 'Interlock Tag', type: 'tag', default: null },
]

/** Inspector fields of a data symbol: its options, the level %, its run tag and (state symbols) the mode and interlock tags. */
export function fieldsOf(f: SymbolFile): ConfigFieldDef[] {
  const out: ConfigFieldDef[] = []
  for (const o of f.options ?? []) {
    out.push(o.type === 'select'
      ? { key: o.key, label: o.label, type: 'select', default: o.default, options: (o.values ?? []).map(v => ({ value: v, label: v })) }
      : { key: o.key, label: o.label, type: o.type, default: o.default })
  }
  if (f.runTag) out.push({ key: 'runSensorId', label: f.runTag, type: 'tag', default: null })
  if (f.binding === 'state') out.push(...STATE_FIELDS)
  return out
}

function textOf(t: SymbolText, f: SymbolFile, p: SymbolRenderProps): string | null {
  if (t.text != null) return t.text
  if (t.from === '$level') return p.level != null && p.config.showPercent !== false ? `${Math.round(p.level * 100)}%` : null
  const v = p.config[t.from!] ?? f.options?.find(o => o.key === t.from)?.default
  return v == null || v === '' ? null : String(v)
}

function DataSymbol({ f, p }: { f: SymbolFile; p: SymbolRenderProps }) {
  const st = symbolStyle(p.state, { runState: p.runState, manual: p.manual, interlocked: p.interlocked, alarmPriority: p.alarmPriority })
  const opts = f.options ?? []
  const layers = f.layers.filter(l => matches(l.if, p.config, opts))
  const draw = (paint: Paint, d: string, key: string, width?: number, part: 'all' | 'fill' | 'stroke' = 'all') => {
    const s = st.paints[paint]
    if (!s.visible) return null
    return (
      <path key={key} d={d}
        fill={part === 'stroke' ? 'none' : s.fill}
        stroke={part === 'fill' ? 'none' : s.stroke}
        strokeWidth={width != null ? width + (p.state === 'fault' ? 0.75 : 0) : s.strokeWidth}
        strokeDasharray={s.dash} className={s.animate}
        vectorEffect="non-scaling-stroke" strokeLinejoin="round" strokeLinecap={paint === 'flow' ? undefined : 'round'} />
    )
  }
  const lv = f.level
  const clipId = `${p.uid}-lv`
  // Text stays upright and readable when the symbol is rotated or mirrored.
  const rot = Number(p.config.rotation ?? 0)
  const fx = p.config.flipX ? -1 : 1
  const fy = p.config.flipY ? -1 : 1
  const upright = (x: number, y: number) =>
    rot || fx < 0 || fy < 0 ? `translate(${x} ${y}) scale(${fx} ${fy}) rotate(${-rot}) translate(${-x} ${-y})` : undefined

  return (
    <g className={st.blink ? 'hmi-blink' : undefined}>
      {lv && (
        <>
          {lv.clip && <defs><clipPath id={clipId}><path d={lv.clip} /></clipPath></defs>}
          {layers.map((l, i) => (l.paint === 'shell' ? draw(l.paint, l.d, `f${i}`, l.width, 'fill') : null))}
          {p.level != null && (
            <rect x={lv.x} y={lv.y + lv.h * (1 - p.level)} width={lv.w} height={lv.h * p.level} fill="var(--isa-level)"
              clipPath={lv.clip ? `url(#${clipId})` : undefined} style={{ transition: 'y .6s ease, height .6s ease' }} />
          )}
        </>
      )}
      {layers.map((l, i) => draw(l.paint, l.d, `l${i}`, l.width, lv && l.paint === 'shell' ? 'stroke' : 'all'))}
      {(f.text ?? []).filter(t => matches(t.if, p.config, opts)).map((t, i) => {
        const s = textOf(t, f, p)
        if (s == null) return null
        return (
          <text key={`t${i}`} x={t.x} y={t.y} textAnchor="middle" dominantBaseline="central" fontSize={t.size} fontWeight={t.weight ?? 600}
            fill={t.onBody ? st.textOnBody : st.text} transform={upright(t.x, t.y)}
            style={{ fontFamily: t.font === 'mono' ? 'var(--font-mono)' : 'var(--font-ui)' }}>{s}</text>
        )
      })}
    </g>
  )
}

/** A SymbolDef drawn from a symbol file. */
export function symbolFromFile(f: SymbolFile): SymbolDef {
  return {
    key: f.key, name: f.name, category: f.category, library: 'isa', keywords: f.keywords, palette: f.palette,
    viewBox: f.viewBox, size: f.size, binding: f.binding, stretch: f.stretch, ports: f.ports,
    fields: fieldsOf(f),
    render: p => <DataSymbol f={f} p={p} />,
  }
}

/** Every symbol file, ISA-101 category order, then name. */
const rank = (c: string) => { const i = (ISA_CATEGORIES as readonly string[]).indexOf(c); return i < 0 ? ISA_CATEGORIES.length : i }
export const SYMBOL_FILES: SymbolFile[] = Object.values(files)
  .filter(f => typeof f?.key === 'string')
  .sort((a, b) => rank(a.category) - rank(b.category) || a.name.localeCompare(b.name))

export const DATA_SYMBOLS: SymbolDef[] = SYMBOL_FILES.map(symbolFromFile)
