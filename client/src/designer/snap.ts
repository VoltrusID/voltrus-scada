// Snapping while dragging: to user guides, to the edges and centres of other
// widgets and of the stage, then to the grid. Object and guide snaps win over
// the grid when they are within the threshold, and draw a guide line.

import type { Box } from './model.ts'

export interface Guide { axis: 'x' | 'y'; pos: number }

export interface SnapOptions {
  grid: number | null
  /** Pixels (in stage units) within which an edge snaps to a target. */
  threshold: number
  objects: boolean
  guides: Guide[]
  stage: { w: number; h: number }
}

export interface SnapResult { x: number; y: number; lines: Guide[] }

/** Candidate lines of a box: start, centre, end. */
const lines = (start: number, size: number) => [start, start + size / 2, start + size]

function best(moving: number[], targets: number[], threshold: number): { delta: number; at: number } | null {
  let out: { delta: number; at: number } | null = null
  for (const m of moving) {
    for (const t of targets) {
      const d = t - m
      if (Math.abs(d) <= threshold && (!out || Math.abs(d) < Math.abs(out.delta))) out = { delta: d, at: t }
    }
  }
  return out
}

/** Where a box being moved to (box.x, box.y) lands, and which guide lines to show. */
export function snapBox(box: Box, others: Box[], o: SnapOptions): SnapResult {
  const xs: number[] = [], ys: number[] = []
  for (const g of o.guides) (g.axis === 'x' ? xs : ys).push(g.pos)
  if (o.objects) {
    xs.push(0, o.stage.w / 2, o.stage.w)
    ys.push(0, o.stage.h / 2, o.stage.h)
    for (const b of others) { xs.push(...lines(b.x, b.width)); ys.push(...lines(b.y, b.height)) }
  }
  const out: Guide[] = []
  let x = box.x, y = box.y
  const sx = best(lines(box.x, box.width), xs, o.threshold)
  if (sx) { x += sx.delta; out.push({ axis: 'x', pos: sx.at }) } else if (o.grid) x = Math.round(x / o.grid) * o.grid
  const sy = best(lines(box.y, box.height), ys, o.threshold)
  if (sy) { y += sy.delta; out.push({ axis: 'y', pos: sy.at }) } else if (o.grid) y = Math.round(y / o.grid) * o.grid
  return { x, y, lines: out }
}

/** Snap one value (a resize edge) to guides, object lines or the grid. */
export function snapValue(v: number, axis: 'x' | 'y', others: Box[], o: SnapOptions): { v: number; line: Guide | null } {
  const targets = o.guides.filter(g => g.axis === axis).map(g => g.pos)
  if (o.objects) for (const b of others) targets.push(...(axis === 'x' ? lines(b.x, b.width) : lines(b.y, b.height)))
  const s = best([v], targets, o.threshold)
  if (s) return { v: s.at, line: { axis, pos: s.at } }
  return { v: o.grid ? Math.round(v / o.grid) * o.grid : v, line: null }
}

/** Ruler tick spacing in stage units for a zoom: about every 50-100 screen pixels. */
export function rulerStep(scale: number): { major: number; minor: number } {
  const steps = [10, 20, 50, 100, 200, 500, 1000]
  const major = steps.find(s => s * scale >= 60) ?? 1000
  return { major, minor: major / (major === 20 || major === 200 ? 4 : 5) }
}
