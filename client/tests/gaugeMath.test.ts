// SCADA-16 W1/W2: the dial gauge drew its arcs on the mirrored circle (sweep-flag 0) and the
// card gauge painted a tag without limits (warn=crit=0) as a fully red arc.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { dialArcPath, gaugeBands, polarToCartesian } from '../src/gaugeMath.ts'

// Parse "M sx sy A r r 0 large sweep ex ey".
function parse(path: string) {
  const m = path.match(/^M (\S+) (\S+) A (\S+) (\S+) 0 (\d) (\d) (\S+) (\S+)$/)
  assert.ok(m, path)
  const n = m.slice(1).map(Number)
  return { sx: n[0], sy: n[1], r: n[2], large: n[4], sweep: n[5], ex: n[6], ey: n[7] }
}

// Center of the circle through both endpoints for the arc flags SVG would use.
function arcCenter(p: ReturnType<typeof parse>) {
  const { sx, sy, ex, ey, r, large, sweep } = p
  const mx = (sx + ex) / 2, my = (sy + ey) / 2
  const dx = ex - sx, dy = ey - sy
  const d = Math.hypot(dx, dy)
  const h = Math.sqrt(Math.max(0, r * r - (d / 2) ** 2))
  // Unit normal; pick the candidate that SVG's flag combination selects.
  const nx = -dy / d, ny = dx / d
  const sign = large === sweep ? -1 : 1
  return { x: mx + sign * h * nx, y: my + sign * h * ny }
}

test('dial arc runs clockwise (sweep 1) on the circle centred on the dial', () => {
  const full = parse(dialArcPath(100, 100, 75, 225, -45))
  assert.equal(full.sweep, 1)
  assert.equal(full.large, 1)
  const c = arcCenter(full)
  assert.ok(Math.abs(c.x - 100) < 1e-6 && Math.abs(c.y - 100) < 1e-6, `centre ${c.x},${c.y}`)
})

test('dial arc spans of at most 180 degrees use the small arc, longer ones the large arc', () => {
  const small = parse(dialArcPath(100, 100, 75, 225, 90))
  assert.equal(small.large, 0)
  assert.equal(small.sweep, 1)
  const c = arcCenter(small)
  assert.ok(Math.abs(c.x - 100) < 1e-6 && Math.abs(c.y - 100) < 1e-6)
  assert.equal(parse(dialArcPath(100, 100, 75, 225, 40)).large, 1)
})

test('dial arc endpoints match the tick/needle polar mapping', () => {
  const p = parse(dialArcPath(100, 100, 75, 225, -45))
  const s = polarToCartesian(100, 100, 75, 225)
  assert.ok(Math.abs(p.sx - s.x) < 1e-9 && Math.abs(p.sy - s.y) < 1e-9)
})

test('tag without limits has no bands', () => {
  assert.equal(gaugeBands(0, 100, 0, 0), null)
})

test('tag with limits gets clamped band fractions', () => {
  assert.deepEqual(gaugeBands(0, 100, 75, 90), { warnPct: 0.75, critPct: 0.9 })
  assert.deepEqual(gaugeBands(0, 100, 0, 90), { warnPct: 0, critPct: 0.9 })
  assert.deepEqual(gaugeBands(0, 100, 150, 200), { warnPct: 1, critPct: 1 })
})
