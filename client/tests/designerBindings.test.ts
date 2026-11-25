import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  CACHE_LIMIT, cacheSize, compile, escapeRef, evaluate, ExprError, MAX_DEPTH, MAX_SOURCE, parse, refsOf, safeEvaluate,
} from '../src/designer/expr.ts'
import {
  applyBindings, bindingErrors, bindingRefs, describeBinding, evaluateBinding, liveReader, refIndex, refsById, sharedRefIndex,
  shownAtRuntime, tagRef, withBinding, type Binding,
} from '../src/designer/bindings.ts'

const tags: Record<string, number | null> = { 'wtp/level': 87.5, 'wtp/run': 1, 'wtp/fault': 0, 'wtp/flow': 1234.567, 'wtp/none': null }
const read = (r: string) => (r in tags ? tags[r] : null)
const ev = (s: string) => evaluate(parse(s), read)

test('expressions: arithmetic, precedence, comparison, logic, ternary', () => {
  assert.equal(ev('1 + 2 * 3'), 7)
  assert.equal(ev('(1 + 2) * 3'), 9)
  assert.equal(ev('10 - 4 - 3'), 3, 'left associative')
  assert.equal(ev('-2 * 3'), -6)
  assert.equal(ev('{wtp/level} > 85'), true)
  assert.equal(ev('{wtp/level} > 85 ? "HIGH" : "OK"'), 'HIGH')
  assert.equal(ev('{wtp/run} && !{wtp/fault}'), true)
  assert.equal(ev('{wtp/fault} || {wtp/run} == 1'), true)
  assert.equal(ev('"P-101 " + text({wtp/flow}, 1)'), 'P-101 1234.6')
  assert.equal(ev('round({wtp/flow} / 100, 1)'), 12.3)
  assert.equal(ev('clamp(150, 0, 100)'), 100)
  assert.equal(ev('scale(50, 0, 100, 4, 20)'), 12)
  assert.equal(ev('bit(5, 2)'), true)
  assert.equal(ev('if({wtp/fault}, "TRIP", "ok")'), 'ok')
  assert.equal(ev('1 / 0'), null, 'division by zero has no value')
})

test('a missing tag or value gives no value instead of zero', () => {
  assert.equal(ev('{wtp/none} + 1'), null)
  assert.equal(ev('{nope/x} * 2'), null)
  assert.equal(ev('text({wtp/none})'), '')
})

test('parse errors name the problem and position', () => {
  for (const bad of ['1 +', '(1 + 2', '{wtp/level', 'foo(1)', 'round()', '1 2', '"abc', '{}']) {
    assert.throws(() => parse(bad), ExprError, bad)
  }
  const e = compile('1 +')
  assert.ok(e instanceof ExprError)
  assert.deepEqual([...refsOf(parse('{a/b} + max({c/d}, {a/b})'))], ['a/b', 'c/d'])
})

test('tag, expression and map bindings evaluate with the property type', () => {
  assert.equal(evaluateBinding({ kind: 'tag', tag: 'wtp/run' }, read, 'boolean'), true)
  assert.equal(evaluateBinding({ kind: 'tag', tag: 'wtp/level' }, read, 'string'), '87.5')
  assert.equal(evaluateBinding({ kind: 'tag', tag: 'wtp/none' }, read, 'number'), undefined)
  assert.equal(evaluateBinding({ kind: 'expr', expr: '{wtp/level} / 100' }, read, 'number'), 0.875)
  const map: Binding = {
    kind: 'map', expr: '{wtp/level}',
    cases: [{ op: '>=', v: 95, out: '#f04646' }, { op: '>=', v: 85, out: '#f5b21b' }], else: '#8b939e',
  }
  assert.equal(evaluateBinding(map, read, 'color'), '#f5b21b')
  assert.equal(evaluateBinding(map, () => 10, 'color'), '#8b939e')
  assert.equal(evaluateBinding({ kind: 'map', expr: '{wtp/run} == 1', cases: [{ op: '==', v: 1, out: 'RUN' }] }, read, 'string'), 'RUN')
})

