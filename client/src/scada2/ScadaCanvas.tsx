import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { ScadaLayoutState } from './engine/widgetState'
import WidgetShell from './widgets/WidgetShell'
import type { SensorConfig, SensorData } from '../types'

/** Screens are authored on a fixed stage and scaled to the viewport, so they look the same on every monitor. */
export const STAGE_W = 1920
export const STAGE_H = 1080
export const GRID = 10

export type DropPayload =
  | { kind: 'symbol'; key: string }
  | { kind: 'widget'; type: string }
  | { kind: 'sensor'; id: number }

export const DND_SYMBOL = 'application/x-voltrus-symbol'
export const DND_WIDGET = 'application/x-voltrus-widget'
export const DND_TAG = 'application/x-voltrus-tag'

interface Props {
  layout: ScadaLayoutState
  sensors: SensorConfig[]
  currentValues: Record<string, SensorData>
  editMode: boolean
  selectedId: string | null
  zoom: 'fit' | number
  showGrid: boolean
  cursor?: string
  locked?: boolean
  onScaleChange?: (scale: number) => void
  onSelectWidget: (id: string) => void
  onMoveWidget: (id: string, x: number, y: number) => void
  onResizeWidget: (id: string, x: number, y: number, width: number, height: number) => void
  onDrop: (payload: DropPayload, x: number, y: number) => void
  onClickEmpty: () => void
  onPointerMove?: (x: number, y: number) => void
  onWriteValue?: (widgetId: string, sensorId: number, value: number) => void
  children?: React.ReactNode // connection layer overlay
}

const snap = (v: number) => Math.round(v / GRID) * GRID

export const ScadaCanvas: React.FC<Props> = ({
  layout, sensors, currentValues, editMode, selectedId, zoom, showGrid, cursor, locked, onScaleChange,
  onSelectWidget, onMoveWidget, onResizeWidget, onDrop, onClickEmpty, onPointerMove, onWriteValue, children,
}) => {
  const viewportRef = useRef<HTMLDivElement>(null)
  const stageRef = useRef<HTMLDivElement>(null)
  const [vp, setVp] = useState({ w: 0, h: 0 })

  useLayoutEffect(() => {
    const el = viewportRef.current
    if (!el) return
    const ro = new ResizeObserver(([e]) => setVp({ w: e.contentRect.width, h: e.contentRect.height }))
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  const pad = editMode ? 48 : 0
  const fit = vp.w > 0 ? Math.max(0.1, Math.min((vp.w - pad) / STAGE_W, (vp.h - pad) / STAGE_H)) : 1
  const scale = zoom === 'fit' ? fit : zoom

  useEffect(() => { onScaleChange?.(scale) }, [scale, onScaleChange])

  const toStage = useCallback((clientX: number, clientY: number) => {
    const r = stageRef.current!.getBoundingClientRect()
    return { x: (clientX - r.left) / scale, y: (clientY - r.top) / scale }
  }, [scale])

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault()
    const { x, y } = toStage(e.clientX, e.clientY)
    const sym = e.dataTransfer.getData(DND_SYMBOL)
    const wt = e.dataTransfer.getData(DND_WIDGET)
    const sid = parseInt(e.dataTransfer.getData(DND_TAG), 10)
    if (sym) onDrop({ kind: 'symbol', key: sym }, snap(x), snap(y))
    else if (wt) onDrop({ kind: 'widget', type: wt }, snap(x), snap(y))
    else if (!isNaN(sid)) onDrop({ kind: 'sensor', id: sid }, snap(x), snap(y))
  }, [onDrop, toStage])

  const sensorMap = new Map(sensors.map(s => [s.id, s]))
  const sw = STAGE_W * scale
  const sh = STAGE_H * scale

  return (
    <div
      ref={viewportRef}
      style={{
        position: 'absolute', inset: 0, overflow: zoom === 'fit' ? 'hidden' : 'auto',
        background: editMode ? 'var(--bg)' : 'var(--hmi-stage)',
      }}
    >
      <div style={{ minWidth: '100%', minHeight: '100%', width: sw + pad, height: sh + pad, display: 'grid', placeItems: 'center' }}>
        <div style={{ width: sw, height: sh, position: 'relative' }}>
          <div
            ref={stageRef}
            onDrop={handleDrop}
            onDragOver={e => { e.preventDefault(); e.dataTransfer.dropEffect = 'copy' }}
            onMouseDown={e => { if (e.target === stageRef.current) onClickEmpty() }}
            onMouseMove={onPointerMove ? e => { const p = toStage(e.clientX, e.clientY); onPointerMove(p.x, p.y) } : undefined}
            style={{
              position: 'absolute', left: 0, top: 0, width: STAGE_W, height: STAGE_H,
              transform: `scale(${scale})`, transformOrigin: '0 0',
              background: 'var(--hmi-stage)',
              backgroundImage: editMode && showGrid
                ? 'radial-gradient(circle, color-mix(in srgb, var(--hmi-stroke) 28%, transparent) 1px, transparent 1.2px)'
                : undefined,
              backgroundSize: `${GRID * 2}px ${GRID * 2}px`,
              boxShadow: editMode ? '0 0 0 1px var(--border), 0 12px 40px rgba(0,0,0,.35)' : undefined,
              cursor,
            }}
          >
            {children}
            {layout.widgets.map(w => (
              <WidgetShell
                key={w.id}
                instance={w}
                sensor={w.sensorId != null ? sensorMap.get(w.sensorId) : undefined}
                data={w.sensorId != null ? currentValues[String(w.sensorId)] : undefined}
                sensors={sensors}
                currentValues={currentValues}
                editMode={editMode}
                selected={selectedId === w.id}
                scale={scale}
                locked={locked}
                onSelect={onSelectWidget}
                onMove={onMoveWidget}
                onResize={onResizeWidget}
                onWriteValue={
                  onWriteValue && w.sensorId != null
                    ? (value: number) => onWriteValue(w.id, w.sensorId!, value)
                    : undefined
                }
              />
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}
