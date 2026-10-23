import type { WidgetInstance } from '../scada2/widgets/types'
import type { SymbolDef } from '../scada2/hmi/symbols'
import { resolveSymbol, footerHeight } from '../scada2/hmi/resolve'
import { STAGE_W, STAGE_H } from '../scada2/ScadaCanvas'

/** Metres per stage pixel: the 1920 × 1080 screen becomes a 48 × 27 m plant. */
export const SCALE = 0.025

export const toWorld = (px: number, py: number): [number, number] => [(px - STAGE_W / 2) * SCALE, (py - STAGE_H / 2) * SCALE]

export interface PlacedItem {
  id: string
  widget: WidgetInstance
  def: SymbolDef
  config: Record<string, any>
  key: string
  /** Footprint centre and size in metres. */
  x: number
  z: number
  w: number
  d: number
  h: number
  /** Half-extents where pipes attach. */
  hx: number
  hz: number
  /** Top of the model, for labels and cable drops. */
  top: number
  rotY: number
}

export interface PlacedArea {
  id: string
  title: string
  subtitle: string
  x: number
  z: number
  w: number
  d: number
}

// Symbols drawn in plan in 2D (their 2D height is a depth); everything else is drawn in elevation (2D height is a height).
const PLAN = new Set(['clarifier', 'gravity_filter', 'conveyor', 'busbar', 'pipe', 'pipe_vertical', 'pipe_elbow', 'pipe_tee', 'heat_exchanger', 'tank_horizontal', 'solar_panel', 'battery'])
const INLINE = new Set(['valve', 'control_valve', 'check_valve', 'flow_meter'])

function extents(key: string, w: number, d: number, h: number): { hx: number; hz: number; top: number } {
  const s = Math.min(w, d)
  switch (key) {
    case 'pump': return { hx: s * 0.55, hz: s * 0.26, top: 0.35 + s * 0.42 }
    case 'dosing_pump': return { hx: s * 0.45, hz: s * 0.36, top: 0.2 + s * 0.85 }
    case 'fan': return { hx: s * 0.52, hz: s * 0.3, top: 0.2 + s * 0.75 }
    case 'motor': return { hx: s * 0.4, hz: s * 0.2, top: 0.6 }
    case 'compressor': return { hx: s * 0.5, hz: s * 0.35, top: 0.2 + s * 0.65 }
    case 'tank': case 'mixer': case 'silo': {
      const r = s * 0.46
      return { hx: r, hz: r, top: h + r * 0.3 }
    }
    case 'tank_horizontal': return { hx: w * 0.45, hz: Math.min(d * 0.45, w * 0.22), top: 0.4 + Math.min(d * 0.9, w * 0.44) }
    case 'clarifier': { const R = Math.max(1.5, (w + d) / 4); return { hx: R, hz: R, top: 2.4 } }
    case 'gravity_filter': return { hx: w * 0.46, hz: Math.max(d, w * 0.6) * 0.46, top: 3.0 }
    case 'valve': case 'check_valve': return { hx: s * 0.42, hz: 0.15, top: 1.35 }
    case 'control_valve': return { hx: s * 0.42, hz: 0.15, top: 1.5 }
    case 'flow_meter': return { hx: Math.min(w, d * 1.4) * 0.4, hz: 0.2, top: 1.2 }
    case 'instrument': return { hx: 0.15, hz: 0.15, top: 1.6 }
    case 'meter': return { hx: 0.3, hz: 0.15, top: 2.0 }
    case 'transformer': { const k = Math.max(1.6, s * 1.1); return { hx: k * 0.5, hz: k * 0.4, top: 0.4 + k * 0.95 } }
    case 'breaker': return { hx: 0.45, hz: 0.3, top: 2.25 }
    case 'busbar': return { hx: w / 2, hz: 0.25, top: 2.7 }
    case 'generator': return { hx: s * 0.6, hz: s * 0.35, top: 0.3 + s * 0.9 }
    case 'inverter': return { hx: 0.5, hz: 0.35, top: 2.0 }
    case 'battery': return { hx: Math.max(2.4, w * 0.9) / 2, hz: Math.max(1.2, d * 0.8) / 2, top: 2.2 }
    case 'solar_panel': return { hx: w * 0.45, hz: d * 0.4, top: 1.9 }
    case 'heat_exchanger': return { hx: w * 0.45, hz: Math.min(d * 0.35, 0.45), top: 1.7 }
    case 'filter': return { hx: s * 0.3, hz: s * 0.3, top: 0.7 + s * 0.9 }
    case 'pipe': case 'pipe_vertical': case 'pipe_elbow': case 'pipe_tee': return { hx: w / 2, hz: d / 2, top: 0.9 }
    case 'conveyor': return { hx: w * 0.48, hz: d * 0.3, top: 1.1 }
    default: return { hx: w * 0.35, hz: d * 0.35, top: 1.0 }
  }
}

