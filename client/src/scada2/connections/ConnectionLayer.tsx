import React from 'react'
import type { WidgetInstance, ConnectionInstance } from '../widgets/types'
import type { SensorData } from '../../types'
import { portPoint, resolveSymbol } from '../hmi/resolve'
import type { PortSide } from '../hmi/symbols'

interface Props {
  widgets: WidgetInstance[]
  connections: ConnectionInstance[]
  currentValues: Record<string, SensorData>
  editMode: boolean
  selectedId: string | null
  onSelect?: (id: string) => void
  /** Rubber-band line while the connect tool has a source picked. */
  pending?: { fromId: string; x: number; y: number } | null
}

type Pt = [number, number]

const center = (w: WidgetInstance): Pt => [w.x + w.width / 2, w.y + w.height / 2]

/** Side of `w` that faces `toward`, weighted by the box aspect. */
function facingSide(w: WidgetInstance, toward: Pt): PortSide {
  const [cx, cy] = center(w)
  const dx = toward[0] - cx
  const dy = toward[1] - cy
  if (Math.abs(dx) * w.height >= Math.abs(dy) * w.width) return dx >= 0 ? 'right' : 'left'
  return dy >= 0 ? 'bottom' : 'top'
}

const SIDES: PortSide[] = ['left', 'right', 'top', 'bottom']
const HORIZONTAL = (s: PortSide) => s === 'left' || s === 'right'

/** Sides a symbol has nozzles on, in its own (unrotated) frame mapped to the stage. Plain widgets connect anywhere. */
function nozzleSides(w: WidgetInstance): PortSide[] {
  const r = resolveSymbol(w)
  const ports = r?.def.ports
  if (!r || !ports || r.def.stretch) return SIDES
  const rot = ((((Number(r.config.rotation ?? 0)) % 360) + 360) % 360) / 90
  const cw: Record<PortSide, PortSide> = { left: 'top', top: 'right', right: 'bottom', bottom: 'left' }
  const mirror: Record<PortSide, PortSide> = { left: 'right', right: 'left', top: 'top', bottom: 'bottom' }
  return (Object.keys(ports) as PortSide[]).map(side => {
    let s = r.config.flipX ? mirror[side] : side
    for (let i = 0; i < rot; i++) s = cw[s]
    return s
  })
}

/** Nozzle on `w` to use toward `other`: the preferred axis first, and only sides that face the other end. */
function pickSide(w: WidgetInstance, other: WidgetInstance, horizontalFirst: boolean): PortSide {
  const [cx, cy] = center(w)
  const [ox, oy] = center(other)
  const h: PortSide = ox >= cx ? 'right' : 'left'
  const v: PortSide = oy >= cy ? 'bottom' : 'top'
  const avail = nozzleSides(w)
  const order = horizontalFirst ? [h, v] : [v, h]
  return order.find(s => avail.includes(s)) ?? order[0]
}

/** Orthogonal route between equipment nozzles: straight, L or Z shaped, using only nozzles the symbols have. */
export function routeConnection(a: WidgetInstance, b: WidgetInstance, waypoints: Pt[]): Pt[] {
  if (waypoints.length > 0) {
    const first = waypoints[0]
    const last = waypoints[waypoints.length - 1]
    return [portPoint(a, facingSide(a, first)), ...waypoints, portPoint(b, facingSide(b, last))]
  }
  const [ax, ay] = center(a)
  const [bx, by] = center(b)
  const horizontal = Math.abs(bx - ax) >= Math.abs(by - ay)
  const sa = pickSide(a, b, horizontal)
  const sb = pickSide(b, a, horizontal)
  let pa = portPoint(a, sa)
  let pb = portPoint(b, sb)
  // Busbars and straight pipes are tapped where the other end lines up, not at their midpoint.
  if (isRun(a) && !isRun(b)) pa = tapPoint(a, pb)
  else if (isRun(b) && !isRun(a)) pb = tapPoint(b, pa)
  if (isRun(a) !== isRun(b)) {
    if (Math.abs(pa[0] - pb[0]) < 0.5 || Math.abs(pa[1] - pb[1]) < 0.5) return [pa, pb]
  }
  if (HORIZONTAL(sa) && HORIZONTAL(sb)) {
    if (Math.abs(pa[1] - pb[1]) < 0.5) return [pa, pb]
    const mx = Math.round((pa[0] + pb[0]) / 2)
    return simplify([pa, [mx, pa[1]], [mx, pb[1]], pb])
  }
  if (!HORIZONTAL(sa) && !HORIZONTAL(sb)) {
    if (Math.abs(pa[0] - pb[0]) < 0.5) return [pa, pb]
    const my = Math.round((pa[1] + pb[1]) / 2)
    return simplify([pa, [pa[0], my], [pb[0], my], pb])
  }
  // One horizontal nozzle, one vertical: a single elbow.
  return simplify(HORIZONTAL(sa) ? [pa, [pb[0], pa[1]], pb] : [pa, [pa[0], pb[1]], pb])
}

/** A stretched symbol (busbar, straight pipe) that can be tapped anywhere along its length. */
function isRun(w: WidgetInstance): boolean {
  return !!resolveSymbol(w)?.def.stretch
}

