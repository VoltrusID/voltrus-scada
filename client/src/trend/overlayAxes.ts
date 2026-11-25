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

const norm = (unit: string | null | undefined) => (unit ?? '').trim()

/**
 * The scale of each overlay, in order. An overlay in the primary tag's unit
 * shares its right axis; the first other unit gets the left axis; every
 * further unit gets a scale of its own (autoscaled to its data, no labels:
 * the overlay's legend shows its value). Overlays of the same unit share
 * their scale, so equal values line up.
 */
export function overlayScales(primaryUnit: string, overlayUnits: string[]): OverlayScale[] {
  const primary = norm(primaryUnit)
  let leftUnit: string | null = null
  return overlayUnits.map(raw => {
    const unit = norm(raw)
    if (unit === primary) return { id: 'right', side: 'right' }
    if (leftUnit === null) leftUnit = unit
    if (unit === leftUnit) return { id: 'left', side: 'left' }
    return { id: `unit:${unit}`, side: 'own' }
  })
}

/** Axis label of a price: two decimals and the unit. */
export function axisLabel(price: number, unit: string): string {
  const u = norm(unit)
  return price.toFixed(2) + (u ? ` ${u}` : '')
}

/**
 * Overlay colours that stand apart from the primary line and from each
 * other: a colour already taken (tags often keep the default) is replaced by
 * the next free trend pen colour.
 */
export function distinctColors(primary: string, colors: string[], palette: readonly string[]): string[] {
  const taken = new Set([primary.toLowerCase()])
  return colors.map(c => {
    const color = taken.has(c.toLowerCase()) ? palette.find(p => !taken.has(p.toLowerCase())) ?? c : c
    taken.add(color.toLowerCase())
    return color
  })
}

/** Where an overlay is read, for its legend chip. */
export function sideLabel(side: OverlaySide): string {
  return side === 'right' ? 'right axis' : side === 'left' ? 'left axis' : 'own scale'
}
