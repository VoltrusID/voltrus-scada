// The phone operator layout (docs/designer.md, phase 5): sections by area,
// reading order, what is dropped, and the engineer's adjustments
// (`config.mobile`) surviving regeneration.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  adjustedCount, classify, generateOperatorLayout, mergeOrder, mobileOverride, movePatch, readingOrder, resetPatch,
  setHiddenPatch, setSpanPatch, visibleLayout, type GenerateOptions, type MobilePatches,
} from '../src/scada2/mobile/operatorLayout.ts'
import {
  ageParts, asSensorData, clearSnapshots, isLiveValue, loadSnapshot, MAX_SCREENS, pickValues, saveSnapshot, type ScreenSnapshot,
} from '../src/scada2/mobile/lastKnown.ts'
import type { WidgetInstance } from '../src/scada2/widgets/types.ts'

type W = WidgetInstance
const w = (id: string, type: string, x: number, y: number, extra: Partial<W> = {}): W =>
  ({ id, type: type as W['type'], sensorId: null, sensorIds: [], x, y, width: 80, height: 60, config: {}, ...extra })
const tag = (id: number): Partial<W> => ({ sensorId: id, sensorIds: [id] })
const area = (id: string, x: number, y: number, width: number, height: number, title: string, config: Record<string, any> = {}) =>
  w(id, 'area', x, y, { width, height, config: { title, ...config } })
const sym = (id: string, x: number, y: number, symbol: string, extra: Partial<W> = {}) =>
  w(id, 'symbol', x, y, { ...extra, config: { symbol, ...(extra.config ?? {}) } })

const LIB: GenerateOptions = {
  symbol: key => ({
    isa_pump: { binding: 'state' as const, category: 'Rotating equipment' },
    isa_tank_vertical: { binding: 'level' as const, category: 'Vessels' },
    isa_junction: { binding: 'none' as const, category: 'Piping' },
    isa_arrow: { binding: 'none' as const, category: 'Piping' },
  })[key],
}

/** Plant: two areas side by side, things outside them, decoration everywhere. */
function plant(): W[] {
  return [
    w('title', 'label', 20, 10, { config: { text: 'Plant overview' } }),
    w('kpi', 'big_number', 600, 10, tag(9)),
    area('a_raw', 20, 100, 600, 500, 'Raw water'),
    area('a_filt', 700, 100, 600, 500, 'Filtration'),
    // Raw water: tank to the right of the pump on one row, then a trend below.
    sym('p1', 60, 200, 'isa_pump', tag(1)),
    sym('t1', 300, 180, 'isa_tank_vertical', { ...tag(2), height: 120 }),
    w('tr', 'trend', 60, 400, { ...tag(2), width: 500, height: 150 }),
    sym('j1', 200, 220, 'isa_junction'),
    w('pipe1', 'pipe', 140, 230),
    w('lbl', 'label', 60, 150, { config: { text: 'Intake' } }),
    w('lblb', 'label', 160, 150, { config: { text: 'x', bindings: { text: { kind: 'tag', tag: 'wtp/x' } } } }),
    // Filtration: an equipment instance with no primary tag yet, a table, a static tank drawing, a control.
    sym('f1', 740, 200, 'isa_pump', { config: { symbol: 'isa_pump', equipment: 'P-201' } }),
    w('tbl', 'data_table', 740, 400, { sensorIds: [3, 4], width: 400, height: 150 }),
    sym('deco', 1000, 200, 'isa_tank_vertical'),
    w('sp', 'slider', 1000, 300, tag(5)),
  ]
}

const ids = (layout: ReturnType<typeof generateOperatorLayout>) =>
  layout.sections.map(s => `${s.id ?? '-'}:${s.items.map(i => i.id + (i.hidden ? '(h)' : '')).join(',')}`)