test('applyBindings replaces bound properties and keeps the rest', () => {
  let config: Record<string, any> = { label: 'fixed', decimals: 1 }
  assert.equal(applyBindings(config, read), config, 'nothing bound: same object')
  config = withBinding(config, 'label', { kind: 'expr', expr: '{wtp/level} > 85 ? "HIGH" : "OK"' })
  config = withBinding(config, 'visible', { kind: 'tag', tag: 'wtp/fault' })
  config = withBinding(config, 'decimals', { kind: 'expr', expr: '{wtp/none}' })
  const out = applyBindings(config, read, { label: 'string', decimals: 'number' })
  assert.equal(out.label, 'HIGH')
  assert.equal(out.visible, false)
  assert.equal(out.decimals, 1, 'no value: the fixed value stays')
  assert.deepEqual(bindingRefs(config).sort(), ['wtp/fault', 'wtp/level', 'wtp/none'])
  // Unbinding the last property removes the bindings key.
  let c2 = withBinding({ a: 1 }, 'label', { kind: 'tag', tag: 'x/y' })
  c2 = withBinding(c2, 'label', null)
  assert.deepEqual(c2, { a: 1 })
})

test('refs are device/key, as in the project format, and errors name unknown tags', () => {
  const sensors = [{ id: 4, key: 'level', device_id: 'wtp' }, { id: 9, key: 'calc', device_id: '' }]
  assert.equal(tagRef(sensors[0]), 'wtp/level')
  assert.equal(tagRef(sensors[1]), '/calc')
  const idx = refIndex(sensors)
  assert.equal(liveReader(idx, { '4': { value: 12 } })('wtp/level'), 12)
  assert.equal(liveReader(idx, {})('wtp/level'), null)
  assert.deepEqual(bindingErrors({ kind: 'tag', tag: 'wtp/level' }, idx), [])
  assert.deepEqual(bindingErrors({ kind: 'expr', expr: '{wtp/level} + {wtp/gone}' }, idx), ['Unknown tag {wtp/gone}'])
  assert.equal(bindingErrors({ kind: 'expr', expr: '1 +' }, idx).length, 1)
})

test('adversarial expressions never throw while evaluating; they give no value', () => {
  const live: Record<string, number> = { 'd/a': 7, 'd/b': 0, 'd/k': 3.14159, 'd/decimals': 1e9 }
  const r = (ref: string) => live[ref] ?? null
  const cases: [string, unknown][] = [
    ['text({d/a}/{d/b}, 101)', ''],            // division by zero (no value), absurd decimals
    ['text(1.5, 101)', '1.50000000000000000000'], // decimals clamped to 20
    ['text(1.5, -1)', '2'],                    // and to 0
    ['text({d/k}, {d/decimals})', (3.14159).toFixed(20)],
    ['text(1/0 + 1, 2)', ''],
    ['round(2.345, 400)', 2.345],
    ['round(1234.5, -3)', 1235],
    ['bit(-1, 0)', true],
    ['bit(-2, 0)', false],
    ['bit(-2, 1)', true],
    ['bit(5, 99)', null],
    ['bit(5, 1.5)', null],
    ['"1" == 1', false],
    ['true == 1', true],
    ['scale(1, 2, 2, 0, 1)', null],
  ]
  for (const [src, want] of cases) {
    const n = parse(src)
    assert.doesNotThrow(() => safeEvaluate(n, r), src)
    assert.deepEqual(safeEvaluate(n, r), want, src)
  }
  // Every binding kind swallows whatever goes wrong.
  const throwing = () => { throw new Error('boom') }
  for (const b of [
    { kind: 'tag', tag: 'd/a' }, { kind: 'expr', expr: '{d/a} + 1' }, { kind: 'map', expr: '{d/a}', cases: [{ op: '>=', v: 1, out: 'x' }] },
  ] as Binding[]) assert.equal(evaluateBinding(b, throwing, 'string'), undefined, b.kind)
  // Saved data of the wrong shape.
  for (const bad of [null, 3, 'x', { kind: 'expr' }, { kind: 'map', expr: '1' }, { kind: 'nope', expr: '1' }, { kind: 'map', expr: '1', cases: 'x' }]) {
    assert.doesNotThrow(() => evaluateBinding(bad as any, r, 'string'))
    assert.doesNotThrow(() => applyBindings({ bindings: { label: bad } }, r))
  }
})

