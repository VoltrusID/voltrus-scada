// SCADA-17 W12: the LED display drew segment "a" three times (two extra conditional lines on
// top of the normal one), stacking its opacity. Each of the seven segments is drawn once.
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { test } from 'node:test'
import { ledSegmentLines } from '../src/scada2/widgets/ledSegments.ts'

test('a digit has seven distinct segment lines', () => {
  const lines = ledSegmentLines(28)
  assert.equal(lines.length, 7)
  const keys = new Set(lines.map((l) => `${l.x1},${l.y1},${l.x2},${l.y2}`))
  assert.equal(keys.size, 7)
})

test('segment a is the top bar and moves with the digit offset', () => {
  const [a] = ledSegmentLines(0)
  assert.deepEqual(a, { x1: 3, y1: 1, x2: 19, y2: 1 })
  const [shifted] = ledSegmentLines(56)
  assert.deepEqual(shifted, { x1: 59, y1: 1, x2: 75, y2: 1 })
})

test('the widget draws the segment lines once, with no extra conditional copy', async () => {
  const src = await readFile(new URL('../src/scada2/widgets/LEDDisplayWidget.tsx', import.meta.url), 'utf8')
  assert.equal((src.match(/<line\b/g) ?? []).length, 1, 'one <line> element, mapped over ledSegmentLines')
})