/** Apply patches as the Designer's patch edit does (a key set to undefined is removed). */
function apply(ws: W[], patches: MobilePatches): W[] {
  return ws.map(x => {
    const p = patches[x.id]
    if (!p) return x
    const config = { ...x.config }
    for (const [k, v] of Object.entries(p.config)) { if (v === undefined) delete config[k]; else config[k] = v }
    return { ...x, config }
  })
}

test('components are sorted into sections by area, in reading order, with decoration dropped', () => {
  const layout = generateOperatorLayout({ widgets: plant() }, LIB)
  assert.deepEqual(ids(layout), [
    '-:title(h),kpi',
    'a_raw:lbl(h),lblb,p1,t1,tr',
    'a_filt:f1,sp,tbl',
  ])
  assert.deepEqual(layout.sections.map(s => s.title), ['', 'Raw water', 'Filtration'])
  const kinds = Object.fromEntries(layout.sections.flatMap(s => s.items).map(i => [i.id, i.kind]))
  assert.deepEqual(kinds, {
    title: 'note', kpi: 'value', lbl: 'note', lblb: 'note', p1: 'equipment', t1: 'equipment', tr: 'card', f1: 'equipment',
    sp: 'control', tbl: 'card',
  })
  // Pipes, piping symbols and an unbound tank drawing are not listed at all.
  const listed = new Set(layout.sections.flatMap(s => s.items.map(i => i.id)))
  for (const gone of ['pipe1', 'j1', 'deco', 'a_raw', 'a_filt']) assert.ok(!listed.has(gone), gone)
  // Operators see neither hidden items nor empty sections.
  assert.deepEqual(ids(visibleLayout(layout)), ['-:kpi', 'a_raw:lblb,p1,t1,tr', 'a_filt:f1,sp,tbl'])
})

test('classify: what each widget becomes', () => {
  assert.equal(classify(w('a', 'area', 0, 0)), 'area')
  assert.equal(classify(w('p', 'pipe', 0, 0, tag(1))), 'decoration')
  assert.equal(classify(sym('pp', 0, 0, 'pipe_horizontal', tag(1))), 'decoration', 'pipe symbols are lines even with a flow tag')
  assert.equal(classify(sym('j', 0, 0, 'isa_junction', tag(1)), LIB), 'decoration')
  assert.equal(classify(w('legacy', 'pump', 0, 0, tag(1))), 'equipment')
  assert.equal(classify(sym('f', 0, 0, 'isa_pump', { config: { symbol: 'isa_pump', faultSensorId: 4 } }), LIB), 'equipment')
  assert.equal(classify(sym('fp', 0, 0, 'isa_pump', { config: { symbol: 'isa_pump', faceplate: 'pump' } }), LIB), 'equipment')
  assert.equal(classify(w('g', 'gauge', 0, 0)), 'decoration', 'an unbound gauge shows nothing')
  assert.equal(classify(w('g', 'gauge', 0, 0, tag(1))), 'value')
  assert.equal(classify(w('l', 'label', 0, 0, tag(1))), 'value')
  assert.equal(classify(w('mt', 'multi_trend', 0, 0)), 'card')
  assert.equal(classify(w('x', 'custom:acme-dial@1.0.0', 0, 0)), 'card', 'add-on widgets are cards')
  assert.equal(classify(w('pb', 'push_button', 0, 0, tag(1))), 'control')
})

test('reading order bands rows by vertical centre, then left to right', () => {
  const box = (id: string, x: number, y: number, height = 40) => ({ id, x, y, width: 50, height })
  // b starts a little lower than a but sits on its row; c is clearly below.
  const out = readingOrder([box('c', 0, 100), box('b', 0, 10), box('a', 200, 0), box('d', 100, 112)])
  assert.deepEqual(out.map(b => b.id), ['b', 'a', 'c', 'd'])
  // Ties are broken by id, so the order is the same on every run.
  assert.deepEqual(readingOrder([box('z', 0, 0), box('y', 0, 0)]).map(b => b.id), ['y', 'z'])
})

