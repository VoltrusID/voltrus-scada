import React, { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { SensorConfig } from '../types'
import type { WidgetInstance } from '../scada2/widgets/types'
import { getWidgetComponent } from '../scada2/widgets/registry'
import { ScreenWidgetView, WidgetErrorBoundary, WidgetPlaceholder } from '../components/ScreenWidgetView'
import { ConnectionLayer } from '../scada2/connections/ConnectionLayer'
import { useBoundInstance } from '../scada2/widgets/WidgetShell'
import { useTrackedCurrentValues } from '../store/dashboardStore'
import { DND_SYMBOL, DND_TAG, DND_WIDGET, STAGE_H, STAGE_W } from '../scada2/ScadaCanvas'
import {
  boundsOf, expandToGroups, groupOf, isLocked, marqueeSelect, movePatches, type Box, type Doc, type Edit,
} from './model'
import { rulerStep, snapBox, snapValue, type Guide } from './snap'
import { D } from './ui'

export const RULER = 20
const PAD = 160
const HANDLE = 7

export type CanvasDrop =
  | { kind: 'tag'; id: number }
  | { kind: 'symbol'; key: string }
  | { kind: 'widget'; type: string }
  | { kind: 'equipment'; key: string }

export const DND_EQUIPMENT = 'application/x-voltrus-equipment'

interface Props {
  doc: Doc
  sensors: SensorConfig[]
  selection: string[]
  scale: number
  zoom: 'fit' | number
  onScaleChange: (s: number) => void
  onZoomTo: (s: number) => void
  showGrid: boolean
  snap: boolean
  showRulers: boolean
  guides: Guide[]
  onGuidesChange: (g: Guide[]) => void
  tool: 'select' | 'connect'
  connectFrom: string | null
  onConnectPick: (id: string) => void
  selectedConnection: string | null
  onSelectConnection: (id: string) => void
  onSelect: (ids: string[]) => void
  onEdit: (e: Edit, coalesce?: string) => void
  onDrop: (d: CanvasDrop, x: number, y: number, target: string | null, client: { x: number; y: number }) => void
  onCursor: (p: { x: number; y: number } | null) => void
}

type Handle = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w'
const HANDLES: Handle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w']

type Drag =
  | { kind: 'move'; ids: string[]; sx: number; sy: number; dx: number; dy: number; lines: Guide[]; started: boolean }
  | { kind: 'resize'; id: string; handle: Handle; sx: number; sy: number; orig: Box; box: Box; lines: Guide[] }
  | { kind: 'marquee'; sx: number; sy: number; x: number; y: number; additive: boolean; base: string[]; click?: string }
  | { kind: 'guide'; index: number | null; axis: 'x' | 'y'; pos: number }
  | { kind: 'pan'; cx: number; cy: number; left: number; top: number }

const STAGE = { w: STAGE_W, h: STAGE_H }

export function DesignerCanvas(p: Props) {
  const { doc, selection, scale, zoom } = p
  const viewportRef = useRef<HTMLDivElement>(null)
  const stageRef = useRef<HTMLDivElement>(null)
  const [vp, setVp] = useState({ w: 0, h: 0 })
  const [scroll, setScroll] = useState({ left: 0, top: 0 })
  const [drag, setDrag] = useState<Drag | null>(null)
  const dragRef = useRef<Drag | null>(null)
  dragRef.current = drag
  const [hover, setHover] = useState<string | null>(null)
  const [space, setSpace] = useState(false)
  const zoomAnchor = useRef<{ sx: number; sy: number; cx: number; cy: number } | null>(null)
  const latest = useRef(p)
  latest.current = p

  useLayoutEffect(() => {
    const el = viewportRef.current
    if (!el) return
    const ro = new ResizeObserver(([e]) => setVp({ w: e.contentRect.width, h: e.contentRect.height }))
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  // Fit: the whole stage in view with a margin.
  const fit = vp.w > 0 ? Math.max(0.05, Math.min((vp.w - 48) / STAGE_W, (vp.h - 48) / STAGE_H)) : 1
  useEffect(() => { if (zoom === 'fit') p.onScaleChange(fit) }, [zoom, fit]) // eslint-disable-line react-hooks/exhaustive-deps

  const sw = STAGE_W * scale
  const sh = STAGE_H * scale
  // Pasteboard around the stage; centred when the stage is smaller than the viewport.
  const offX = Math.max(PAD, (vp.w - sw) / 2)
  const offY = Math.max(PAD, (vp.h - sh) / 2)
  const contentW = sw + offX * 2
  const contentH = sh + offY * 2

  // Keep the point under the pointer still while zooming with the wheel; centre on fit.
  useLayoutEffect(() => {
    const el = viewportRef.current
    if (!el) return
    const a = zoomAnchor.current
    if (a) {
      el.scrollLeft = offX + a.sx * scale - a.cx
      el.scrollTop = offY + a.sy * scale - a.cy
      zoomAnchor.current = null
    } else if (zoom === 'fit') {
      el.scrollLeft = (contentW - vp.w) / 2
      el.scrollTop = (contentH - vp.h) / 2
    }
    setScroll({ left: el.scrollLeft, top: el.scrollTop })
  }, [scale]) // eslint-disable-line react-hooks/exhaustive-deps

  const toStage = useCallback((clientX: number, clientY: number) => {
    const r = stageRef.current!.getBoundingClientRect()
    return { x: (clientX - r.left) / scale, y: (clientY - r.top) / scale }
  }, [scale])

  // Space held: drag pans.
  useEffect(() => {
    const isTyping = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName))
    const down = (e: KeyboardEvent) => { if (e.code === 'Space' && !isTyping(e.target)) { setSpace(true); e.preventDefault() } }
    const up = (e: KeyboardEvent) => { if (e.code === 'Space') setSpace(false) }
    window.addEventListener('keydown', down)
    window.addEventListener('keyup', up)
    return () => { window.removeEventListener('keydown', down); window.removeEventListener('keyup', up) }
  }, [])

  // Ctrl/Cmd + wheel zooms around the pointer (native listener: React's wheel handler is passive).
  useEffect(() => {
    const el = viewportRef.current
    if (!el) return
    const onWheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return
      e.preventDefault()
      const { scale: s, onZoomTo } = latest.current
      const next = Math.max(0.1, Math.min(4, s * (e.deltaY < 0 ? 1.15 : 1 / 1.15)))
      const r = el.getBoundingClientRect()
      const st = stageRef.current!.getBoundingClientRect()
      zoomAnchor.current = { sx: (e.clientX - st.left) / s, sy: (e.clientY - st.top) / s, cx: e.clientX - r.left, cy: e.clientY - r.top }
      onZoomTo(Math.round(next * 100) / 100)
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => el.removeEventListener('wheel', onWheel)
  }, [])

  const byId = useMemo(() => new Map(doc.widgets.map(w => [w.id, w])), [doc.widgets])
  const selSet = useMemo(() => new Set(selection), [selection])

  const snapOpts = (alt: boolean) => ({
    grid: p.snap && !alt ? 10 : null,
    threshold: 6 / scale,
    objects: p.snap && !alt,
    guides: p.snap && !alt ? p.guides : [],
    stage: STAGE,
  })

  // ── Pointer handling: one window listener pair per gesture ──

  const begin = (d: Drag, e: React.PointerEvent) => {
    e.preventDefault()
    setDrag(d)
    const move = (ev: PointerEvent) => onMove(ev)
    const up = (ev: PointerEvent) => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      onUp(ev)
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
  }

  const onMove = (ev: PointerEvent) => {
    const d = dragRef.current
    if (!d) return
    const { doc } = latest.current
    if (d.kind === 'pan') {
      const el = viewportRef.current!
      el.scrollLeft = d.left - (ev.clientX - d.cx)
      el.scrollTop = d.top - (ev.clientY - d.cy)
      return
    }
    const pt = toStage(ev.clientX, ev.clientY)
    if (d.kind === 'guide') { setDrag({ ...d, pos: Math.round(d.axis === 'x' ? pt.x : pt.y) }); return }
    if (d.kind === 'marquee') { setDrag({ ...d, x: pt.x, y: pt.y }); return }
    if (d.kind === 'move') {
      const rawDx = pt.x - d.sx, rawDy = pt.y - d.sy
      if (!d.started && Math.hypot(rawDx * scale, rawDy * scale) < 3) return
      const moving = d.ids.map(id => byIdNow(doc, id)).filter(Boolean) as WidgetInstance[]
      const b = boundsOf(moving)
      if (!b) return
      const others = doc.widgets.filter(w => !d.ids.includes(w.id))
      const s = snapBox({ ...b, x: b.x + rawDx, y: b.y + rawDy }, others, snapOpts(ev.altKey))
      setDrag({ ...d, started: true, dx: Math.round(s.x - b.x), dy: Math.round(s.y - b.y), lines: s.lines })
      return
    }
    if (d.kind === 'resize') {
      const dx = pt.x - d.sx, dy = pt.y - d.sy
      const o = d.orig
      let l = o.x, t = o.y, r = o.x + o.width, btm = o.y + o.height
      const others = doc.widgets.filter(w => w.id !== d.id)
      const opts = snapOpts(ev.altKey)
      const lines: Guide[] = []
      const edge = (v: number, axis: 'x' | 'y') => { const s = snapValue(v, axis, others, opts); if (s.line) lines.push(s.line); return s.v }
      if (d.handle.includes('w')) l = Math.min(edge(o.x + dx, 'x'), r - 10)
      if (d.handle.includes('e')) r = Math.max(edge(o.x + o.width + dx, 'x'), l + 10)
      if (d.handle.includes('n')) t = Math.min(edge(o.y + dy, 'y'), btm - 10)
      if (d.handle.includes('s')) btm = Math.max(edge(o.y + o.height + dy, 'y'), t + 10)
      // Shift on a corner keeps the aspect ratio.
      if (ev.shiftKey && d.handle.length === 2) {
        const ratio = o.width / o.height
        const w = r - l, h = btm - t
        if (w / h > ratio) { const nh = w / ratio; if (d.handle.includes('n')) t = btm - nh; else btm = t + nh } else { const nw = h * ratio; if (d.handle.includes('w')) l = r - nw; else r = l + nw }
      }
      l = Math.max(0, l); t = Math.max(0, t); r = Math.min(STAGE_W, r); btm = Math.min(STAGE_H, btm)
      setDrag({ ...d, box: { x: Math.round(l), y: Math.round(t), width: Math.round(r - l), height: Math.round(btm - t) }, lines })
    }
  }

  const onUp = (ev: PointerEvent) => {
    const d = dragRef.current
    setDrag(null)
    if (!d) return
    const { doc, onEdit, onSelect, guides, onGuidesChange } = latest.current
    if (d.kind === 'move' && d.started && (d.dx || d.dy)) {
      onEdit({ type: 'patch', patches: movePatches(doc, d.ids, d.dx, d.dy, STAGE) })
    } else if (d.kind === 'resize') {
      const b = d.box, o = d.orig
      if (b.x !== o.x || b.y !== o.y || b.width !== o.width || b.height !== o.height) onEdit({ type: 'patch', patches: { [d.id]: { ...b } } })
    } else if (d.kind === 'marquee') {
      const box = { x: Math.min(d.sx, d.x), y: Math.min(d.sy, d.y), width: Math.abs(d.x - d.sx), height: Math.abs(d.y - d.sy) }
      const tiny = box.width * scale < 3 && box.height * scale < 3
      const hit = tiny ? (d.click ? expandToGroups(doc, [d.click]) : []) : marqueeSelect(doc, box).filter(id => id !== d.click)
      onSelect(d.additive ? [...new Set([...d.base, ...hit])] : hit)
    } else if (d.kind === 'guide') {
      const max = d.axis === 'x' ? STAGE_W : STAGE_H
      const keep = d.pos >= 0 && d.pos <= max
      const next = guides.filter((_, i) => i !== d.index)
      if (keep) next.push({ axis: d.axis, pos: d.pos })
      onGuidesChange(next)
    }
    void ev
  }

  const onStageDown = (e: React.PointerEvent) => {
    // Gestures call preventDefault, which keeps focus where it was: leave a property or filter field so shortcuts reach the canvas.
    if (document.activeElement instanceof HTMLElement && document.activeElement !== document.body) document.activeElement.blur()
    if (e.button === 1 || (e.button === 0 && space)) {
      const el = viewportRef.current!
      begin({ kind: 'pan', cx: e.clientX, cy: e.clientY, left: el.scrollLeft, top: el.scrollTop }, e)
      return
    }
    if (e.button !== 0) return
    const target = (e.target as HTMLElement).closest('[data-wid]') as HTMLElement | null
    const pt = toStage(e.clientX, e.clientY)
    const additive = e.shiftKey || e.ctrlKey || e.metaKey
    if (!target) {
      begin({ kind: 'marquee', sx: pt.x, sy: pt.y, x: pt.x, y: pt.y, additive, base: selection }, e)
      return
    }
    const id = target.dataset.wid!
    if (p.tool === 'connect') { e.preventDefault(); p.onConnectPick(id); return }
    // A process area is a backdrop: dragging on it draws a marquee (a click still selects it). Select it first to move it.
    const tw = byId.get(id)
    if (tw?.type === 'area' && !selSet.has(id)) {
      begin({ kind: 'marquee', sx: pt.x, sy: pt.y, x: pt.x, y: pt.y, additive, base: selection, click: id }, e)
      return
    }
    let next: string[]
    if (additive) {
      const g = expandToGroups(doc, [id])
      next = g.every(x => selSet.has(x)) ? selection.filter(x => !g.includes(x)) : [...selection, ...g.filter(x => !selSet.has(x))]
    } else next = selSet.has(id) ? selection : expandToGroups(doc, [id])
    if (next !== selection) p.onSelect(next)
    if (additive) return
    const ids = next.filter(x => { const w = byId.get(x); return w && !isLocked(w) })
    if (ids.length && byId.get(id) && !isLocked(byId.get(id)!)) begin({ kind: 'move', ids, sx: pt.x, sy: pt.y, dx: 0, dy: 0, lines: [], started: false }, e)
    else e.preventDefault()
  }

  const onHandleDown = (e: React.PointerEvent, w: WidgetInstance, handle: Handle) => {
    e.stopPropagation()
    const pt = toStage(e.clientX, e.clientY)
    begin({ kind: 'resize', id: w.id, handle, sx: pt.x, sy: pt.y, orig: { x: w.x, y: w.y, width: w.width, height: w.height }, box: { x: w.x, y: w.y, width: w.width, height: w.height }, lines: [] }, e)
  }

  const onRulerDown = (e: React.PointerEvent, axis: 'x' | 'y') => {
    const pt = toStage(e.clientX, e.clientY)
    begin({ kind: 'guide', index: null, axis, pos: Math.round(axis === 'x' ? pt.x : pt.y) }, e)
  }

  // ── Drag and drop from the panes ──

  const onDrop = (e: React.DragEvent) => {
    e.preventDefault()
    const pt = toStage(e.clientX, e.clientY)
    const x = Math.round(pt.x / 10) * 10, y = Math.round(pt.y / 10) * 10
    const target = ((e.target as HTMLElement).closest?.('[data-wid]') as HTMLElement | null)?.dataset.wid ?? null
    const client = { x: e.clientX, y: e.clientY }
    const tag = parseInt(e.dataTransfer.getData(DND_TAG), 10)
    const sym = e.dataTransfer.getData(DND_SYMBOL)
    const wt = e.dataTransfer.getData(DND_WIDGET)
    const eq = e.dataTransfer.getData(DND_EQUIPMENT)
    if (!isNaN(tag)) p.onDrop({ kind: 'tag', id: tag }, x, y, target, client)
    else if (eq) p.onDrop({ kind: 'equipment', key: eq }, x, y, null, client)
    else if (sym) p.onDrop({ kind: 'symbol', key: sym }, x, y, null, client)
    else if (wt) p.onDrop({ kind: 'widget', type: wt }, x, y, null, client)
  }

  // ── What to draw ──

  const moving = drag?.kind === 'move' && drag.started ? drag : null
  const resizing = drag?.kind === 'resize' ? drag : null
  const shown = useMemo(() => {
    if (!moving && !resizing) return doc.widgets
    return doc.widgets.map(w => {
      if (moving && moving.ids.includes(w.id)) return { ...w, x: w.x + moving.dx, y: w.y + moving.dy }
      if (resizing && resizing.id === w.id) return { ...w, ...resizing.box }
      return w
    })
  }, [doc.widgets, moving, resizing])

  const selWidgets = shown.filter(w => selSet.has(w.id))
  const selBox = boundsOf(selWidgets)
  const single = selWidgets.length === 1 ? selWidgets[0] : null
  // A selection that is exactly one group draws the group's frame.
  const groupFrame = selWidgets.length > 1 && new Set(selWidgets.map(groupOf)).size === 1 && groupOf(selWidgets[0]) != null
  const snapLines = moving?.lines ?? resizing?.lines ?? []

  const sensorMap = useMemo(() => new Map(p.sensors.map(s => [s.id, s])), [p.sensors])
  const marquee = drag?.kind === 'marquee' ? drag : null
  const guideDrag = drag?.kind === 'guide' ? drag : null
  const sx = (v: number) => v * scale
  const cursor = drag?.kind === 'pan' || space ? 'grab' : p.tool === 'connect' ? 'crosshair' : 'default'

  return (
    <div style={{ position: 'relative', flex: 1, minWidth: 0, minHeight: 0, background: 'var(--bg)', overflow: 'hidden' }}>
      {p.showRulers && (
        <>
          <div style={{ position: 'absolute', left: 0, top: 0, width: RULER, height: RULER, background: D.chrome, borderRight: `1px solid ${D.line}`, borderBottom: `1px solid ${D.line}`, zIndex: 3 }} />
          <Ruler axis="x" length={vp.w} offset={offX - scroll.left} scale={scale} band={selBox ? [selBox.x, selBox.x + selBox.width] : null}
            onPointerDown={e => onRulerDown(e, 'y')} />
          <Ruler axis="y" length={vp.h} offset={offY - scroll.top} scale={scale} band={selBox ? [selBox.y, selBox.y + selBox.height] : null}
            onPointerDown={e => onRulerDown(e, 'x')} />
        </>
      )}
      <div
        ref={viewportRef}
        data-testid="designer-viewport"
        onScroll={e => setScroll({ left: e.currentTarget.scrollLeft, top: e.currentTarget.scrollTop })}
        onDragOver={e => { e.preventDefault(); e.dataTransfer.dropEffect = 'copy' }}
        onDrop={onDrop}
        onPointerMove={e => { if (!stageRef.current) return; const pt = toStage(e.clientX, e.clientY); p.onCursor({ x: Math.round(pt.x), y: Math.round(pt.y) }) }}
        onPointerLeave={() => p.onCursor(null)}
        onPointerDown={e => { if (e.target === e.currentTarget || (e.target as HTMLElement).dataset.pasteboard) onStageDown(e) }}
        style={{
          position: 'absolute', left: p.showRulers ? RULER : 0, top: p.showRulers ? RULER : 0, right: 0, bottom: 0, overflow: 'auto', cursor,
          backgroundColor: 'color-mix(in srgb, var(--bg) 85%, var(--card))',
        }}
      >
        <div data-pasteboard="1" style={{ width: contentW, height: contentH, position: 'relative' }}>
          {/* Stage, in stage units, scaled */}
          <div style={{ position: 'absolute', left: offX, top: offY, width: sw, height: sh }}>
            <div
              ref={stageRef}
              data-testid="designer-stage"
              onPointerDown={onStageDown}
              onDoubleClick={e => {
                // Double click: just that member, inside its group.
                const id = ((e.target as HTMLElement).closest('[data-wid]') as HTMLElement | null)?.dataset.wid
                if (id && p.tool === 'select') p.onSelect([id])
              }}
              onPointerLeave={() => setHover(null)}
              style={{
                position: 'absolute', left: 0, top: 0, width: STAGE_W, height: STAGE_H, transform: `scale(${scale})`, transformOrigin: '0 0',
                background: 'var(--hmi-stage)',
                backgroundImage: p.showGrid ? 'linear-gradient(color-mix(in srgb, var(--hmi-stroke) 9%, transparent) 1px, transparent 1px), linear-gradient(90deg, color-mix(in srgb, var(--hmi-stroke) 9%, transparent) 1px, transparent 1px), linear-gradient(color-mix(in srgb, var(--hmi-stroke) 4%, transparent) 1px, transparent 1px), linear-gradient(90deg, color-mix(in srgb, var(--hmi-stroke) 4%, transparent) 1px, transparent 1px)' : undefined,
                backgroundSize: '100px 100px, 100px 100px, 10px 10px, 10px 10px',
                boxShadow: '0 0 0 1px var(--border), 0 8px 30px rgba(0,0,0,.35)',
              }}
            >
              <ConnectionLayer widgets={shown} connections={doc.connections} editMode
                selectedId={p.selectedConnection} onSelect={p.tool === 'select' ? p.onSelectConnection : undefined} />
              {shown.map(w => (
                <DesignWidget key={w.id} w={w} sensor={w.sensorId != null ? sensorMap.get(w.sensorId) : undefined} sensors={p.sensors}
                  onHover={setHover} />
              ))}
            </div>
            {/* Overlay in screen pixels: outlines, handles and guides keep their size at any zoom */}
            <svg width={sw} height={sh} style={{ position: 'absolute', left: 0, top: 0, overflow: 'visible', pointerEvents: 'none' }}>
              {p.guides.map((g, i) => (guideDrag?.index === i ? null : (
                <line key={i} {...(g.axis === 'x' ? { x1: sx(g.pos), x2: sx(g.pos), y1: -offY, y2: sh + offY } : { y1: sx(g.pos), y2: sx(g.pos), x1: -offX, x2: sw + offX })}
                  stroke="#3fb6f2" strokeWidth={1} style={{ pointerEvents: 'stroke', cursor: g.axis === 'x' ? 'ew-resize' : 'ns-resize' }}
                  data-testid="designer-guide"
                  onPointerDown={e => { e.stopPropagation(); begin({ kind: 'guide', index: i, axis: g.axis, pos: g.pos }, e as unknown as React.PointerEvent) }} />
              )))}
              {guideDrag && (
                <line {...(guideDrag.axis === 'x' ? { x1: sx(guideDrag.pos), x2: sx(guideDrag.pos), y1: -offY, y2: sh + offY } : { y1: sx(guideDrag.pos), y2: sx(guideDrag.pos), x1: -offX, x2: sw + offX })}
                  stroke="#3fb6f2" strokeDasharray="4 3" />
              )}
              {hover && !selSet.has(hover) && !drag && (() => {
                const w = byId.get(hover)
                return w ? <rect x={sx(w.x) - 0.5} y={sx(w.y) - 0.5} width={sx(w.width) + 1} height={sx(w.height) + 1} fill="none" stroke={D.accent} strokeOpacity={0.55} strokeDasharray="3 2" /> : null
              })()}
              {selWidgets.map(w => (
                <rect key={w.id} x={sx(w.x) - 0.5} y={sx(w.y) - 0.5} width={sx(w.width) + 1} height={sx(w.height) + 1} fill="none"
                  stroke={isLocked(w) ? 'var(--text-muted)' : D.accent} strokeWidth={1} strokeDasharray={isLocked(w) ? '3 2' : undefined} />
              ))}
              {selBox && selWidgets.length > 1 && (
                <rect x={sx(selBox.x) - 3.5} y={sx(selBox.y) - 3.5} width={sx(selBox.width) + 7} height={sx(selBox.height) + 7} fill="none"
                  stroke={D.accent} strokeDasharray={groupFrame ? undefined : '5 3'} strokeWidth={groupFrame ? 1.5 : 1} />
              )}
              {selWidgets.filter(isLocked).map(w => (
                <g key={`l${w.id}`} transform={`translate(${sx(w.x + w.width) - 14} ${sx(w.y) + 2})`}>
                  <rect width="12" height="12" rx="2" fill="var(--card)" stroke="var(--text-muted)" />
                  <path d="M3.5 6V4.5a2.5 2.5 0 0 1 5 0V6M3 6h6v4H3z" fill="none" stroke="var(--text-muted)" />
                </g>
              ))}
              {snapLines.map((l, i) => (
                <line key={`s${i}`} {...(l.axis === 'x' ? { x1: sx(l.pos), x2: sx(l.pos), y1: 0, y2: sh } : { y1: sx(l.pos), y2: sx(l.pos), x1: 0, x2: sw })}
                  stroke="#f0458b" strokeWidth={1} />
              ))}
              {marquee && (
                <rect x={sx(Math.min(marquee.sx, marquee.x))} y={sx(Math.min(marquee.sy, marquee.y))}
                  width={sx(Math.abs(marquee.x - marquee.sx))} height={sx(Math.abs(marquee.y - marquee.sy))}
                  fill="color-mix(in srgb, var(--cyan) 10%, transparent)" stroke={D.accent} strokeDasharray="3 2" data-testid="designer-marquee" />
              )}
              {(moving || resizing) && selBox && (
                <g transform={`translate(${sx(selBox.x)} ${sx(selBox.y) - 20})`}>
                  <rect width={resizing ? 74 : 86} height="16" rx="3" fill="var(--card)" stroke={D.line} />
                  <text x="5" y="11.5" fontSize="10" fill="var(--fg)" style={{ fontFamily: 'var(--font-mono)' }}>
                    {resizing ? `${resizing.box.width} × ${resizing.box.height}` : `${selBox.x}, ${selBox.y}`}
                  </text>
                </g>
              )}
            </svg>
            {/* Resize handles: real elements so they take the pointer */}
            {single && !isLocked(single) && p.tool === 'select' && !moving && HANDLES.map(h => {
              const cx = h.includes('w') ? 0 : h.includes('e') ? 1 : 0.5
              const cy = h.includes('n') ? 0 : h.includes('s') ? 1 : 0.5
              return (
                <div key={h} data-testid={`designer-handle-${h}`} onPointerDown={e => onHandleDown(e, single, h)} style={{
                  position: 'absolute', left: sx(single.x + single.width * cx) - HANDLE / 2, top: sx(single.y + single.height * cy) - HANDLE / 2,
                  width: HANDLE, height: HANDLE, background: 'var(--card)', border: `1.5px solid ${D.accent}`, borderRadius: 1,
                  cursor: `${h}-resize`, zIndex: 4,
                }} />
              )
            })}
          </div>
        </div>
      </div>
    </div>
  )
}

const byIdNow = (doc: Doc, id: string) => doc.widgets.find(w => w.id === id)

type DesignWidgetProps = { w: WidgetInstance; sensor: SensorConfig | undefined; sensors: SensorConfig[]; onHover: (id: string | null) => void }

/** One widget in design mode: live values and bindings shown, never interactive. A widget that fails to draw shows a placeholder. */
const DesignWidget = memo(function DesignWidget(props: DesignWidgetProps) {
  const { w, onHover } = props
  return (
    <WidgetErrorBoundary type={w.type} resetKey={w} fallback={
      <div data-wid={w.id} data-testid="designer-widget" onPointerEnter={() => onHover(w.id)} style={{ position: 'absolute', left: w.x, top: w.y, width: w.width, height: w.height }}>
        <WidgetPlaceholder text="This component failed to draw" detail={w.type} />
      </div>
    }>
      <DesignWidgetInner {...props} />
    </WidgetErrorBoundary>
  )
})

function DesignWidgetInner({ w, sensor, sensors, onHover }: DesignWidgetProps) {
  const values = useTrackedCurrentValues()
  const inst = useBoundInstance(w, sensors, values, true)
  const Comp = getWidgetComponent(inst.type)
  const data = inst.sensorId != null ? values[String(inst.sensorId)] : undefined
  // Hidden widgets stay visible to the designer, faded.
  const faded = inst.config?.visible === false
  return (
    <div data-wid={w.id} data-testid="designer-widget" onPointerEnter={() => onHover(w.id)} style={{
      position: 'absolute', left: w.x, top: w.y, width: w.width, height: w.height, display: 'flex', flexDirection: 'column',
      opacity: faded ? 0.3 : typeof inst.config?.opacity === 'number' ? Math.max(0.15, inst.config.opacity) : undefined,
      background: inst.type === 'data_table' ? 'var(--card)' : undefined,
    }}>
      <div style={{ flex: 1, minHeight: 0, pointerEvents: 'none', overflow: ['area', 'symbol', 'pump', 'motor', 'valve', 'tank_level', 'horizontal_tank', 'pipe'].includes(inst.type) ? 'visible' : 'hidden' }}>
        {Comp ? (
          <Comp instance={inst} sensor={sensor} data={data} sensors={sensors} currentValues={values} editMode selected={false} />
        ) : (
          <ScreenWidgetView type={inst.type} config={inst.config ?? {}} surface="scada" width={w.width} height={w.height} editMode sensors={sensors} values={values} />
        )}
      </div>
    </div>
  )
}

function Ruler({ axis, length, offset, scale, band, onPointerDown }: {
  axis: 'x' | 'y'; length: number; offset: number; scale: number; band: [number, number] | null
  onPointerDown: (e: React.PointerEvent) => void
}) {
  const { major, minor } = rulerStep(scale)
  const from = Math.floor(-offset / scale / minor) * minor
  const to = (length - offset) / scale
  const ticks: React.ReactNode[] = []
  for (let v = Math.max(from, -minor * 200); v <= to; v += minor) {
    const at = offset + v * scale
    const isMajor = Math.abs(v % major) < 1e-6
    ticks.push(axis === 'x'
      ? <line key={v} x1={at} x2={at} y1={isMajor ? 4 : 13} y2={RULER} stroke="var(--text-muted)" strokeWidth={1} />
      : <line key={v} y1={at} y2={at} x1={isMajor ? 4 : 13} x2={RULER} stroke="var(--text-muted)" strokeWidth={1} />)
    if (isMajor) {
      ticks.push(axis === 'x'
        ? <text key={`t${v}`} x={at + 3} y={10} fontSize={9} fill="var(--text-muted)" style={{ fontFamily: 'var(--font-mono)' }}>{v}</text>
        : <text key={`t${v}`} x={10} y={at + 3} fontSize={9} fill="var(--text-muted)" transform={`rotate(-90 10 ${at + 3})`} style={{ fontFamily: 'var(--font-mono)' }}>{v}</text>)
    }
  }
  const style: React.CSSProperties = axis === 'x'
    ? { position: 'absolute', left: RULER, top: 0, right: 0, height: RULER, cursor: 'ns-resize' }
    : { position: 'absolute', left: 0, top: RULER, bottom: 0, width: RULER, cursor: 'ew-resize' }
  return (
    <svg data-testid={`designer-ruler-${axis}`} onPointerDown={onPointerDown}
      width={axis === 'x' ? Math.max(0, length) : RULER} height={axis === 'x' ? RULER : Math.max(0, length)} style={{ ...style, background: D.chrome, zIndex: 2, borderBottom: axis === 'x' ? `1px solid ${D.line}` : undefined, borderRight: axis === 'y' ? `1px solid ${D.line}` : undefined }}>
      {band && (axis === 'x'
        ? <rect x={offset + band[0] * scale} width={(band[1] - band[0]) * scale} y={0} height={RULER} fill="color-mix(in srgb, var(--cyan) 18%, transparent)" />
        : <rect y={offset + band[0] * scale} height={(band[1] - band[0]) * scale} x={0} width={RULER} fill="color-mix(in srgb, var(--cyan) 18%, transparent)" />)}
      {ticks}
    </svg>
  )
}
