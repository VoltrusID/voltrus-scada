import React, { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react'
import type { SensorConfig } from '../types'
import type { ConnectionInstance, ScadaWidgetType, WidgetInstance } from '../scada2/widgets/types'
import { WIDGET_MANIFESTS } from '../scada2/widgets/registry'
import { SYMBOL_MAP } from '../scada2/hmi/symbols'
import { FACEPLATE_PRESETS, type FaceplateDef } from '../scada2/hmi/faceplateMap'
import { loadLayoutFromAPI, saveLayoutToAPI } from '../scada2/engine/layout'
import { ScadaView } from '../scada2/ScadaView'
import { widgetTitle } from '../scada2/ScadaInspector'
import { STAGE_H, STAGE_W } from '../scada2/ScadaCanvas'
import { useScreens, useSensors, useScadaEquipment, useScadaConnections, useSaveScadaEquipment, useSaveScadaConnections } from '../queries'
import { useCreateFaceplate, useFaceplates } from '../queries/scada'
import { useTrackedCurrentValues } from '../store/dashboardStore'
import { openTrend } from '../trend/TrendPopup'
import { toast } from '../components/Toast'
import { ThemeToggle } from '../components/ThemeToggle'
import {
  alignPatches, bindPrimaryPatch, boundsOf, distributePatches, duplicate, emptyHistory, expandToGroups, groupOf, groupPatches,
  historyReducer, isDirty, isLocked, lockPatches, matchSizePatches, movePatches, newId, ungroupPatches,
  type AlignMode, type Edit, type Patch,
} from './model'
import type { Guide } from './snap'
import { DesignerCanvas, type CanvasDrop } from './DesignerCanvas'
import { EQUIPMENT, PalettePane, ProjectBrowser, TagBrowserPane } from './panes'
import { PropertyEditor } from './PropertyEditor'
import { D, Icon, Pane, Sep, SmallBtn, ToolBtn, mono } from './ui'

interface Props {
  screen: string
  onOpenScreen: (id: string) => void
  onClose: () => void
  onOpenPage: (page: 'scripts' | 'alarms' | 'tags') => void
}

const ZOOM_STEPS = [0.1, 0.25, 0.33, 0.5, 0.67, 0.75, 1, 1.25, 1.5, 2, 3, 4]
const isTyping = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName))
const isDigital = (s: SensorConfig | undefined) => !!s && s.max_val - s.min_val <= 1 && s.min_val >= 0

/** What a dropped tag can become, best guess first. */
function dropChoices(s: SensorConfig | undefined): { kind: 'symbol' | 'widget'; key: string; label: string }[] {
  if (isDigital(s)) {
    return [
      { kind: 'symbol', key: 'isa_pump', label: 'Pump (ISA-101)' },
      { kind: 'symbol', key: 'isa_valve', label: 'Valve (ISA-101)' },
      { kind: 'symbol', key: 'isa_motor', label: 'Motor (ISA-101)' },
      { kind: 'widget', key: 'status_light', label: 'Status light' },
      { kind: 'widget', key: 'on_off', label: 'On/off indicator' },
      { kind: 'widget', key: 'toggle', label: 'Toggle (writes)' },
      { kind: 'widget', key: 'big_number', label: 'Number' },
    ]
  }
  const level = s && (s.unit === '%' || /level/i.test(`${s.key} ${s.name}`))
  const out: { kind: 'symbol' | 'widget'; key: string; label: string }[] = [
    { kind: 'widget', key: 'big_number', label: 'Number' },
    { kind: 'widget', key: 'indicator', label: 'Value indicator' },
    { kind: 'symbol', key: 'isa_tank', label: 'Tank (ISA-101)' },
    { kind: 'symbol', key: 'isa_control_valve', label: 'Control valve (ISA-101)' },
    { kind: 'widget', key: 'bar', label: 'Bar' },
    { kind: 'widget', key: 'gauge', label: 'Gauge' },
    { kind: 'widget', key: 'trend', label: 'Trend' },
    { kind: 'widget', key: 'slider', label: 'Slider (writes)' },
  ]
  if (level) out.unshift(out.splice(2, 1)[0])
  return out
}

const MOTOR_PRESET: FaceplateDef = { ...FACEPLATE_PRESETS[0], id: 'motor', name: 'Motor', kind: 'motor' }

function usePersisted<T>(key: string, initial: T): [T, (v: T | ((p: T) => T)) => void] {
  const [v, setV] = useState<T>(() => {
    try { const s = localStorage.getItem(key); return s ? JSON.parse(s) as T : initial } catch { return initial }
  })
  useEffect(() => { try { localStorage.setItem(key, JSON.stringify(v)) } catch { /* private mode */ } }, [key, v])
  return [v, setV]
}

