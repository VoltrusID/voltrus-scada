import React, { useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { SensorConfig } from '../types'
import { loadLayoutFromAPI } from '../scada2/engine/layout'
import { ConnectionLayer } from '../scada2/connections/ConnectionLayer'
import { STAGE_H, STAGE_W } from '../scada2/ScadaCanvas'
import type { LayoutRows } from '../queries/designHistory'
import { DesignWidget } from './DesignerCanvas'
import { OUTLINE_COLORS, type Outline } from './historyModel'

const noHover = () => {}

/**
 * A screen state drawn read-only, fitted to its box: the Designer's own widget
 * rendering (live values, bindings), no selection or editing. Used for the
 * History preview and both sides of a compare, with optional outlines.
 */
export function StageView({ rows, sensors, outlines, testId, label }: {
  rows: LayoutRows
  sensors: SensorConfig[]
  outlines?: Record<string, Outline>
  testId?: string
  label?: React.ReactNode
}) {
  const box = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ w: 0, h: 0 })
  useLayoutEffect(() => {
    const el = box.current
    if (!el) return
    const measure = () => setSize({ w: el.clientWidth, h: el.clientHeight })
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])
  const doc = useMemo(() => loadLayoutFromAPI(rows.equipment ?? [], rows.connections ?? []), [rows])
  const sensorMap = useMemo(() => new Map(sensors.map(s => [s.id, s])), [sensors])
  // A label takes a strip above the stage.
  const top = label ? 22 : 0
  const scale = size.w > 0 && size.h > 0 ? Math.max(0.05, Math.min((size.w - 24) / STAGE_W, (size.h - top - 24) / STAGE_H)) : 0.25
  const sw = STAGE_W * scale, sh = STAGE_H * scale
  const outlined = outlines ? doc.widgets.filter(w => outlines[w.id]) : []
  return (
    <div ref={box} data-testid={testId} style={{ position: 'relative', flex: 1, minWidth: 0, minHeight: 0, overflow: 'hidden', background: 'color-mix(in srgb, var(--bg) 85%, var(--card))' }}>
      {label && <div style={{ position: 'absolute', left: 8, top: 6, zIndex: 3, fontSize: 11, fontWeight: 600, color: 'var(--text-dim)' }}>{label}</div>}
      <div style={{ position: 'absolute', left: Math.max(0, (size.w - sw) / 2), top: top + Math.max(0, (size.h - top - sh) / 2), width: sw, height: sh }}>
        <div style={{
          position: 'absolute', left: 0, top: 0, width: STAGE_W, height: STAGE_H, transform: `scale(${scale})`, transformOrigin: '0 0',
          background: 'var(--hmi-stage)', boxShadow: '0 0 0 1px var(--border)', pointerEvents: 'none',
        }}>
          <ConnectionLayer widgets={doc.widgets} connections={doc.connections} editMode selectedId={null} />
          {doc.widgets.map(w => (
            <DesignWidget key={w.id} w={w} sensor={w.sensorId != null ? sensorMap.get(w.sensorId) : undefined} sensors={sensors} onHover={noHover} />
          ))}
        </div>
        <svg width={sw} height={sh} style={{ position: 'absolute', left: 0, top: 0, overflow: 'visible', pointerEvents: 'none' }}>
          {outlined.map(w => (
            <rect key={w.id} data-testid={`designer-outline-${outlines![w.id]}`} data-wid={w.id}
              x={w.x * scale - 2} y={w.y * scale - 2} width={w.width * scale + 4} height={w.height * scale + 4}
              fill={`color-mix(in srgb, ${OUTLINE_COLORS[outlines![w.id]]} 10%, transparent)`} stroke={OUTLINE_COLORS[outlines![w.id]]} strokeWidth={2} rx={2} />
          ))}
        </svg>
      </div>
    </div>
  )
}
