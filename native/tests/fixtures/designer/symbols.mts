// Writes symbols.json: the symbol library as the web client reads it, so
// `mimic::library::tests` checks the native loader against the web code
// itself. CI runs this and fails when the file it writes differs from the
// one committed (native.yml, designer-parity).
//
// From the repository root, with Node 23.6 or later:
//
//   node native/tests/fixtures/designer/symbols.mts
//
// What it writes, from client/src/scada2/hmi/symbolFile.ts,
// symbolStyle.ts and symbolPalette.ts:
// - the keys the web loads (dataSymbols.tsx globs symbols/*.json without
//   `_*.json` and `*.schema.json`) and each one's view box, size, binding,
//   stretch, palette flag, category and ports, with its validation problems;
// - which layers and texts show for each option value (`matches`);
// - the state styles (`symbolStyle`) for every state, run tag and badge;
// - the palette's groups and a few searches (`paletteGroups`,
//   `filterSymbols`) over the ISA-101 files.

import { readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '../../../..')
const hmi = join(root, 'client/src/scada2/hmi')
const F = await import(join(hmi, 'symbolFile.ts'))
const S = await import(join(hmi, 'symbolStyle.ts'))
const P = await import(join(hmi, 'symbolPalette.ts'))

// The same selection as dataSymbols.tsx's import.meta.glob.
const dir = join(root, 'symbols')
const names = readdirSync(dir)
  .filter(n => n.endsWith('.json') && !n.startsWith('_') && !n.endsWith('.schema.json'))
  .sort()
const files = names.map(n => JSON.parse(readFileSync(join(dir, n), 'utf8')))

// Every config worth checking `matches` with: the defaults, then each
// option set to each of its values (booleans both ways).
function configs(f: any): Record<string, unknown>[] {
  const out: Record<string, unknown>[] = [{}]
  for (const o of f.options ?? []) {
    const values = o.type === 'select' ? o.values : o.type === 'boolean' ? [true, false] : ['', 'X1']
    for (const v of values) out.push({ [o.key]: v })
  }
  return out
}

const symbols = files.map((f, i) => ({
  key: f.key,
  file: names[i].replace(/\.json$/, ''),
  name: f.name,
  category: f.category,
  viewBox: f.viewBox,
  size: f.size,
  binding: f.binding,
  stretch: !!f.stretch,
  palette: f.palette !== false,
  ports: f.ports ?? {},
  problems: F.validateSymbolFile(f, names[i].replace(/\.json$/, '')),
  shown: configs(f).map(config => ({
    config,
    layers: f.layers.map((l: any, j: number) => (F.matches(l.if, config, f.options) ? j : -1)).filter((j: number) => j >= 0),
    text: (f.text ?? []).map((t: any, j: number) => (F.matches(t.if, config, f.options) ? j : -1)).filter((j: number) => j >= 0),
  })),
}))

// Colour names as the native side keeps them: `var(--isa-line)` is `isa-line`.
const name = (v: string) => v.replace(/^var\(--(.*)\)$/, '$1')
const styles: unknown[] = []
for (const state of ['idle', 'running', 'stopped', 'fault', 'stale'] as const) {
  for (const runOn of [false, true]) {
    for (const [manual, interlocked, alarmPriority] of [[false, false, null], [true, false, null], [false, true, 2], [true, true, 1], [false, false, 3]] as const) {
      const st = S.symbolStyle(state, { runState: runOn ? 'running' : 'stopped', manual, interlocked, alarmPriority })
      styles.push({
        state, runOn, manual, interlocked, alarmPriority,
        paints: Object.fromEntries(S.PAINTS.map((p: string) => {
          const x = st.paints[p]
          return [p, { stroke: name(x.stroke), width: x.strokeWidth, fill: name(x.fill), dash: x.dash ?? null, visible: x.visible, animate: !!x.animate }]
        })),
        blink: st.blink,
        text: name(st.text),
        textOnBody: name(st.textOnBody),
        badges: st.badges.map((b: any) => ({ kind: b.kind, text: b.text, shape: b.shape, fill: name(b.fill), stroke: name(b.stroke), color: name(b.color), blink: !!b.blink })),
      })
    }
  }
}

// The palette over the ISA-101 files, as the web lists them (library 'isa').
const entries = files.map(f => ({ key: f.key, name: f.name, category: f.category, library: 'isa', keywords: f.keywords, palette: f.palette }))
const rank = (c: string) => { const i = (F.ISA_CATEGORIES as readonly string[]).indexOf(c); return i < 0 ? F.ISA_CATEGORIES.length : i }
entries.sort((a, b) => rank(a.category) - rank(b.category) || a.name.localeCompare(b.name))
const searches = ['', 'valve', 'pump', 'isa_pump', 'isa_pum', 'tank cone', 'isa-101 heat', 'piping', 'nothing matches this'].map(query => ({
  query,
  keys: (query ? P.filterSymbols(entries, query, 'isa') : P.filterSymbols(entries, '', 'isa')).map((s: any) => s.key),
}))

const out = {
  keys: files.map(f => f.key),
  order: entries.map(e => e.key),
  groups: P.paletteGroups(entries).map((g: any) => g.label),
  searches,
  symbols,
  styles,
}
writeFileSync(join(here, 'symbols.json'), JSON.stringify(out, null, 1) + '\n')
console.log(`symbols.json: ${files.length} symbols, ${styles.length} styles`)