export function Designer({ screen: requested, onOpenScreen, onClose, onOpenPage }: Props) {
  const { data: screens = [] } = useScreens(true)
  const { data: sensors = [] } = useSensors(true)
  const screen = requested === 'all' && screens.length ? screens[0].id : requested
  const eqQ = useScadaEquipment(screen, true)
  const connQ = useScadaConnections(screen, true)
  const saveEq = useSaveScadaEquipment()
  const saveConn = useSaveScadaConnections()
  const { data: faceplates = [] } = useFaceplates()
  const createFaceplate = useCreateFaceplate()

  const [h, dispatch] = useReducer(historyReducer, emptyHistory)
  const doc = h.present
  const dirty = isDirty(h)
  const [selection, setSelectionRaw] = useState<string[]>([])
  const [selConn, setSelConn] = useState<string | null>(null)
  const [tool, setTool] = useState<'select' | 'connect'>('select')
  const [connectFrom, setConnectFrom] = useState<string | null>(null)
  const [connectType, setConnectType] = useState<ConnectionInstance['connectionType']>('flow')
  const [zoom, setZoom] = useState<'fit' | number>('fit')
  const [scale, setScale] = useState(1)
  const [showGrid, setShowGrid] = usePersisted('designer.grid', true)
  const [snap, setSnap] = usePersisted('designer.snap', true)
  const [showRulers, setShowRulers] = usePersisted('designer.rulers', true)
  const [guidesByScreen, setGuidesByScreen] = usePersisted<Record<string, Guide[]>>('designer.guides', {})
  const guides = guidesByScreen[screen] ?? []
  const [preview, setPreview] = useState(false)
  const [cursor, setCursor] = useState<{ x: number; y: number } | null>(null)
  const [saving, setSaving] = useState(false)
  const [dropMenu, setDropMenu] = useState<{ tagId: number; x: number; y: number; cx: number; cy: number } | null>(null)
  const clipboard = useRef<{ widgets: WidgetInstance[]; connections: ConnectionInstance[]; pastes: number } | null>(null)
  const loadedScreen = useRef<string | null>(null)
  const savingRef = useRef(false)

  const setSelection = useCallback((ids: string[]) => { setSelectionRaw(ids); if (ids.length) setSelConn(null) }, [])
  const edit = useCallback((e: Edit, coalesce?: string) => dispatch({ type: 'edit', edit: e, coalesce }), [])
  const patch = useCallback((patches: Record<string, Patch>, coalesce?: string) => {
    if (Object.keys(patches).length) dispatch({ type: 'edit', edit: { type: 'patch', patches }, coalesce })
  }, [])

  // Drop selected ids that no longer exist (after undo, delete, load).
  useEffect(() => {
    const ids = new Set(doc.widgets.map(w => w.id))
    if (selection.some(id => !ids.has(id))) setSelectionRaw(s => s.filter(id => ids.has(id)))
    if (selConn && !doc.connections.some(c => c.id === selConn)) setSelConn(null)
  }, [doc, selection, selConn])

  // ── Load and save ──

  const loaded = eqQ.isFetched && connQ.isFetched && !eqQ.isFetching && !connQ.isFetching
  useEffect(() => {
    if (!loaded || savingRef.current) return
    const same = loadedScreen.current === screen
    if (same && dirty) return
    const next = loadLayoutFromAPI(eqQ.data ?? [], connQ.data ?? [])
    if (same && JSON.stringify(next) === JSON.stringify(doc)) return
    loadedScreen.current = screen
    dispatch({ type: 'load', doc: next })
    setSelectionRaw([])
    setSelConn(null)
  }, [screen, loaded, eqQ.data, connQ.data]) // eslint-disable-line react-hooks/exhaustive-deps

  const save = useCallback(async (): Promise<boolean> => {
    const id = loadedScreen.current
    if (!id || savingRef.current) return false
    savingRef.current = true
    setSaving(true)
    const snapshot = doc
    try {
      const rows = saveLayoutToAPI(snapshot, w => sensors.find(s => s.id === w.sensorId)?.name || widgetTitle(w))
      await Promise.all([
        saveEq.mutateAsync({ screenId: id, items: rows.equipment }),
        saveConn.mutateAsync({ screenId: id, items: rows.connections }),
      ])
      dispatch({ type: 'saved', doc: snapshot })
      toast.success('Screen saved')
      return true
    } catch (err: any) {
      toast.error(`Screen not saved: ${err?.message || 'request failed'}`)
      return false
    } finally {
      savingRef.current = false
      setSaving(false)
    }
  }, [doc, sensors, saveEq, saveConn])

  const openScreen = useCallback(async (id: string) => {
    if (dirty && !(await save())) return
    onOpenScreen(id)
  }, [dirty, save, onOpenScreen])

  const close = useCallback(async () => {
    if (dirty) {
      if (!window.confirm('Save changes to this screen before closing the Designer?\n\nOK saves, Cancel keeps editing.')) return
      if (!(await save())) return
    }
    onClose()
  }, [dirty, save, onClose])

  useEffect(() => {
    if (!dirty) return
    const onUnload = (e: BeforeUnloadEvent) => { e.preventDefault() }
    window.addEventListener('beforeunload', onUnload)
    return () => window.removeEventListener('beforeunload', onUnload)
  }, [dirty])

  // ── Creating components ──

  const place = (w: number, h: number, x?: number, y?: number) => {
    const n = doc.widgets.length % 8
    const px = x ?? Math.round((STAGE_W / 2 - w / 2 + n * 20) / 10) * 10
    const py = y ?? Math.round((STAGE_H / 2 - h / 2 + n * 20) / 10) * 10
    return { x: Math.max(0, Math.min(STAGE_W - w, px)), y: Math.max(0, Math.min(STAGE_H - h, py)) }
  }

  const make = useCallback((kind: 'symbol' | 'widget', key: string, x?: number, y?: number, tagId: number | null = null, config: Record<string, any> = {}): WidgetInstance | null => {
    let type: ScadaWidgetType, size: [number, number], cfg: Record<string, any>
    if (kind === 'symbol') {
      const def = SYMBOL_MAP[key]
      if (!def) return null
      type = 'symbol'; size = def.size; cfg = { symbol: key, ...config }
    } else {
      const m = WIDGET_MANIFESTS[key as ScadaWidgetType]
      if (!m) return null
      type = key as ScadaWidgetType; size = [m.defaultWidth, m.defaultHeight]; cfg = { ...config }
    }
    const pos = place(size[0], size[1], x, y)
    return { id: newId('w'), type, sensorId: tagId, sensorIds: tagId != null ? [tagId] : [], x: pos.x, y: pos.y, width: size[0], height: size[1], config: cfg }
  }, [doc.widgets.length]) // eslint-disable-line react-hooks/exhaustive-deps

  const add = useCallback((w: WidgetInstance | null) => {
    if (!w) return
    edit({ type: 'add', widgets: [w] })
    setSelection([w.id])
  }, [edit, setSelection])

  const addEquipment = useCallback((key: string, x?: number, y?: number) => {
    const e = EQUIPMENT.find(q => q.key === key)
    if (!e) return
    // The equipment's faceplate definition: the one of that kind, or a starting one saved now.
    let fp = faceplates.find(f => f.id === e.faceplate) ?? faceplates.find(f => f.kind === e.faceplate)
    if (!fp) {
      fp = e.faceplate === 'motor' ? MOTOR_PRESET : FACEPLATE_PRESETS.find(p => p.id === e.faceplate)!
      createFaceplate.mutate(fp, { onError: (err: any) => toast.error(`Faceplate not created: ${err?.message || 'request failed'}`) })
    }
    const n = doc.widgets.filter(w => w.config?.symbol === e.symbol).length + 1
    add(make('symbol', e.symbol, x != null ? x - SYMBOL_MAP[e.symbol].size[0] / 2 : undefined, y != null ? y - SYMBOL_MAP[e.symbol].size[1] / 2 : undefined, null,
      { label: `${e.prefix}-${100 + n}`, showStatus: true, faceplate: fp.id, faceplateMap: {} }))
  }, [faceplates, createFaceplate, doc.widgets, add, make])

  const onDrop = useCallback((d: CanvasDrop, x: number, y: number, target: string | null, client: { x: number; y: number }) => {
    if (d.kind === 'tag') {
      const tw = target ? doc.widgets.find(w => w.id === target) : undefined
      const takes = tw && (WIDGET_MANIFESTS[tw.type]?.maxBindings ?? 0) > 0 && !isLocked(tw)
      if (tw && takes) {
        // Onto a component: bind it (a multi-tag component such as a table or multi-pen trend gets one more tag).
        const multi = (WIDGET_MANIFESTS[tw.type]?.maxBindings ?? 1) > 1
        patch({ [tw.id]: multi
          ? { sensorIds: [...new Set([...(tw.sensorIds ?? []), d.id])], sensorId: tw.sensorId ?? d.id }
          : bindPrimaryPatch(d.id) })
        // Selected, so the property editor shows the new tag.
        setSelection([tw.id])
        return
      }
      setDropMenu({ tagId: d.id, x, y, cx: client.x, cy: client.y })
      return
    }
    if (d.kind === 'equipment') { addEquipment(d.key, x, y); return }
    if (d.kind === 'symbol') {
      const def = SYMBOL_MAP[d.key]
      add(make('symbol', d.key, def ? x - def.size[0] / 2 : x, def ? y - def.size[1] / 2 : y))
      return
    }
    const m = WIDGET_MANIFESTS[d.type as ScadaWidgetType]
    add(make('widget', d.type, m ? x - m.defaultWidth / 2 : x, m ? y - m.defaultHeight / 2 : y))
  }, [doc.widgets, sensors, patch, setSelection, addEquipment, add, make])

  const pickDrop = (c: { kind: 'symbol' | 'widget'; key: string }) => {
    if (!dropMenu) return
    const s = sensors.find(q => q.id === dropMenu.tagId)
    const size = c.kind === 'symbol' ? SYMBOL_MAP[c.key]?.size : [WIDGET_MANIFESTS[c.key as ScadaWidgetType]?.defaultWidth, WIDGET_MANIFESTS[c.key as ScadaWidgetType]?.defaultHeight]
    const [w, hgt] = size ?? [100, 60]
    const cfg = c.kind === 'symbol' && s ? { label: s.name, ...(SYMBOL_MAP[c.key]?.binding === 'state' ? { showStatus: true } : {}) } : {}
    add(make(c.kind, c.key, Math.round((dropMenu.x - w / 2) / 10) * 10, Math.round((dropMenu.y - hgt / 2) / 10) * 10, dropMenu.tagId, cfg))
    setDropMenu(null)
  }

  // ── Selection operations ──

  const selWidgets = useMemo(() => doc.widgets.filter(w => selection.includes(w.id)), [doc.widgets, selection])
  const align = (m: AlignMode) => patch(alignPatches(doc, selection, m))
  const distribute = (a: 'h' | 'v') => patch(distributePatches(doc, selection, a))
  const matchSize = (dim: 'width' | 'height') => patch(matchSizePatches(doc, selection, dim))
  const group = () => { if (selection.length > 1) patch(groupPatches(doc, selection)) }
  const ungroup = () => patch(ungroupPatches(doc, selection))
  const allLocked = selWidgets.length > 0 && selWidgets.every(isLocked)
  const toggleLock = () => patch(lockPatches(doc, selection, !allLocked))
  const order = (to: 'front' | 'back' | 'forward' | 'backward') => { if (selection.length) edit({ type: 'order', ids: selection, to }) }

  const removeSelection = useCallback(() => {
    if (selConn) { edit({ type: 'removeConnection', id: selConn }); setSelConn(null); return }
    const ids = selWidgets.filter(w => !isLocked(w)).map(w => w.id)
    if (ids.length) edit({ type: 'remove', ids })
    if (ids.length < selWidgets.length) toast.info('Locked components were kept; unlock them to delete')
  }, [selConn, selWidgets, edit])

  const copy = useCallback(() => {
    if (!selWidgets.length) return
    const ids = new Set(selWidgets.map(w => w.id))
    clipboard.current = { widgets: selWidgets, connections: doc.connections.filter(c => ids.has(c.fromWidgetId) && ids.has(c.toWidgetId)), pastes: 0 }
  }, [selWidgets, doc.connections])

  const paste = useCallback(() => {
    const c = clipboard.current
    if (!c) return
    c.pastes += 1
    const out = duplicate(c.widgets, c.connections, 20 * c.pastes)
    edit({ type: 'add', widgets: out.widgets, connections: out.connections })
    setSelection(out.widgets.map(w => w.id))
  }, [edit, setSelection])

  const duplicateSel = useCallback(() => {
    if (!selWidgets.length) return
    const ids = new Set(selWidgets.map(w => w.id))
    const out = duplicate(selWidgets, doc.connections.filter(c => ids.has(c.fromWidgetId) && ids.has(c.toWidgetId)), 20)
    edit({ type: 'add', widgets: out.widgets, connections: out.connections })
    setSelection(out.widgets.map(w => w.id))
  }, [selWidgets, doc.connections, edit, setSelection])

  const connectPick = (id: string) => {
    if (connectFrom && connectFrom !== id) {
      const c: ConnectionInstance = { id: newId('c'), fromWidgetId: connectFrom, toWidgetId: id, connectionType: connectType, waypoints: [] }
      edit({ type: 'addConnection', connection: c })
      setConnectFrom(null)
      setSelConn(c.id)
      setSelectionRaw([])
    } else setConnectFrom(id)
  }

  const zoomBy = useCallback((dir: 1 | -1) => {
    const cur = zoom === 'fit' ? scale : zoom
    const next = dir > 0 ? ZOOM_STEPS.find(z => z > cur + 0.001) : [...ZOOM_STEPS].reverse().find(z => z < cur - 0.001)
    if (next) { setZoom(next); setScale(next) }
  }, [zoom, scale])

  // ── Keyboard ──

  const keys = useRef<Record<string, () => void>>({})
  keys.current = {
    save: () => { void save() }, undo: () => dispatch({ type: 'undo' }), redo: () => dispatch({ type: 'redo' }),
    copy, paste, cut: () => { copy(); removeSelection() }, duplicate: duplicateSel, remove: removeSelection,
    all: () => setSelection(doc.widgets.filter(w => !isLocked(w)).map(w => w.id)),
    group, ungroup, lock: toggleLock, front: () => order('front'), back: () => order('back'), forward: () => order('forward'), backward: () => order('backward'),
    preview: () => setPreview(p => !p), fit: () => setZoom('fit'), zoomIn: () => zoomBy(1), zoomOut: () => zoomBy(-1),
    escape: () => { setSelection([]); setSelConn(null); setConnectFrom(null); setTool('select'); setDropMenu(null) },
  }
  const nudge = useRef<(dx: number, dy: number) => void>(() => {})
  nudge.current = (dx, dy) => patch(movePatches(doc, selection, dx, dy, { w: STAGE_W, h: STAGE_H }), `nudge:${selection.join(',')}`)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isTyping(e.target)) return
      const k = keys.current
      const mod = e.ctrlKey || e.metaKey
      const key = e.key.toLowerCase()
      if (e.key === 'F5' || (mod && key === 'p')) { e.preventDefault(); k.preview(); return }
      if (preview) { if (key === 'escape') k.preview(); return }
      if (mod) {
        const map: Record<string, () => void> = {
          s: k.save, z: e.shiftKey ? k.redo : k.undo, y: k.redo, c: k.copy, v: k.paste, x: k.cut, d: k.duplicate, a: k.all,
          g: e.shiftKey ? k.ungroup : k.group, l: k.lock, ']': e.shiftKey ? k.front : k.forward, '}': k.front, '[': e.shiftKey ? k.back : k.backward, '{': k.back,
          '0': k.fit, '=': k.zoomIn, '+': k.zoomIn, '-': k.zoomOut,
        }
        const f = map[key]
        if (f) { e.preventDefault(); f() }
        return
      }
      if (key === 'delete' || key === 'backspace') { e.preventDefault(); k.remove(); return }
      if (key === 'escape') { k.escape(); return }
      if (key === 'v') { setTool('select'); setConnectFrom(null); return }
      if (key === 'c') { setTool('connect'); setSelection([]); return }
      if (key.startsWith('arrow')) {
        e.preventDefault()
        const step = e.shiftKey ? 10 : 1
        nudge.current(key === 'arrowleft' ? -step : key === 'arrowright' ? step : 0, key === 'arrowup' ? -step : key === 'arrowdown' ? step : 0)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [preview, setSelection])

  // ── Render ──

  const screenName = screens.find(s => s.id === screen)?.name ?? (screen === 'all' ? 'Overview' : screen)
  const n = selWidgets.length
  const selBox = boundsOf(selWidgets)
  const previewRows = useMemo(() => (preview ? saveLayoutToAPI(doc, w => widgetTitle(w)) : null), [preview, doc])

  const arrangeButtons = (
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 2, padding: 6 }}>
      <ToolBtn icon="alignLeft" title="Align left" onClick={() => align('left')} disabled={n < 2} />
      <ToolBtn icon="alignHCenter" title="Align centres horizontally" onClick={() => align('hcenter')} disabled={n < 2} />
      <ToolBtn icon="alignRight" title="Align right" onClick={() => align('right')} disabled={n < 2} />
      <ToolBtn icon="alignTop" title="Align top" onClick={() => align('top')} disabled={n < 2} />
      <ToolBtn icon="alignVMiddle" title="Align middles vertically" onClick={() => align('vmiddle')} disabled={n < 2} />
      <ToolBtn icon="alignBottom" title="Align bottom" onClick={() => align('bottom')} disabled={n < 2} />
      <ToolBtn icon="distH" title="Distribute horizontally" onClick={() => distribute('h')} disabled={n < 3} />
      <ToolBtn icon="distV" title="Distribute vertically" onClick={() => distribute('v')} disabled={n < 3} />
    </div>
  )

  return (
    <div data-testid="designer" role="application" aria-label="Voltrus Designer" style={{
      position: 'fixed', inset: 0, zIndex: 60, display: 'flex', flexDirection: 'column', background: 'var(--bg)', color: 'var(--fg)', fontSize: D.font,
      userSelect: 'none',
    }}>
      {/* Title bar */}
      <div style={{ height: 34, flexShrink: 0, display: 'flex', alignItems: 'center', gap: 10, padding: '0 8px 0 12px', borderBottom: `1px solid ${D.line}`, background: D.pane }}>
        <span style={{ fontWeight: 700, letterSpacing: '.01em' }}>Voltrus Designer</span>
        <span style={{ fontSize: 10, padding: '1px 5px', borderRadius: 3, border: `1px solid ${D.line}`, color: 'var(--text-muted)' }}>beta</span>
        <Sep />
        <Icon name="screen" size={13} />
        <span data-testid="designer-screen-name" style={{ fontWeight: 600 }}>{screenName}</span>
        {dirty && <span title="Unsaved changes" style={{ width: 7, height: 7, borderRadius: 4, background: 'var(--warn)' }} />}
        <div style={{ flex: 1 }} />
        <div role="radiogroup" aria-label="Mode" style={{ display: 'flex', border: `1px solid ${D.line}`, borderRadius: 4, overflow: 'hidden' }}>
          {(['design', 'preview'] as const).map(m => (
            <button key={m} type="button" role="radio" aria-checked={(m === 'preview') === preview} data-testid={`designer-mode-${m}`}
              onClick={() => setPreview(m === 'preview')} title={m === 'preview' ? 'Run the screen with live data (F5)' : 'Edit the screen'}
              style={{
                height: 24, padding: '0 12px', display: 'flex', alignItems: 'center', gap: 5, border: 'none', cursor: 'pointer', fontSize: D.small, fontWeight: 600,
                background: (m === 'preview') === preview ? D.accentBg : 'transparent', color: (m === 'preview') === preview ? D.accent : 'var(--text-dim)',
              }}>
              <Icon name={m === 'preview' ? 'play' : 'select'} size={11} />{m === 'preview' ? 'Preview' : 'Design'}
            </button>
          ))}
        </div>
        <ThemeToggle size={16} />
        <ToolBtn icon="close" title="Close the Designer" onClick={() => { void close() }} testId="designer-close" />
      </div>

      {/* Toolbar */}
      {!preview && (
        <div role="toolbar" aria-label="Designer tools" style={{ height: 32, flexShrink: 0, display: 'flex', alignItems: 'center', gap: 2, padding: '0 6px', borderBottom: `1px solid ${D.line}`, background: D.chrome, overflowX: 'auto' }}>
          <ToolBtn icon="save" title="Save (Ctrl+S)" label={saving ? 'Saving…' : dirty ? 'Save' : 'Saved'} onClick={() => { void save() }} disabled={!dirty || saving} active={dirty} testId="designer-save" />
          <ToolBtn icon="undo" title={`Undo (Ctrl+Z) · ${h.past.length} step${h.past.length === 1 ? '' : 's'}`} onClick={() => dispatch({ type: 'undo' })} disabled={!h.past.length} testId="designer-undo" />
          <ToolBtn icon="redo" title="Redo (Ctrl+Y)" onClick={() => dispatch({ type: 'redo' })} disabled={!h.future.length} testId="designer-redo" />
          <Sep />
          <ToolBtn icon="select" title="Select (V)" onClick={() => { setTool('select'); setConnectFrom(null) }} active={tool === 'select'} />
          <ToolBtn icon="connect" title="Connect (C): click source, then target" onClick={() => { setTool('connect'); setSelection([]) }} active={tool === 'connect'} />
          {tool === 'connect' && (
            <select value={connectType} onChange={e => setConnectType(e.target.value as ConnectionInstance['connectionType'])} aria-label="Connection type"
              style={{ height: 22, fontSize: D.small, background: 'var(--input-bg)', color: 'var(--fg)', border: `1px solid ${D.line}`, borderRadius: 3 }}>
              <option value="flow">Process flow</option><option value="power">Power</option><option value="data">Signal</option>
            </select>
          )}
          <Sep />
          <ToolBtn icon="alignLeft" title="Align left" onClick={() => align('left')} disabled={n < 2} testId="designer-align-left" />
          <ToolBtn icon="alignHCenter" title="Align centres horizontally" onClick={() => align('hcenter')} disabled={n < 2} testId="designer-align-hcenter" />
          <ToolBtn icon="alignRight" title="Align right" onClick={() => align('right')} disabled={n < 2} />
          <ToolBtn icon="alignTop" title="Align top" onClick={() => align('top')} disabled={n < 2} testId="designer-align-top" />
          <ToolBtn icon="alignVMiddle" title="Align middles vertically" onClick={() => align('vmiddle')} disabled={n < 2} />
          <ToolBtn icon="alignBottom" title="Align bottom" onClick={() => align('bottom')} disabled={n < 2} />
          <ToolBtn icon="distH" title="Distribute horizontally (3 or more)" onClick={() => distribute('h')} disabled={n < 3} />
          <ToolBtn icon="distV" title="Distribute vertically (3 or more)" onClick={() => distribute('v')} disabled={n < 3} />
          <ToolBtn icon="sameW" title="Same width as the first selected" onClick={() => matchSize('width')} disabled={n < 2} />
          <ToolBtn icon="sameH" title="Same height as the first selected" onClick={() => matchSize('height')} disabled={n < 2} />
          <Sep />
          <ToolBtn icon="group" title="Group (Ctrl+G)" onClick={group} disabled={n < 2} testId="designer-group" />
          <ToolBtn icon="ungroup" title="Ungroup (Ctrl+Shift+G)" onClick={ungroup} disabled={!selWidgets.some(groupOf)} />
          <ToolBtn icon={allLocked ? 'unlock' : 'lock'} title={allLocked ? 'Unlock (Ctrl+L)' : 'Lock (Ctrl+L)'} onClick={toggleLock} disabled={!n} active={allLocked} testId="designer-lock" />
          <Sep />
          <ToolBtn icon="front" title="Bring to front (Ctrl+Shift+])" onClick={() => order('front')} disabled={!n} />
          <ToolBtn icon="forward" title="Bring forward (Ctrl+])" onClick={() => order('forward')} disabled={!n} />
          <ToolBtn icon="backward" title="Send backward (Ctrl+[)" onClick={() => order('backward')} disabled={!n} />
          <ToolBtn icon="back" title="Send to back (Ctrl+Shift+[)" onClick={() => order('back')} disabled={!n} />
          <Sep />
          <ToolBtn icon="copy" title="Duplicate (Ctrl+D)" onClick={duplicateSel} disabled={!n} />
          <ToolBtn icon="trash" title="Delete (Del)" onClick={removeSelection} disabled={!n && !selConn} />
          <div style={{ flex: 1 }} />
          <ToolBtn icon="grid" title="Grid" onClick={() => setShowGrid(g => !g)} active={showGrid} />
          <ToolBtn icon="snap" title="Snap to grid, guides and components (hold Alt to bypass)" onClick={() => setSnap(s => !s)} active={snap} testId="designer-snap" />
          <ToolBtn icon="ruler" title="Rulers (drag from a ruler to add a guide)" onClick={() => setShowRulers(r => !r)} active={showRulers} />
          <Sep />
          <ToolBtn icon="zoomOut" title="Zoom out (Ctrl+-)" onClick={() => zoomBy(-1)} />
          <button type="button" onClick={() => setZoom('fit')} title="Fit (Ctrl+0)" style={{ ...mono, width: 46, height: 22, border: `1px solid ${D.line}`, borderRadius: 3, background: 'transparent', color: zoom === 'fit' ? D.accent : 'var(--fg)', cursor: 'pointer' }}>
            {Math.round(scale * 100)}%
          </button>
          <ToolBtn icon="zoomIn" title="Zoom in (Ctrl++)" onClick={() => zoomBy(1)} />
        </div>
      )}

      {/* Body */}
      <div style={{ flex: 1, minHeight: 0, display: 'flex' }}>
        {!preview && (
          <aside style={{ width: 264, flexShrink: 0, display: 'flex', flexDirection: 'column', borderRight: `1px solid ${D.line}`, background: D.pane }}>
            <Pane title="Project" icon="folder" grow={1} testId="designer-project">
              <ProjectBrowser screens={screens} active={screen} onOpenScreen={id => { void openScreen(id) }} doc={doc} selection={selection} onSelect={setSelection}
                onToggleLock={id => patch(lockPatches(doc, [id], !isLocked(doc.widgets.find(w => w.id === id)!)))}
                onToggleVisible={id => { const w = doc.widgets.find(x => x.id === id)!; patch({ [id]: { config: { visible: w.config?.visible === false ? undefined : false } } }) }}
                faceplates={faceplates} onOpenPage={onOpenPage} />
            </Pane>
            <Pane title="Tag browser" icon="tag" grow={1.3} testId="designer-tag-browser">
              <TagBrowserPane sensors={sensors} onPick={id => {
                if (selection.length === 1) { const w = doc.widgets.find(x => x.id === selection[0]); if (w && (WIDGET_MANIFESTS[w.type]?.maxBindings ?? 0) > 0) patch({ [w.id]: bindPrimaryPatch(id) }) }
              }} />
            </Pane>
          </aside>
        )}

        <main style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', position: 'relative' }}>
          {preview ? (
            <div data-testid="designer-preview" style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
              <ScadaView
                sensors={sensors}
                activeScreen={`preview:${screen}`}
                scadaEquipment={previewRows!.equipment}
                scadaConnections={previewRows!.connections}
                loaded
                onSave={async () => {}}
                onSensorClick={id => openTrend(id)}
                isAdmin={false}
              />
            </div>
          ) : !loaded && loadedScreen.current !== screen ? (
            <div style={{ flex: 1, display: 'grid', placeItems: 'center', color: 'var(--text-muted)' }}>Loading {screenName}…</div>
          ) : (
            <DesignerCanvas
              doc={doc} sensors={sensors} selection={selection} scale={scale} zoom={zoom}
              onScaleChange={setScale} onZoomTo={s => { setZoom(s); setScale(s) }}
              showGrid={showGrid} snap={snap} showRulers={showRulers}
              guides={guides} onGuidesChange={g => setGuidesByScreen(all => ({ ...all, [screen]: g }))}
              tool={tool} connectFrom={connectFrom} onConnectPick={connectPick}
              selectedConnection={selConn} onSelectConnection={id => { setSelConn(id); setSelectionRaw([]) }}
              onSelect={setSelection} onEdit={edit} onDrop={onDrop} onCursor={setCursor}
            />
          )}
          {!preview && tool === 'connect' && (
            <div style={{ position: 'absolute', top: 30, left: '50%', transform: 'translateX(-50%)', zIndex: 5, padding: '4px 12px', borderRadius: 999, fontSize: D.small, background: D.pane, border: `1px solid ${D.line}`, color: 'var(--text-dim)' }}>
              {connectFrom ? 'Now click the target component · Esc to cancel' : 'Click the source component'}
            </div>
          )}
          {!preview && loaded && doc.widgets.length === 0 && (
            <div style={{ position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', pointerEvents: 'none', color: 'var(--text-muted)', textAlign: 'center', lineHeight: 1.7 }}>
              <div>Drag a tag from the <b>Tag browser</b> onto the screen for a bound component,<br />or a symbol from <b>Components</b>.</div>
            </div>
          )}
          {dropMenu && (
            <DropMenu at={{ x: dropMenu.cx, y: dropMenu.cy }} tag={sensors.find(s => s.id === dropMenu.tagId)} onPick={pickDrop} onClose={() => setDropMenu(null)} />
          )}
        </main>

        {!preview && (
          <aside style={{ width: 312, flexShrink: 0, display: 'flex', flexDirection: 'column', borderLeft: `1px solid ${D.line}`, background: D.pane }}>
            <Pane title="Components" icon="layers" grow={0.9} testId="designer-components">
              <PalettePane onAdd={it => {
                if (it.kind === 'equipment') addEquipment(it.key)
                else add(make(it.kind, it.key))
              }} />
            </Pane>
            <Pane title="Properties" icon="faceplate" grow={1.6} testId="designer-properties"
              actions={n ? <span style={{ ...mono, color: 'var(--text-muted)' }}>{n} selected</span> : undefined}>
              {selConn ? (
                <ConnectionProps conn={doc.connections.find(c => c.id === selConn)!} widgets={doc.widgets}
                  onChange={p => edit({ type: 'patchConnection', id: selConn, patch: p })} onDelete={removeSelection} />
              ) : (
                <PropertyEditor doc={doc} selection={selection} sensors={sensors} screenName={screenName} onPatch={patch} arrange={arrangeButtons} />
              )}
            </Pane>
          </aside>
        )}
      </div>

      {/* Status bar */}
      <div data-testid="designer-status" style={{ height: 22, flexShrink: 0, display: 'flex', alignItems: 'center', gap: 14, padding: '0 10px', borderTop: `1px solid ${D.line}`, background: D.chrome, ...mono, color: 'var(--text-dim)' }}>
        <span style={{ color: preview ? D.accent : undefined }}>{preview ? 'PREVIEW · live data · F5 or Esc to edit' : 'DESIGN'}</span>
        {!preview && <span>{cursor ? `x ${cursor.x}  y ${cursor.y}` : 'x —  y —'}</span>}
        {!preview && <span>{n ? `${n} selected · ${selBox!.x}, ${selBox!.y} · ${selBox!.width} × ${selBox!.height}` : `${doc.widgets.length} components`}</span>}
        <span>{guides.length} guide{guides.length === 1 ? '' : 's'}</span>
        <div style={{ flex: 1 }} />
        <LiveCount sensors={sensors} />
        <span>{h.past.length} undo · {h.future.length} redo</span>
        <span style={{ color: dirty ? 'var(--warn)' : undefined }} data-testid="designer-dirty">{dirty ? 'Unsaved changes' : 'All changes saved'}</span>
      </div>
    </div>
  )
}

/** Own component: it re-renders on every live value, the Designer does not. */
function LiveCount({ sensors }: { sensors: SensorConfig[] }) {
  const values = useTrackedCurrentValues()
  const live = sensors.filter(s => values[String(s.id)] != null).length
  return (
    <span title="Tags with a live value">
      <span style={{ display: 'inline-block', width: 7, height: 7, borderRadius: 4, marginRight: 5, background: live ? 'var(--ok)' : 'var(--text-muted)' }} />
      {live}/{sensors.length} tags live
    </span>
  )
}

function DropMenu({ at, tag, onPick, onClose }: {
  at: { x: number; y: number }; tag: SensorConfig | undefined
  onPick: (c: { kind: 'symbol' | 'widget'; key: string }) => void; onClose: () => void
}) {
  const choices = dropChoices(tag)
  const [i, setI] = useState(0)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.stopPropagation(); onClose() }
      if (e.key === 'ArrowDown') { e.preventDefault(); setI(x => Math.min(choices.length - 1, x + 1)) }
      if (e.key === 'ArrowUp') { e.preventDefault(); setI(x => Math.max(0, x - 1)) }
      if (e.key === 'Enter') { e.preventDefault(); onPick(choices[i]) }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [choices, i, onPick, onClose])
  return (
    <>
      <div onPointerDown={onClose} style={{ position: 'fixed', inset: 0, zIndex: 70 }} />
      <div role="menu" aria-label="Show tag as" data-testid="designer-drop-menu" style={{
        position: 'fixed', left: Math.min(at.x, window.innerWidth - 230), top: Math.min(at.y, window.innerHeight - 300), zIndex: 71, width: 220,
        background: 'var(--card)', border: `1px solid ${D.line}`, borderRadius: 4, boxShadow: '0 8px 28px rgba(0,0,0,.45)', padding: 3,
      }}>
        <div style={{ padding: '4px 8px 5px', fontSize: D.small, color: 'var(--text-muted)', borderBottom: `1px solid ${D.line}`, marginBottom: 3 }}>
          Show <b style={{ color: 'var(--fg)' }}>{tag?.name ?? 'tag'}</b> as
        </div>
        {choices.map((c, j) => (
          <button key={c.key} role="menuitem" type="button" onClick={() => onPick(c)} onMouseEnter={() => setI(j)} data-testid={`designer-drop-${c.key}`} style={{
            display: 'flex', width: '100%', alignItems: 'center', height: 24, padding: '0 8px', border: 'none', borderRadius: 3, cursor: 'pointer', fontSize: D.font,
            background: j === i ? D.selBg : 'transparent', color: 'var(--fg)', textAlign: 'left',
          }}>
            <span style={{ flex: 1 }}>{c.label}</span>
            {j === 0 && <span style={{ ...mono, color: 'var(--text-muted)' }}>Enter</span>}
          </button>
        ))}
      </div>
    </>
  )
}

