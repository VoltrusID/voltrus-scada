// The trend popup's pens and their scales. Every pen is scaled on its own axis
// (autoscaled to its data, with margins), so a 0-5 bar pressure and a 0-2000
// m³/h flow are both readable on one chart.

export interface Point { ts: number; value: number }

export interface PenAxis {
  lo: number
  hi: number
  /** Three tick values from bottom to top. */
  ticks: [number, number, number]
}

/** Colours of the pens in order; theme-independent chart colours that read on both backgrounds. */
export const PEN_COLORS = ['#0ea5e9', '#f97316', '#a855f7', '#16a34a', '#e11d48', '#ca8a04']

export const MAX_PENS = PEN_COLORS.length

export function penColor(i: number): string {
  return PEN_COLORS[i % PEN_COLORS.length]
}

/**
 * The axis of one pen: its data range plus 8 % margin. A flat line gets ±1 (or
 * ±1 % of its value) so it sits in the middle; a digital pen (only 0 and 1)
 * is fixed to -0.1..1.1.
 */
export function penAxis(points: Point[]): PenAxis | null {
  const vals = points.map(p => p.value).filter(Number.isFinite)
  if (vals.length === 0) return null
  let lo = Math.min(...vals)
  let hi = Math.max(...vals)
  if (vals.every(v => v === 0 || v === 1)) {
    lo = -0.1; hi = 1.1
  } else if (hi - lo < 1e-9) {
    const pad = Math.max(1, Math.abs(lo) * 0.01)
    lo -= pad; hi += pad
  } else {
    const pad = (hi - lo) * 0.08
    lo -= pad; hi += pad
  }
  return { lo, hi, ticks: [lo, (lo + hi) / 2, hi] }
}

/** Y of a value on a pen's axis, in a box of height h (0 at the top). */
export function yOf(axis: PenAxis, v: number, h: number): number {
  return h - ((v - axis.lo) / (axis.hi - axis.lo)) * h
}

/** Shared time range of all pens, in ms. */
export function timeRange(pens: Point[][]): [number, number] | null {
  let t0 = Infinity
  let t1 = -Infinity
  for (const pts of pens) {
    for (const p of pts) {
      if (p.ts < t0) t0 = p.ts
      if (p.ts > t1) t1 = p.ts
    }
  }
  return Number.isFinite(t0) && t1 > t0 ? [t0, t1] : null
}

/** Pen list after adding a tag (no duplicates, at most MAX_PENS, the newest kept). */
export function addPen(pens: number[], id: number): number[] {
  if (pens.includes(id)) return pens
  return [...pens, id].slice(-MAX_PENS)
}

/** A tick label with as many decimals as the axis span needs. */
export function tickLabel(v: number, axis: PenAxis): string {
  const span = axis.hi - axis.lo
  const d = span >= 100 ? 0 : span >= 10 ? 1 : span >= 1 ? 2 : 3
  return v.toFixed(d)
}

/** History points (`ts` in ms or s) normalised to ms, ascending. */
export function normalisePoints(raw: { ts: number; value: number }[]): Point[] {
  return raw
    .map(p => ({ ts: p.ts > 1e11 ? p.ts : p.ts * 1000, value: p.value }))
    .filter(p => Number.isFinite(p.value))
    .sort((a, b) => a.ts - b.ts)
}