/** Equipment and areas of a SCADA screen, positioned on the plant floor. */
export function placeLayout(widgets: WidgetInstance[]): { items: PlacedItem[]; areas: PlacedArea[] } {
  const items: PlacedItem[] = []
  for (const wdg of widgets) {
    const r = resolveSymbol(wdg)
    if (!r) continue
    const { def, config } = r
    const key = def.key
    const bodyH = Math.max(10, wdg.height - footerHeight(def, config, wdg.sensorId != null))
    const [x, z] = toWorld(wdg.x + wdg.width / 2, wdg.y + bodyH / 2)
    const w = wdg.width * SCALE
    const h = bodyH * SCALE
    const d = PLAN.has(key) ? h : INLINE.has(key) ? w * 0.5 : w
    // Rotation only means something for plan-drawn symbols; elevation symbols keep facing the viewer.
    const rot = PLAN.has(key) || INLINE.has(key) ? Number(config.rotation ?? 0) : 0
    const rotY = (-rot * Math.PI) / 180 + (config.flipX ? Math.PI : 0)
    const ext = extents(key, w, d, h)
    // A 90° turn swaps which way the pipes attach.
    const swap = Math.abs(Math.round(rot / 90)) % 2 === 1
    items.push({
      id: wdg.id, widget: wdg, def, config, key, x, z, w: swap ? d : w, d: swap ? w : d, h,
      hx: swap ? ext.hz : ext.hx, hz: swap ? ext.hx : ext.hz, top: ext.top, rotY,
    })
  }

  const areas: PlacedArea[] = widgets
    .filter(w => w.type === 'area')
    .map(w => {
      const [x, z] = toWorld(w.x + w.width / 2, w.y + w.height / 2)
      return { id: w.id, title: w.config?.title ?? 'Area', subtitle: w.config?.subtitle ?? '', x, z, w: w.width * SCALE, d: w.height * SCALE }
    })
    // Areas that only frame trends or gauges have no plant floor behind them.
    .filter(a => items.some(it => Math.abs(it.x - a.x) <= a.w / 2 && Math.abs(it.z - a.z) <= a.d / 2))

  return { items, areas }
}

export function bounds(items: PlacedItem[]): { cx: number; cz: number; sx: number; sz: number } {
  if (!items.length) return { cx: 0, cz: 0, sx: 20, sz: 20 }
  let x0 = Infinity, x1 = -Infinity, z0 = Infinity, z1 = -Infinity
  for (const it of items) {
    x0 = Math.min(x0, it.x - it.hx); x1 = Math.max(x1, it.x + it.hx)
    z0 = Math.min(z0, it.z - it.hz); z1 = Math.max(z1, it.z + it.hz)
  }
  return { cx: (x0 + x1) / 2, cz: (z0 + z1) / 2, sx: x1 - x0, sz: z1 - z0 }
}