test('nested areas: a component goes to the smallest area holding its centre', () => {
  const ws = [
    area('outer', 0, 0, 1000, 1000, 'Outer'),
    area('inner', 100, 100, 300, 300, 'Inner'),
    sym('p', 150, 150, 'isa_pump', tag(1)),
    sym('q', 600, 600, 'isa_pump', tag(2)),
  ]
  assert.deepEqual(ids(generateOperatorLayout({ widgets: ws }, LIB)), ['outer:q', 'inner:p'])
})

test('a fixed visible:false is hidden on phones too; a bound one is decided at runtime', () => {
  const ws = [
    w('a', 'big_number', 0, 0, { ...tag(1), config: { visible: false } }),
    w('b', 'big_number', 100, 0, { ...tag(2), config: { bindings: { visible: { kind: 'tag', tag: 'd/x' } } } }),
  ]
  assert.deepEqual(ids(generateOperatorLayout({ widgets: ws })), ['-:a(h),b'])
})

test('adjustments: hide, show a label, span; stored only when they differ from the generated layout', () => {
  let ws = plant()
  // Hiding the trend stores hide; showing it again removes the key.
  let p = setHiddenPatch({ widgets: ws }, 'tr', true, LIB)
  assert.deepEqual(p, { tr: { config: { mobile: { hide: true } } } })
  ws = apply(ws, p)
  assert.equal(generateOperatorLayout({ widgets: ws }, LIB).sections[1].items.find(i => i.id === 'tr')!.hidden, true)
  p = setHiddenPatch({ widgets: ws }, 'tr', false, LIB)
  assert.deepEqual(p, { tr: { config: { mobile: undefined } } })
  ws = apply(ws, p)
  assert.equal(ws.find(x => x.id === 'tr')!.config.mobile, undefined)
  // A plain label is dropped by default: showing it stores hide:false.
  p = setHiddenPatch({ widgets: ws }, 'lbl', false, LIB)
  assert.deepEqual(p, { lbl: { config: { mobile: { hide: false } } } })
  ws = apply(ws, p)
  assert.ok(visibleLayout(generateOperatorLayout({ widgets: ws }, LIB)).sections[1].items.some(i => i.id === 'lbl'))
  // Half width, then full width (the default) again.
  p = setSpanPatch({ widgets: ws }, 'p1', 1)
  ws = apply(ws, p)
  assert.equal(generateOperatorLayout({ widgets: ws }, LIB).sections[1].items.find(i => i.id === 'p1')!.span, 1)
  assert.deepEqual(setSpanPatch({ widgets: ws }, 'p1', 1), {}, 'nothing changes, nothing is patched')
  ws = apply(ws, setSpanPatch({ widgets: ws }, 'p1', 2))
  assert.equal(ws.find(x => x.id === 'p1')!.config.mobile, undefined)
  // Hiding a whole section hides it from operators.
  ws = apply(ws, setHiddenPatch({ widgets: ws }, 'a_filt', true, LIB))
  assert.deepEqual(visibleLayout(generateOperatorLayout({ widgets: ws }, LIB)).sections.map(s => s.id), [null, 'a_raw'])
  assert.equal(adjustedCount({ widgets: ws }), 2)
  ws = apply(ws, resetPatch({ widgets: ws }))
  assert.equal(adjustedCount({ widgets: ws }), 0)
  assert.deepEqual(ids(generateOperatorLayout({ widgets: ws }, LIB)), ids(generateOperatorLayout({ widgets: plant() }, LIB)))
})

