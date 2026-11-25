import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import path from 'node:path'
import { ISA_CATEGORIES, matches, pathProblems, validateSymbolFile, type SymbolFile } from '../src/scada2/hmi/symbolFile.ts'
import { STYLE_RULES, alarmPriorityOf, symbolStyle, v } from '../src/scada2/hmi/symbolStyle.ts'
import { filterSymbols, paletteGroups, type PaletteEntry } from '../src/scada2/hmi/symbolPalette.ts'

const dir = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..', 'symbols')
const names = readdirSync(dir).filter(f => f.endsWith('.json') && !f.startsWith('_') && !f.endsWith('.schema.json'))
const files = names.map(f => ({ file: f.replace(/\.json$/, ''), s: JSON.parse(readFileSync(path.join(dir, f), 'utf8')) as SymbolFile }))
const byKey = new Map(files.map(f => [f.s.key, f.s]))

// ── The files ──

test('every symbol file is valid and named after its key', () => {
  assert.ok(files.length >= 35, `only ${files.length} symbol files`)
  for (const { file, s } of files) assert.deepEqual(validateSymbolFile(s, file), [], file)
  assert.equal(byKey.size, files.length, 'keys are unique')
})

test('the library has the symbols equipment types refer to', () => {
  const contract = [
    'isa_pump_centrifugal', 'isa_pump_pd', 'isa_pump_dosing', 'isa_compressor', 'isa_blower', 'isa_fan', 'isa_motor', 'isa_agitator',
    'isa_valve_gate', 'isa_valve_ball', 'isa_valve_butterfly', 'isa_valve_check', 'isa_valve_control', 'isa_valve_3way', 'isa_valve_solenoid',
    'isa_tank_vertical', 'isa_tank_horizontal', 'isa_tank_cone', 'isa_silo', 'isa_clarifier', 'isa_hx_shell_tube', 'isa_hx_plate',
    'isa_filter', 'isa_cyclone', 'isa_instrument', 'isa_pipe', 'isa_pipe_vertical', 'isa_junction', 'isa_reducer', 'isa_flow_arrow',
  ]
  for (const k of contract) assert.ok(byKey.get(k)?.palette !== false && byKey.has(k), k)
  for (const { s } of files) assert.ok((ISA_CATEGORIES as readonly string[]).includes(s.category), `${s.key}: ${s.category}`)
})

test('phase-1 symbols keep their size, ports and binding, so saved screens draw and connect the same', () => {
  const was: Record<string, Pick<SymbolFile, 'viewBox' | 'size' | 'binding' | 'ports'> & { stretch?: boolean }> = {
    isa_pump: { viewBox: [100, 100], size: [90, 104], binding: 'state', ports: { left: [12, 54], right: [94, 18] } },
    isa_motor: { viewBox: [100, 100], size: [80, 96], binding: 'state', ports: { right: [96, 50], top: [50, 4] } },
    isa_valve: { viewBox: [100, 70], size: [90, 92], binding: 'state', ports: { left: [4, 45], right: [96, 45] } },
    isa_control_valve: { viewBox: [100, 100], size: [80, 100], binding: 'value', ports: { left: [4, 75], right: [96, 75] } },
    isa_tank: { viewBox: [100, 160], size: [90, 144], binding: 'level', ports: { top: [50, 4], bottom: [50, 156], left: [4, 130], right: [96, 130] } },
    isa_heat_exchanger: { viewBox: [100, 100], size: [80, 80], binding: 'none', ports: { left: [4, 50], right: [96, 50], top: [50, 4], bottom: [50, 96] } },
    isa_line: { viewBox: [200, 10], size: [200, 10], binding: 'none', stretch: true },
    isa_line_vertical: { viewBox: [10, 200], size: [10, 200], binding: 'none', stretch: true },
  }
  for (const [k, w] of Object.entries(was)) {
    const s = byKey.get(k)
    assert.ok(s, k)
    assert.deepEqual({ viewBox: s.viewBox, size: s.size, binding: s.binding, ...(s.ports ? { ports: s.ports } : {}), ...(s.stretch ? { stretch: true } : {}) }, w, k)
  }
})