/** Point on the run's long edge nearest `toward`. */
function tapPoint(w: WidgetInstance, toward: Pt): Pt {
  const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v))
  if (w.width >= w.height) {
    const x = clamp(toward[0], w.x, w.x + w.width)
    return [x, toward[1] >= w.y + w.height / 2 ? w.y + w.height : w.y]
  }
  const y = clamp(toward[1], w.y, w.y + w.height)
  return [toward[0] >= w.x + w.width / 2 ? w.x + w.width : w.x, y]
}

function simplify(pts: Pt[]): Pt[] {
  return pts.filter((p, i) => {
    if (i === 0 || i === pts.length - 1) return true
    const [a, c] = [pts[i - 1], pts[i + 1]]
    return !((a[0] === p[0] && p[0] === c[0]) || (a[1] === p[1] && p[1] === c[1]))
  })
}

/** Polyline with rounded corners. */
function roundedPath(pts: Pt[], r = 10): string {
  if (pts.length < 2) return ''
  let d = `M${pts[0][0]} ${pts[0][1]}`
  for (let i = 1; i < pts.length - 1; i++) {
    const [px, py] = pts[i - 1]
    const [x, y] = pts[i]
    const [nx, ny] = pts[i + 1]
    const l1 = Math.hypot(x - px, y - py)
    const l2 = Math.hypot(nx - x, ny - y)
    const rr = Math.min(r, l1 / 2, l2 / 2)
    const ax = x - ((x - px) / l1) * rr
    const ay = y - ((y - py) / l1) * rr
    const bx = x + ((nx - x) / l2) * rr
    const by = y + ((ny - y) / l2) * rr
    d += ` L${ax} ${ay} Q${x} ${y} ${bx} ${by}`
  }
  const last = pts[pts.length - 1]
  return d + ` L${last[0]} ${last[1]}`
}

/** Only a run signal counts: an explicit run tag, or the primary tag of a state-bound symbol (pump, valve…). */
export function isRunning(w: WidgetInstance | undefined, values: Record<string, SensorData>): boolean {
  if (!w) return false
  const r = resolveSymbol(w)
  const cfg = r?.config ?? w.config ?? {}
  if (cfg.runSensorId != null) return (values[String(cfg.runSensorId)]?.value ?? 0) > 0.5
  if (!r || r.def.binding !== 'state' || w.sensorId == null) return false
  const v = values[String(w.sensorId)]?.value
  return v != null && v > Number(cfg.onThreshold ?? 0.5)
}

export const CONNECTION_TYPES: { value: ConnectionInstance['connectionType']; label: string }[] = [
  { value: 'flow', label: 'Process pipe' },
  { value: 'power', label: 'Power line' },
  { value: 'data', label: 'Signal / data' },
]

export const ConnectionLayer: React.FC<Props> = ({ widgets, connections, currentValues, editMode, selectedId, onSelect, pending }) => {
  const byId = new Map(widgets.map(w => [w.id, w]))

  return (
    <svg style={{ position: 'absolute', inset: 0, width: '100%', height: '100%', pointerEvents: 'none', zIndex: 1, overflow: 'visible' }}>
      {connections.map(conn => {
        const from = byId.get(conn.fromWidgetId)
        const to = byId.get(conn.toWidgetId)
        if (!from || !to) return null
        const d = roundedPath(routeConnection(from, to, conn.waypoints))
        const selected = editMode && selectedId === conn.id
        // A pipe flows when the equipment at either end is running (tank → running pump counts).
        const flowing = conn.connectionType === 'flow' && (isRunning(from, currentValues) || isRunning(to, currentValues))

        return (
          <g key={conn.id}>
            {selected && <path d={d} fill="none" stroke="var(--cyan)" strokeWidth={conn.connectionType === 'flow' ? 18 : 9} opacity={0.35} strokeLinejoin="round" />}
            {conn.connectionType === 'flow' ? (
              <>
                <path d={d} fill="none" stroke="var(--hmi-pipe-lo)" strokeWidth={11} strokeLinejoin="round" />
                <path d={d} fill="none" stroke="var(--hmi-pipe-hi)" strokeWidth={4.5} strokeLinejoin="round" opacity={0.7} />
                {flowing && (
                  <path d={d} fill="none" stroke="var(--hmi-liquid-hi)" strokeWidth={3} strokeDasharray="8 16"
                    strokeLinecap="round" className="hmi-flow" />
                )}
              </>
            ) : conn.connectionType === 'power' ? (
              <path d={d} fill="none" stroke="#c9a23a" strokeWidth={3} strokeLinejoin="round" />
            ) : (
              <path d={d} fill="none" stroke="#5b8fd1" strokeWidth={1.75} strokeDasharray="6 4" strokeLinejoin="round" />
            )}
            {editMode && onSelect && (
              <path d={d} fill="none" stroke="transparent" strokeWidth={16} style={{ pointerEvents: 'stroke', cursor: 'pointer' }}
                onMouseDown={e => e.stopPropagation()}
                onClick={e => { e.stopPropagation(); onSelect(conn.id) }} />
            )}
          </g>
        )
      })}
      {pending && byId.get(pending.fromId) && (() => {
        const src = byId.get(pending.fromId)!
        const [ax, ay] = portPoint(src, facingSide(src, [pending.x, pending.y]))
        return <line x1={ax} y1={ay} x2={pending.x} y2={pending.y} stroke="var(--cyan)" strokeWidth={2} strokeDasharray="5 4" />
      })()}
    </svg>
  )
}
