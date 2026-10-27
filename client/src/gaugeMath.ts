// Pure gauge geometry and band decisions, shared by the card and dial gauges.

/** Point on the dial at `angleDeg` (counter-clockwise from 3 o'clock, SVG y axis flipped). */
export function polarToCartesian(cx: number, cy: number, radius: number, angleDeg: number): { x: number; y: number } {
  const rad = (angleDeg * Math.PI) / 180
  return { x: cx + radius * Math.cos(rad), y: cy - radius * Math.sin(rad) }
}

/**
 * SVG arc from `startA` to `endA` (degrees, startA > endA runs clockwise on screen).
 * Clockwise on screen is SVG sweep-flag 1; flag 0 would draw the mirrored circle.
 */
export function dialArcPath(cx: number, cy: number, radius: number, startA: number, endA: number): string {
  const s = polarToCartesian(cx, cy, radius, startA)
  const e = polarToCartesian(cx, cy, radius, endA)
  const largeArc = Math.abs(startA - endA) > 180 ? 1 : 0
  return `M ${s.x} ${s.y} A ${radius} ${radius} 0 ${largeArc} 1 ${e.x} ${e.y}`
}

/**
 * Fractions (0..1) of the gauge span where the warn and crit bands start, or null when the
 * tag has no limits. The server stores unset limits as 0/0, which must not paint the arc red.
 */
export function gaugeBands(min: number, max: number, warn: number, crit: number): { warnPct: number; critPct: number } | null {
  if (warn === 0 && crit === 0) return null
  const range = max - min || 1
  const clamp = (v: number) => Math.max(0, Math.min(1, (v - min) / range))
  return { warnPct: clamp(warn), critPct: clamp(crit) }
}