test('the shared style file says what the code does', () => {
  const { $comment, ...rules } = JSON.parse(readFileSync(path.join(dir, '_styles.json'), 'utf8'))
  assert.ok($comment)
  assert.deepEqual(rules, JSON.parse(JSON.stringify(STYLE_RULES)))
  const idle = symbolStyle('idle')
  assert.equal(idle.paints.detail.strokeWidth, STYLE_RULES.paints.detail.width)
  assert.equal(idle.paints.body.strokeWidth, STYLE_RULES.states.idle.width)
  assert.equal(symbolStyle('fault').paints.body.strokeWidth, STYLE_RULES.states.fault.width)
  // Every colour name the rules use has a value in both themes.
  const used = new Set<string>()
  JSON.stringify({ states: STYLE_RULES.states, paints: STYLE_RULES.paints, text: STYLE_RULES.text, badges: STYLE_RULES.badges }, (k, val) => {
    if (k !== 'animate' && typeof val === 'string' && /^(isa|hmi)-/.test(val)) used.add(val)
    return val
  })
  for (const theme of ['dark', 'light'] as const) for (const c of used) assert.ok(c in STYLE_RULES.colors[theme], `${theme} ${c}`)
})

test('path strings: commands and numbers only, starting with M, right argument counts', () => {
  assert.deepEqual(pathProblems('M0 0 L10 10 H5 V2 A4 4 0 1 0 8 8 Z'), [])
  assert.deepEqual(pathProblems('M-1.5e1,2 l3 4 3 4z'), [])
  assert.ok(pathProblems('L0 0').some(e => e.includes('must start with M')))
  assert.ok(pathProblems('M0 0 L5').some(e => e.includes("'L' has 1 numbers")))
  assert.ok(pathProblems('M0 0 <script>').some(e => e.includes('unexpected')))
  assert.ok(pathProblems('').length > 0)
})

test('validation names the field', () => {
  const s = structuredClone(byKey.get('isa_valve_gate')!)
  s.ports = { left: [-1, 74], right: [98, 74] }
  s.options = [{ key: 'actuator', label: 'A', type: 'select', values: ['none'], default: 'motor' }]
  s.layers = [...s.layers, { d: 'M0 0 L1', paint: 'glow' as never, if: { colour: 'red' } }]
  const p = validateSymbolFile(s, 'other').join('\n')
  for (const want of ["must equal the file name 'other'", 'ports.left is outside the viewBox', "default 'motor' is not one of its values",
    "paint 'glow'", "names 'colour', which is not an option", "'L' has 1 numbers"]) assert.ok(p.includes(want), `${want}\n${p}`)
})

test('layer conditions: option values, lists, defaults', () => {
  const opts = byKey.get('isa_valve_control')!.options
  assert.equal(matches({ actuator: 'diaphragm' }, {}, opts), true, 'default actuator of a control valve')
  assert.equal(matches({ actuator: 'motor' }, {}, opts), false)
  assert.equal(matches({ actuator: ['motor', 'solenoid'] }, { actuator: 'solenoid' }, opts), true)
  assert.equal(matches(undefined, {}, opts), true)
})

// ── State styles (ISA-101) ──

test('running is filled, stopped and idle are hollow, both grey', () => {
  const run = symbolStyle('running'), stop = symbolStyle('stopped'), idle = symbolStyle('idle')
  assert.equal(run.paints.body.fill, v('isa-on'))
  assert.equal(stop.paints.body.fill, v('isa-off'))
  assert.equal(idle.paints.body.fill, v('isa-off'))
  for (const s of [run, stop, idle]) {
    assert.equal(s.paints.body.stroke, v('isa-line'))
    assert.equal(s.blink, false)
    assert.equal(s.paints.body.dash, undefined)
    assert.deepEqual(s.badges, [])
  }
  // Shells (vessels, actuators) never fill with state; inner details contrast with a filled body.
  assert.equal(run.paints.shell.fill, v('isa-off'))
  assert.equal(run.paints.inner.stroke, v('hmi-stage'))
  assert.equal(stop.paints.inner.stroke, v('isa-line'))
  assert.equal(run.textOnBody, v('hmi-stage'))
})

test('colour only for abnormal: fault is red and blinks, no data is magenta and dashed', () => {
  const f = symbolStyle('fault')
  assert.equal(f.paints.body.fill, v('hmi-fault'))
  assert.equal(f.paints.body.stroke, v('hmi-fault'))
  assert.ok(f.paints.body.strokeWidth > symbolStyle('running').paints.body.strokeWidth)
  assert.equal(f.blink, true)
  const s = symbolStyle('stale')
  assert.equal(s.paints.body.stroke, v('hmi-stale'))
  assert.equal(s.paints.body.dash, '4 3')
  assert.equal(s.paints.body.fill, v('isa-off'))
  const normal = ['idle', 'running', 'stopped'] as const
  for (const st of normal) {
    const all = JSON.stringify(symbolStyle(st))
    assert.ok(!/fault|stale|alarm/.test(all), `${st} uses no abnormal colour`)
  }
})

