import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { STATE_COLORS, alarmLevel, framesWidget, valueStyle } from '../src/valueStyle.ts'

test('both spellings of the alarm status map to one state', () => {
  // The REST snapshot says critical/warning, the live stream crit/warn.
  for (const s of ['crit', 'critical']) assert.equal(alarmLevel(s), 'crit')
  for (const s of ['warn', 'warning']) assert.equal(alarmLevel(s), 'warn')
  assert.equal(alarmLevel(undefined), 'normal')
  assert.equal(valueStyle({ value: 9, status: 'critical' }).state, 'crit')
  assert.equal(valueStyle({ value: 9, status: 'warn' }).state, 'warn')
})

test('quality wins over the alarm level, offline and no data over both', () => {
  assert.equal(valueStyle({ value: 9, status: 'crit', quality: 'bad_comm_failure' }).state, 'bad')
  assert.equal(valueStyle({ value: 9, status: 'warn', quality: 'uncertain_stale' }).state, 'uncertain')
  assert.equal(valueStyle({ value: NaN, status: 'offline', quality: 'bad' }).state, 'offline')
  assert.equal(valueStyle(undefined).state, 'nodata')
  const normal = valueStyle({ value: 1, status: 'normal', quality: 'good' })
  assert.equal(normal.state, 'normal')
  assert.equal(normal.color, undefined, 'a normal value keeps the widget colour')
  assert.equal(framesWidget(normal), false)
  assert.equal(framesWidget(valueStyle(undefined)), false)
})

test('markers and labels tell states apart without colour', () => {
  const seen = new Set<string>()
  for (const d of [
    { value: 1, status: 'crit' }, { value: 1, status: 'warn' },
    { value: 1, quality: 'bad' }, { value: 1, quality: 'uncertain' },
  ]) {
    const s = valueStyle(d)
    assert.ok(s.label, JSON.stringify(d))
    seen.add(s.label)
  }
  assert.equal(seen.size, 4)
  assert.equal(valueStyle({ value: 1, quality: 'bad' }).marker, '✕')
  assert.equal(valueStyle({ value: 1, quality: 'uncertain' }).marker, '?')
})

test('every state colour is a theme variable that both themes define', () => {
  const css = readFileSync(new URL('../src/index.css', import.meta.url), 'utf8')
  for (const c of Object.values(STATE_COLORS)) {
    const name = c.match(/var\((--[\w-]+)\)/)?.[1]
    assert.ok(name, c)
    for (const sel of [':root', '[data-theme="light"]']) {
      const start = css.indexOf(sel)
      const block = css.slice(start, css.indexOf('\n}', start))
      assert.ok(block.includes(`${name}:`), `${name} missing in ${sel}`)
    }
  }
})