test('oversized or deeply nested expressions are refused at parse, not at the stack', () => {
  assert.throws(() => parse('1+'.repeat(MAX_SOURCE)), /longer than/)
  // A long left-associative chain under the length cap: too many parts or too deep.
  assert.throws(() => parse(Array(600).fill('1').join('+')), ExprError)
  assert.throws(() => parse(Array(MAX_DEPTH + 5).fill('1').join('+')), /nested deeper/)
  assert.throws(() => parse('('.repeat(200) + '1' + ')'.repeat(200)), /nested deeper/)
  assert.throws(() => parse('-'.repeat(300) + '1'), /nested deeper/)
  assert.throws(() => parse('!'.repeat(300) + '1'), /nested deeper/)
  // Within the limits, fine.
  assert.equal(evaluate(parse(Array(40).fill('1').join('+')), () => null), 40)
  assert.equal(evaluate(parse('('.repeat(20) + '1' + ')'.repeat(20)), () => null), 1)
  // compile reports instead of throwing, and keeps oversized text out of its cache.
  const before = cacheSize()
  assert.ok(compile('x'.repeat(MAX_SOURCE + 1)) instanceof ExprError)
  assert.equal(cacheSize(), before)
})

test('the parse cache is a bounded LRU', () => {
  compile('1 + 1')
  for (let i = 0; i < CACHE_LIMIT + 50; i++) {
    compile(`${i} + 0`)
    if (i % 10 === 0) compile('1 + 1') // kept warm
  }
  assert.ok(cacheSize() <= CACHE_LIMIT)
  const warm = compile('1 + 1')
  assert.equal(compile('1 + 1'), warm, 'a recently used entry survives')
})

test('tag refs with braces and backslashes are escaped and read back', () => {
  const ref = 'plc/odd}key\\x'
  const src = `${escapeRef(ref)} + 1`
  assert.deepEqual([...refsOf(parse(src))], [ref])
  assert.equal(evaluate(parse(src), r => (r === ref ? 2 : null)), 3)
  assert.equal(describeBinding({ kind: 'tag', tag: 'a/b}c' }), '{a/b\\}c}')
  // A brace inside a string literal is text, not a ref.
  assert.deepEqual([...refsOf(parse('"{x/y}" + {a/b}'))], ['a/b'])
})

test('duplicate device/key refs are numbered like the exporter, by id', () => {
  const sensors = [
    { id: 30, key: 'power', device_id: 'plts_1' }, { id: 12, key: 'power', device_id: 'plts_1' },
    { id: 20, key: 'power', device_id: 'plts_1' }, { id: 5, key: 'v', device_id: null },
  ]
  const refs = refsById(sensors)
  assert.equal(refs.get(12), 'plts_1/power')
  assert.equal(refs.get(20), 'plts_1/power#2')
  assert.equal(refs.get(30), 'plts_1/power#3')
  assert.equal(refs.get(5), '/v')
  const idx = refIndex(sensors)
  assert.equal(idx.get('plts_1/power#3'), 30)
  assert.equal(sharedRefIndex(sensors), sharedRefIndex(sensors), 'one index per sensors array')
})

test('map cases need a number; tag-picker properties are never bound', () => {
  const idx = new Map([['d/a', 1]])
  assert.deepEqual(bindingErrors({ kind: 'map', expr: '{d/a}', cases: [{ op: '>=', v: NaN, out: 'x' }] }, idx), ['Case 1 needs a number'])
  assert.equal(evaluateBinding({ kind: 'map', expr: '{d/a}', cases: [{ op: '>=', v: NaN, out: 'x' }], else: 'e' }, () => 5, 'string'), 'e')
  const config = withBinding({ runSensorId: 4 }, 'runSensorId', { kind: 'expr', expr: '9' })
  assert.equal(applyBindings(config, () => null, { runSensorId: 'tag' }).runSensorId, 4)
  // A number property never gets NaN from a binding: it keeps its fixed value.
  assert.equal(applyBindings(withBinding({ decimals: 2 }, 'decimals', { kind: 'expr', expr: '"abc"' }), () => null, { decimals: 'number' }).decimals, 2)
})

test('visibility at runtime: bound visible wins when it has a value', () => {
  const r = (ref: string) => (ref === 'd/on' ? 1 : ref === 'd/off' ? 0 : null)
  assert.equal(shownAtRuntime({}, r), true)
  assert.equal(shownAtRuntime({ visible: false }, r), false)
  assert.equal(shownAtRuntime(withBinding({ visible: false }, 'visible', { kind: 'tag', tag: 'd/on' }), r), true)
  assert.equal(shownAtRuntime(withBinding({}, 'visible', { kind: 'tag', tag: 'd/off' }), r), false)
  assert.equal(shownAtRuntime(withBinding({ visible: false }, 'visible', { kind: 'tag', tag: 'd/none' }), r), false, 'no value: fixed value')
})
