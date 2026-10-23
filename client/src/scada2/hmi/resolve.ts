import type { WidgetInstance } from '../widgets/types'
import { SYMBOL_MAP, type PortSide, type SymbolDef } from './symbols'

export interface ResolvedSymbol {
  def: SymbolDef
  /** Config normalised to the symbol vocabulary (legacy keys mapped). */
  config: Record<string, any>
  /** Legacy pump/valve/motor widgets toggled on click; new symbols must opt in. */
  defaultWritable: boolean
}

/** Map a widget (new `symbol` type or a legacy equipment widget) to the library symbol that draws it. */
export function resolveSymbol(w: Pick<WidgetInstance, 'type' | 'config'>): ResolvedSymbol | null {
  const c = w.config ?? {}
  switch (w.type) {
    case 'symbol':
      return { def: SYMBOL_MAP[c.symbol] ?? SYMBOL_MAP.pump, config: c, defaultWritable: false }
    case 'pump':
    case 'motor':
    case 'valve':
      // Legacy "showValue" meant the running/stopped text; these widgets never showed a value badge.
      return { def: SYMBOL_MAP[w.type], config: { ...c, showValue: false, showStatus: c.showStatus ?? c.showValue ?? true }, defaultWritable: true }
    case 'tank_level':
    case 'horizontal_tank':
      return {
        def: SYMBOL_MAP[w.type === 'tank_level' ? 'tank' : 'tank_horizontal'],
        config: { ...c, showPercent: c.showPercent ?? c.showPercentage ?? true },
        defaultWritable: false,
      }
    case 'pipe':
      return { def: SYMBOL_MAP[c.direction === 'vertical' ? 'pipe_vertical' : 'pipe'], config: c, defaultWritable: false }
    default:
      return null
  }
}

// ── Footer (label / value / status rows under the symbol) ──

export const FOOTER = { pad: 2, gap: 2, label: 15, badge: 19, status: 12 }

export function footerRows(def: SymbolDef, config: Record<string, any>, bound: boolean) {
  return {
    label: config.showLabel !== false && !!config.label,
    value: bound && (config.showValue ?? (def.binding === 'level' || def.binding === 'value')),
    status: def.binding === 'state' && !!config.showStatus,
  }
}

export function footerHeight(def: SymbolDef, config: Record<string, any>, bound: boolean): number {
  const r = footerRows(def, config, bound)
  const rows = [r.label && FOOTER.label, r.value && FOOTER.badge, r.status && FOOTER.status].filter(Boolean) as number[]
  if (rows.length === 0) return 0
  return FOOTER.pad + rows.reduce((a, b) => a + b, 0) + FOOTER.gap * (rows.length - 1)
}

// ── Ports ──

const ROTATE_CW: Record<PortSide, PortSide> = { left: 'top', top: 'right', right: 'bottom', bottom: 'left' }
const MIRROR: Record<PortSide, PortSide> = { left: 'right', right: 'left', top: 'top', bottom: 'bottom' }

/** Absolute stage point where a connection attaches to `side` of a widget, following the drawn symbol's nozzles. */
export function portPoint(w: WidgetInstance, side: PortSide): [number, number] {
  const r = resolveSymbol(w)
  if (!r) {
    const mid: Record<PortSide, [number, number]> = {
      left: [w.x, w.y + w.height / 2], right: [w.x + w.width, w.y + w.height / 2],
      top: [w.x + w.width / 2, w.y], bottom: [w.x + w.width / 2, w.y + w.height],
    }
    return mid[side]
  }
  const { def, config } = r
  const rotation = (((Number(config.rotation ?? 0) % 360) + 360) % 360) as 0 | 90 | 180 | 270
  const flip = !!config.flipX

  // Which local side ends up facing `side` after mirror-then-rotate (CSS applies scaleX before rotate).
  let local: PortSide = side
  for (let i = 0; i < rotation / 90; i++) local = (Object.keys(ROTATE_CW) as PortSide[]).find(k => ROTATE_CW[k] === local)!
  if (flip) local = MIRROR[local]

  const [vw, vh] = def.viewBox
  const availH = Math.max(1, w.height - footerHeight(def, config, w.sensorId != null))
  let sx: number, sy: number, ox = 0, oy = 0
  if (def.stretch) {
    sx = w.width / vw
    sy = availH / vh
  } else {
    sx = sy = Math.min(w.width / vw, availH / vh)
    ox = (w.width - vw * sx) / 2
    oy = (availH - vh * sy) / 2
  }
  const fallback: Record<PortSide, [number, number]> = { left: [0, vh / 2], right: [vw, vh / 2], top: [vw / 2, 0], bottom: [vw / 2, vh] }
  const [px, py] = def.ports?.[local] ?? fallback[local]
  let x = ox + px * sx
  let y = oy + py * sy

  const cx = w.width / 2
  const cy = availH / 2
  if (flip) x = 2 * cx - x
  if (rotation) {
    const a = (rotation * Math.PI) / 180
    const dx = x - cx
    const dy = y - cy
    x = cx + dx * Math.cos(a) - dy * Math.sin(a)
    y = cy + dx * Math.sin(a) + dy * Math.cos(a)
  }
  return [w.x + Math.round(x * 10) / 10, w.y + Math.round(y * 10) / 10]
}
