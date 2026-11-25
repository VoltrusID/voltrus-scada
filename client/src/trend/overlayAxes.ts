// Price scales of the tags overlaid on a tag's detail chart. One axis per
// unit: a 0-40 NTU turbidity, a 0-100 % level and a 0-4 mg/L residual each
// fill the chart height instead of sharing one axis where the small ranges
// draw as flat lines.

export type OverlaySide = 'right' | 'left' | 'own'

export interface OverlayScale {
  /** lightweight-charts price scale id. */
  id: string
  /** right: the primary tag's axis; left: the left axis; own: a scale of its own with no drawn axis. */
  side: OverlaySide
}

const trimmed = (unit: string | null | undefined) => (unit ?? '').trim()

/**
 * A unit as compared: trimmed, case-folded and NFKC-normalised, so `m³/h`,
 * `M3/h` and ` m3/h ` are one unit and share one axis.
 */
export function unitKey(unit: string | null | undefined): string {
  return trimmed(unit).normalize('NFKC').toLowerCase()
}

/**
 * The scale of each overlay, in order. An overlay in the primary tag's unit
 * shares its right axis; the first other unit gets the left axis; every
 * further unit gets a scale of its own (autoscaled to its data, no labels:
 * the overlay's legend shows its value). Overlays of the same unit share
 * their scale, so equal values line up.
 */
export function overlayScales(primaryUnit: string, overlayUnits: string[]): OverlayScale[] {
  const primary = unitKey(primaryUnit)
  let leftUnit: string | null = null
  return overlayUnits.map(raw => {
    const unit = unitKey(raw)
    if (unit === primary) return { id: 'right', side: 'right' }
    if (leftUnit === null) leftUnit = unit
    if (unit === leftUnit) return { id: 'left', side: 'left' }
    return { id: `unit:${unit}`, side: 'own' }
  })
}

/** Decimals of axis labels: the tag's (0..6), two when it has none. */
export function axisDecimals(decimals?: number | null): number {
  if (decimals == null || !Number.isFinite(decimals)) return 2
  return Math.min(Math.max(Math.round(decimals), 0), 6)
}

/** Axis label of a price: the tag's decimals (two by default) and its unit. */
export function axisLabel(price: number, unit: string, decimals?: number | null): string {
  const u = trimmed(unit)
  const n = price.toFixed(axisDecimals(decimals))
  return (/^-0(\.0+)?$/.test(n) ? n.slice(1) : n) + (u ? ` ${u}` : '')
}

/** A colour as compared: `#ABC`, ` #aabbcc ` and `#aabbcc` are one colour. */
function colorKey(color: string): string {
  const c = color.trim().toLowerCase()
  const short = /^#([0-9a-f])([0-9a-f])([0-9a-f])$/.exec(c)
  return short ? `#${short[1]}${short[1]}${short[2]}${short[2]}${short[3]}${short[3]}` : c
}

/**
 * Overlay colours that stand apart from the primary line and from each
 * other: a colour already taken (tags often keep the default) is replaced by
 * the next free trend pen colour.
 */
export function distinctColors(primary: string, colors: string[], palette: readonly string[]): string[] {
  const taken = new Set([colorKey(primary)])
  return colors.map(c => {
    const color = taken.has(colorKey(c)) ? palette.find(p => !taken.has(colorKey(p))) ?? c : c
    taken.add(colorKey(color))
    return color
  })
}

/** Where an overlay is read, for its legend chip. */
export function sideLabel(side: OverlaySide): string {
  return side === 'right' ? 'right axis' : side === 'left' ? 'left axis' : 'own scale'
}