test('reordering stores every position of the section, and moving back to the generated order removes them', () => {
  let ws = plant()
  let p = movePatch({ widgets: ws }, 'tr', -1, LIB)
  assert.deepEqual(Object.keys(p).sort(), ['lbl', 'lblb', 'p1', 't1', 'tr'])
  ws = apply(ws, p)
  assert.deepEqual(ids(generateOperatorLayout({ widgets: ws }, LIB))[1], 'a_raw:lbl(h),lblb,p1,tr,t1')
  // Out of range: nothing.
  assert.deepEqual(movePatch({ widgets: ws }, 'lbl', -1, LIB), {})
  // Back down: the generated order again, so no position is kept.
  p = movePatch({ widgets: ws }, 'tr', 1, LIB)
  ws = apply(ws, p)
  assert.equal(adjustedCount({ widgets: ws }), 0)
  // Sections move by their area.
  ws = apply(ws, movePatch({ widgets: ws }, 'a_filt', -1, LIB))
  assert.deepEqual(generateOperatorLayout({ widgets: ws }, LIB).sections.map(s => s.id), [null, 'a_filt', 'a_raw'])
  assert.deepEqual(mobileOverride(ws.find(x => x.id === 'a_filt')!.config), { order: 0 })
})

test('regeneration keeps adjustments: new components land by their generated neighbours, deleted ones leave no trace', () => {
  let ws = plant()
  ws = apply(ws, movePatch({ widgets: ws }, 'tr', -1, LIB)) // lbl, lblb, p1, tr, t1
  ws = apply(ws, setHiddenPatch({ widgets: ws }, 'p1', true, LIB))
  // The screen is edited: a new pump below the first one (generated right after p1), t1 deleted.
  ws = [...ws.filter(x => x.id !== 't1'), sym('p2', 60, 300, 'isa_pump', tag(7))]
  const layout = generateOperatorLayout({ widgets: ws }, LIB)
  assert.deepEqual(ids(layout)[1], 'a_raw:lbl(h),lblb,p1(h),p2,tr')
  // A component moved into another area takes its adjustments along and sorts by its stored position there.
  ws = ws.map(x => (x.id === 'tr' ? { ...x, x: 760, y: 560, width: 100, height: 20 } : x))
  const moved = generateOperatorLayout({ widgets: ws }, LIB)
  assert.ok(moved.sections.find(s => s.id === 'a_filt')!.items.some(i => i.id === 'tr'))
})

test('malformed or stale adjustments are ignored, never fatal', () => {
  assert.deepEqual(mobileOverride({ mobile: { hide: 'yes', order: 'first', span: 3, extra: 1 } }), {})
  assert.deepEqual(mobileOverride({ mobile: [1, 2] }), {})
  assert.deepEqual(mobileOverride({ mobile: { order: Infinity, span: 1 } }), { span: 1 })
  assert.deepEqual(mobileOverride(undefined), {})
  const ws = [
    w('a', 'big_number', 0, 0, { ...tag(1), config: { mobile: { order: 'x', hide: 1 } } }),
    w('b', 'big_number', 100, 0, { ...tag(2), config: { mobile: null } }),
    // Positions copied from components since deleted: gaps and duplicates are fine.
    w('c', 'big_number', 0, 100, { ...tag(3), config: { mobile: { order: 7 } } }),
    w('d', 'big_number', 100, 100, { ...tag(4), config: { mobile: { order: 7 } } }),
  ]
  assert.deepEqual(ids(generateOperatorLayout({ widgets: ws })), ['-:a,b,c,d'], 'a and b keep their place before c and d')
  // An area that no longer exists simply has no section; its members fall back to the loose section.
  assert.deepEqual(ids(generateOperatorLayout({ widgets: [sym('p', 0, 0, 'isa_pump', tag(1))] }, LIB)), ['-:p'])
  assert.deepEqual(generateOperatorLayout({ widgets: [] }), { sections: [] })
})

test('mergeOrder puts unordered items after their generated predecessor', () => {
  const g = ['a', 'b', 'c', 'd', 'e'].map(id => ({ id }))
  const order: Record<string, number> = { c: 0, a: 1 }
  assert.deepEqual(mergeOrder(g, x => order[x.id]).map(x => x.id), ['c', 'd', 'e', 'a', 'b'])
  assert.deepEqual(mergeOrder(g, () => undefined), g)
})