test('flow layers draw only while the run tag is on, animated', () => {
  assert.equal(symbolStyle('idle').paints.flow.visible, false)
  const on = symbolStyle('idle', { runState: 'running' }).paints.flow
  assert.equal(on.visible, true)
  assert.equal(on.animate, 'hmi-flow')
  assert.ok(symbolStyle('idle', { runState: 'running' }).paints.run.strokeWidth > symbolStyle('idle').paints.run.strokeWidth)
})

test('manual and interlock are grey letter boxes; alarm priority has its own shape, colour and number', () => {
  const s = symbolStyle('running', { manual: true, interlocked: true, alarmPriority: 1 })
  assert.deepEqual(s.badges.map(b => [b.kind, b.text, b.shape]), [['alarm', '1', 'square'], ['manual', 'M', 'box'], ['interlock', 'I', 'box']])
  for (const b of s.badges.filter(x => x.kind !== 'alarm')) {
    assert.equal(b.fill, v('isa-off'))
    assert.equal(b.stroke, v('isa-line'))
  }
  assert.equal(s.badges[0].fill, v('isa-alarm-1'))
  assert.equal(s.badges[0].blink, true)
  const shapes = ([1, 2, 3] as const).map(p => symbolStyle('idle', { alarmPriority: p }).badges[0])
  assert.deepEqual(shapes.map(b => b.shape), ['square', 'triangle', 'diamond'], 'priority reads without colour')
  assert.deepEqual(shapes.map(b => b.text), ['1', '2', '3'])
  assert.equal(new Set(shapes.map(b => b.fill)).size, 3)
  assert.equal(alarmPriorityOf('crit'), 1)
  assert.equal(alarmPriorityOf('warn'), 2)
  assert.equal(alarmPriorityOf('ok'), null)
})

// ── Palette ──

const entries: PaletteEntry[] = [
  ...files.map(({ s }) => ({ key: s.key, name: s.name, category: s.category, keywords: s.keywords, palette: s.palette, library: 'isa' as const })),
  { key: 'pump', name: 'Centrifugal Pump', category: 'Rotating', library: 'shaded' },
  { key: 'breaker', name: 'Circuit Breaker', category: 'Electrical', library: 'shaded' },
]

test('palette groups: ISA-101 first (all, then each category in order), then the shaded library', () => {
  const g = paletteGroups(entries)
  assert.equal(g[0].id, 'isa')
  assert.deepEqual(g.filter(x => x.library === 'isa' && x.category).map(x => x.category), [...ISA_CATEGORIES])
  assert.deepEqual(g.filter(x => x.library === 'shaded').map(x => x.id), ['shaded:Rotating', 'shaded:Electrical'])
})

test('palette browsing hides phase-1 duplicates; a category shows only its symbols', () => {
  const all = filterSymbols(entries, '', 'isa').map(s => s.key)
  assert.ok(all.includes('isa_pump_centrifugal'))
  assert.ok(!all.includes('isa_pump') && !all.includes('isa_line'), 'hidden from browsing')
  assert.ok(!all.includes('pump'), 'shaded library not in ISA-101')
  const valves = filterSymbols(entries, '', 'isa:Valves')
  assert.ok(valves.length >= 7 && valves.every(s => s.category === 'Valves'))
  assert.deepEqual(filterSymbols(entries, '', 'shaded:Electrical').map(s => s.key), ['breaker'])
})

test('palette search: name, key, keywords and category, all words, ISA-101 first', () => {
  const keys = (q: string) => filterSymbols(entries, q, 'isa:Valves').map(s => s.key)
  assert.ok(keys('metering').includes('isa_pump_dosing'), 'keyword, outside the chosen category')
  assert.deepEqual(keys('pump').slice(-1), ['pump'], 'shaded after ISA-101')
  assert.ok(keys('pump')[0].startsWith('isa_'))
  assert.deepEqual(keys('valve ball'), ['isa_valve_ball'])
  assert.ok(keys('heat transfer').includes('isa_hx_plate'), 'category')
  assert.ok(keys('fcv').includes('isa_valve_control'))
  assert.deepEqual(keys('isa_line'), ['isa_line'], 'a hidden symbol by its exact key')
  assert.deepEqual(keys('zzz'), [])
})
