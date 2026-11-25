import { test } from 'node:test'
import assert from 'node:assert/strict'
import { compile, evaluate, ExprError, parse, refsOf } from '../src/designer/expr.ts'
import {
  applyBindings, bindingErrors, bindingRefs, evaluateBinding, liveReader, refIndex, tagRef, withBinding, type Binding,
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