// ── Last-known values ──

function memoryStorage() {
  const m = new Map<string, string>()
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => { m.set(k, v) }, removeItem: (k: string) => { m.delete(k) }, m }
}
const snap = (screen: string, savedAt = 1000): ScreenSnapshot => ({
  screen, screenName: screen.toUpperCase(), savedAt, equipment: [], connections: [],
  sensors: [{ id: 1, key: 'level', name: 'Level', unit: '%', decimals: 1, device_id: 'wtp', min_val: 0, max_val: 100 }],
  values: { 1: { value: 42.5, ts_ms: 900, quality: 'good', status: 'ok' } },
})

test('snapshots: newest first, per user, capped, cleared on sign-out', () => {
  const s = memoryStorage()
  assert.equal(loadSnapshot(s), null)
  for (let i = 0; i < MAX_SCREENS + 2; i++) assert.ok(saveSnapshot(s, 'ana', snap(`s${i}`, i)))
  assert.equal(loadSnapshot(s)!.screen, `s${MAX_SCREENS + 1}`)
  assert.equal(loadSnapshot(s, 's0'), null, 'the oldest fell off')
  assert.equal(loadSnapshot(s, 's3', 'ana')!.screenName, 'S3')
  assert.equal(loadSnapshot(s, 's3', 'bob'), null, 'another user sees none')
  // Another user saving replaces the first user's snapshots.
  saveSnapshot(s, 'bob', snap('x'))
  assert.equal(loadSnapshot(s, 's3'), null)
  clearSnapshots(s)
  assert.equal(loadSnapshot(s), null)
  // Junk in storage reads as nothing.
  s.setItem('voltrus.operator.lastKnown.v1', '{not json')
  assert.equal(loadSnapshot(s), null)
  // A huge snapshot is not kept.
  const big = snap('big')
  big.equipment = Array.from({ length: 5000 }, (_, i) => ({ equipment_id: `e${i}`, equipment_type: 'label', equipment_name: 'x'.repeat(80), x: 0, y: 0, width: 1, height: 1, rotation: 0, sensor_bindings: '[]', config: '{}' })) as any
  assert.equal(saveSnapshot(s, 'ana', big), false)
})

test('pickValues keeps finite values of the listed tags, with their time and quality', () => {
  const cur: Record<string, any> = {
    1: { id: 1, value: 3, ts_ms: 10, quality: 'good', status: 'ok' },
    2: { id: 2, value: NaN, status: 'ok' },
    3: { id: 3, value: 9, status: 'crit' },
  }
  assert.deepEqual(pickValues([1, 2, 4], cur), { 1: { value: 3, ts_ms: 10, quality: 'good', status: 'ok' } })
  const back = asSensorData(snap('s'))
  assert.equal(back['1'].value, 42.5)
  assert.equal(back['1'].ts_ms, 900)
})

test('a value is live only while the stream is up and it is recent', () => {
  const now = 1_000_000
  assert.equal(isLiveValue(true, { ts_ms: now - 5000 }, now), true)
  assert.equal(isLiveValue(true, {}, now), true, 'no timestamp: as fresh as the stream')
  assert.equal(isLiveValue(false, { ts_ms: now }, now), false, 'offline: never live')
  assert.equal(isLiveValue(true, { ts_ms: now - 61_000 }, now), false)
  assert.equal(isLiveValue(true, undefined, now), false)
  assert.deepEqual(ageParts(2000), { n: 0, unit: 'now' })
  assert.deepEqual(ageParts(45_000), { n: 45, unit: 's' })
  assert.deepEqual(ageParts(12 * 60_000 + 5), { n: 12, unit: 'min' })
  assert.deepEqual(ageParts(3 * 3600_000), { n: 3, unit: 'h' })
  assert.deepEqual(ageParts(50 * 3600_000), { n: 2, unit: 'd' })
  assert.deepEqual(ageParts(-5), { n: 0, unit: 'now' })
})