function ConnectionProps({ conn, widgets, onChange, onDelete }: {
  conn: ConnectionInstance; widgets: WidgetInstance[]; onChange: (p: Partial<ConnectionInstance>) => void; onDelete: () => void
}) {
  if (!conn) return null
  const name = (id: string) => { const w = widgets.find(x => x.id === id); return w ? (w.config?.label || w.name || widgetTitle(w)) : id }
  return (
    <div style={{ padding: 10, display: 'flex', flexDirection: 'column', gap: 8, fontSize: D.font }}>
      <div><b>Connection</b> <span style={{ ...mono, color: 'var(--text-muted)' }}>{conn.id}</span></div>
      <div>{name(conn.fromWidgetId)} → {name(conn.toWidgetId)}</div>
      <label style={{ display: 'flex', gap: 8, alignItems: 'center' }}>Type
        <select value={conn.connectionType} onChange={e => onChange({ connectionType: e.target.value as ConnectionInstance['connectionType'] })}
          style={{ height: 22, background: 'var(--input-bg)', color: 'var(--fg)', border: `1px solid ${D.line}`, borderRadius: 3 }}>
          <option value="flow">Process flow</option><option value="power">Power</option><option value="data">Signal</option>
        </select>
      </label>
      <div><SmallBtn danger onClick={onDelete}>Delete connection</SmallBtn></div>
    </div>
  )
}
