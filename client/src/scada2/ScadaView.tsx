import React, { useReducer, useState, useCallback, useRef, useEffect } from 'react'
import type { ScadaWidgetType, ConnectionInstance, WidgetInstance } from './widgets/types'
import { WIDGET_MANIFESTS, getCompatibleWidgetTypes, getWidgetComponent } from './widgets/registry'
import { ScadaCanvas, STAGE_W, STAGE_H, type DropPayload } from './ScadaCanvas'
import { ScadaPalette, PANEL_WIDTH } from './ScadaPalette'
import { WidgetInspector, ConnectionInspector, ScreenInspector, INSPECTOR_WIDTH, widgetTitle } from './ScadaInspector'
import { ConnectionLayer, CONNECTION_TYPES } from './connections/ConnectionLayer'
import { historyReducer, emptyHistory, createWidget, createConnection, type ScadaLayoutState } from './engine/widgetState'
import { loadLayoutFromAPI } from './engine/layout'
import { SYMBOL_MAP, SYMBOLS } from './hmi/symbols'
import { SymbolThumb } from './hmi/SymbolView'
import { Faceplate } from './hmi/Faceplate'
import { AlarmBanner } from './AlarmBanner'
import type { SensorConfig, SensorData, ScadaEquipmentItem, ScadaConnectionItem } from '../types'
import { useDashboardSelector } from '../store/dashboardStore'
import { wsWriteTagResult } from '../api/websocket'
import { writeTag } from '../api/commands'
import { toast } from '../components/Toast'

// Import all widgets to trigger registration
import './widgets/SymbolWidget'
import './widgets/AreaWidget'
import './widgets/IndicatorWidget'
import './widgets/GaugeWidget'
import './widgets/BarWidget'
import './widgets/StatusLightWidget'
import './widgets/OnOffWidget'
import './widgets/LabelWidget'
import './widgets/TrendWidget'
import './widgets/BigNumberWidget'
import './widgets/DataTableWidget'
import './widgets/SliderWidget'
import './widgets/NumberInputWidget'
import './widgets/ToggleWidget'
import './widgets/MeterWidget'
import './widgets/SemicircleGaugeWidget'
import './widgets/DonutWidget'
import './widgets/LEDDisplayWidget'
import './widgets/PushButtonWidget'
import './widgets/BarChartWidget'
import './widgets/PieChartWidget'
import './widgets/MultiTrendWidget'
import './widgets/AnnunciatorWidget'
import './widgets/PIDFaceplateWidget'
import './widgets/ProgressBarWidget'

interface Props {
  sensors: SensorConfig[]
  activeScreen: string
  scadaEquipment: ScadaEquipmentItem[]
  scadaConnections: ScadaConnectionItem[]
  /** Both queries have returned for the active screen; editing before that would overwrite it with nothing. */
  loaded: boolean
  /** Resolves once both PUTs and their refetches have finished. */
  onSave: (screenId: string, equipment: ScadaEquipmentItem[], connections: ScadaConnectionItem[]) => Promise<void>
  onSensorClick: (sensorId: number) => void
  isAdmin: boolean
  toolbarSlotSetter?: (slot: React.ReactNode) => void
}

type Selection = { kind: 'widget' | 'connection'; id: string } | null
type Tool = 'select' | 'connect'

const EQUIPMENT_TYPES = new Set<ScadaWidgetType>(['symbol', 'pump', 'motor', 'valve', 'tank_level', 'horizontal_tank', 'pipe'])
// Clicking these at runtime opens a faceplate for the bound tag. Operator controls (slider, toggle…) keep their own clicks.
const FACEPLATE_TYPES = new Set<ScadaWidgetType>([
  ...EQUIPMENT_TYPES, 'gauge', 'indicator', 'bar', 'status_light', 'big_number', 'meter', 'semicircle_gauge', 'donut',
  'led_display', 'trend', 'progress_bar', 'on_off',
])

function toAPI(layout: ScadaLayoutState, sensors: SensorConfig[]) {
  const equipment: ScadaEquipmentItem[] = layout.widgets.map((w, i) => ({
    equipment_id: w.id,
    equipment_type: w.type as any,
    equipment_name: w.config?.label || sensors.find(s => s.id === w.sensorId)?.name || widgetTitle(w),
    x: w.x,
    y: w.y,
    width: w.width,
    height: w.height,
    rotation: Number(w.config?.rotation ?? 0),
    sensor_bindings: JSON.stringify(w.sensorIds ?? (w.sensorId != null ? [w.sensorId] : [])),
    config: JSON.stringify({ ...w.config, z: i }),
  }))
  const connections: ScadaConnectionItem[] = layout.connections.map(c => ({
    connection_id: c.id,
    from_equipment: c.fromWidgetId,
    to_equipment: c.toWidgetId,
    waypoints: JSON.stringify(c.waypoints),
    connection_type: c.connectionType,
  }))
  return { equipment, connections }
}

/** Keep a widget's box on the stage. */
const clampPos = (x: number, y: number, w: number, h: number) => ({
  x: Math.max(0, Math.min(STAGE_W - w, x)),
  y: Math.max(0, Math.min(STAGE_H - h, y)),
})

const newId = (p: string) => `${p}_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`
const isTyping = (t: EventTarget | null) =>
  t instanceof HTMLElement && (t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName))

const ZOOM_STEPS = [0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1, 1.25, 1.5, 2]

export const ScadaView: React.FC<Props> = ({
  sensors, activeScreen, scadaEquipment, scadaConnections, loaded,
  onSave, onSensorClick, isAdmin, toolbarSlotSetter,
}) => {
  const currentValues = useDashboardSelector(s => s.currentValues) as Record<string, SensorData>
  const [history, dispatch] = useReducer(historyReducer, emptyHistory)
  const layout = history.present
  const dirty = history.present !== history.saved

  const [editMode, setEditMode] = useState(false)
  const [tool, setTool] = useState<Tool>('select')
  const [connectType, setConnectType] = useState<ConnectionInstance['connectionType']>('flow')
  const [connectFrom, setConnectFrom] = useState<string | null>(null)
  const [pointer, setPointer] = useState<{ x: number; y: number } | null>(null)
  const [selection, setSelection] = useState<Selection>(null)
  const [pendingSensor, setPendingSensor] = useState<{ sensorId: number; x: number; y: number } | null>(null)
  const [zoom, setZoom] = useState<'fit' | number>('fit')
  const [scale, setScale] = useState(1)
  const [showGrid, setShowGrid] = useState(true)
  const [saving, setSaving] = useState(false)
  const [faceplateId, setFaceplateId] = useState<string | null>(null)
  const clipboard = useRef<WidgetInstance | null>(null)
  const loadedScreen = useRef<string | null>(null)
  const savingRef = useRef(false)
  /** Screen whose auto-save failed on switch; its edits stay on screen until a manual save succeeds. */
  const autoSaveFailed = useRef<string | null>(null)

  /** Persist `snapshot` to `screenId`. Only marks clean if that screen is still the one being edited. */
  const saveLayout = useCallback(async (screenId: string, snapshot: ScadaLayoutState, auto = false): Promise<boolean> => {
    savingRef.current = true
    setSaving(true)
    try {
      const { equipment, connections } = toAPI(snapshot, sensors)
      await onSave(screenId, equipment, connections)
      if (loadedScreen.current === screenId) dispatch({ type: 'MARK_SAVED', snapshot })
      if (autoSaveFailed.current === screenId) autoSaveFailed.current = null
      return true
    } catch (err: any) {
      // Set before `saving` flips so the reload effect never retries a failed auto-save.
      if (auto) autoSaveFailed.current = screenId
      toast.error(auto
        ? `Screen not saved: ${err?.message || 'request failed'}. Edits kept on screen; press Save to retry.`
        : `Screen not saved: ${err?.message || 'request failed'}`)
      return false
    } finally {
      savingRef.current = false
      setSaving(false)
    }
  }, [sensors, onSave])

  // Latest values for the unmount auto-save below.
  const latest = useRef({ dirty, layout, saveLayout })
  latest.current = { dirty, layout, saveLayout }

  // Load the server copy when the screen changes. Unsaved edits on the previous screen are saved first;
  // background refetches never clobber edits, and are ignored while our own save is in flight.
  useEffect(() => {
    if (!loaded || savingRef.current) return
    const sameScreen = loadedScreen.current === activeScreen
    if (sameScreen && autoSaveFailed.current === activeScreen) autoSaveFailed.current = null
    if (sameScreen && dirty) return
    if (!sameScreen && dirty && loadedScreen.current) {
      // Save the previous screen first; this effect re-runs when the save settles (`saving` dep).
      // If it failed, keep its edits on screen rather than drop them; a manual Save retries.
      const prev = loadedScreen.current
      if (autoSaveFailed.current === prev) return
      void saveLayout(prev, layout, true)
      return
    }
    const next = loadLayoutFromAPI(scadaEquipment, scadaConnections)
    // The refetch after our own save echoes what's on screen; keep undo history and selection.
    if (sameScreen && JSON.stringify(next) === JSON.stringify(layout)) return
    loadedScreen.current = activeScreen
    dispatch({ type: 'LOAD_LAYOUT', widgets: next.widgets, connections: next.connections })
    setSelection(null)
    setFaceplateId(null)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeScreen, scadaEquipment, scadaConnections, loaded, saving])

  // Leaving the SCADA view (unmount) with unsaved edits: save them rather than drop them.
  useEffect(() => () => {
    const { dirty, layout, saveLayout } = latest.current
    if (dirty && loadedScreen.current) void saveLayout(loadedScreen.current, layout)
  }, [])

  const selectedWidget = selection?.kind === 'widget' ? layout.widgets.find(w => w.id === selection.id) : undefined
  const selectedConn = selection?.kind === 'connection' ? layout.connections.find(c => c.id === selection.id) : undefined

  // ── Mutations ──

  const addWidget = useCallback((w: WidgetInstance) => {
    dispatch({ type: 'ADD_WIDGET', widget: w })
    setSelection({ kind: 'widget', id: w.id })
  }, [])

  /** Default spot for click-to-add: stage centre, cascading so items don't stack exactly. */
  const placeAt = (w: number, h: number) => {
    const n = layout.widgets.length % 8
    return clampPos(Math.round((STAGE_W / 2 - w / 2 + n * 20) / 10) * 10, Math.round((STAGE_H / 2 - h / 2 + n * 20) / 10) * 10, w, h)
  }

  const addSymbol = useCallback((key: string, x?: number, y?: number, sensorId: number | null = null) => {
    const def = SYMBOL_MAP[key]
    if (!def) return
    const [w, h] = def.size
    const pos = x != null && y != null ? clampPos(x, y, w, h) : placeAt(w, h)
    const widget = createWidget('symbol', sensorId, pos.x, pos.y, w, h)
    widget.config = { symbol: key }
    addWidget(widget)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [addWidget, layout.widgets.length])

  const addWidgetType = useCallback((type: ScadaWidgetType, x?: number, y?: number, sensorId: number | null = null) => {
    const m = WIDGET_MANIFESTS[type]
    const pos = x != null && y != null ? clampPos(x, y, m.defaultWidth, m.defaultHeight) : placeAt(m.defaultWidth, m.defaultHeight)
    addWidget(createWidget(type, sensorId, pos.x, pos.y, m.defaultWidth, m.defaultHeight))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [addWidget, layout.widgets.length])

  const handleDrop = useCallback((p: DropPayload, x: number, y: number) => {
    if (p.kind === 'symbol') addSymbol(p.key, x, y)
    else if (p.kind === 'widget') addWidgetType(p.type as ScadaWidgetType, x, y)
    else setPendingSensor({ sensorId: p.id, x, y })
  }, [addSymbol, addWidgetType])

  const handlePickWidgetType = useCallback((type: ScadaWidgetType) => {
    if (!pendingSensor) return
    addWidgetType(type, pendingSensor.x, pendingSensor.y, pendingSensor.sensorId)
    setPendingSensor(null)
  }, [pendingSensor, addWidgetType])

  const handlePickSymbol = useCallback((key: string) => {
    if (!pendingSensor) return
    addSymbol(key, pendingSensor.x, pendingSensor.y, pendingSensor.sensorId)
    setPendingSensor(null)
  }, [pendingSensor, addSymbol])

  const deleteSelection = useCallback(() => {
    if (!selection) return
    dispatch(selection.kind === 'widget' ? { type: 'REMOVE_WIDGET', id: selection.id } : { type: 'REMOVE_CONNECTION', id: selection.id })
    setSelection(null)
  }, [selection])

  const pasteWidget = useCallback((src: WidgetInstance) => {
    addWidget({ ...src, id: newId('w'), ...clampPos(src.x + 20, src.y + 20, src.width, src.height), config: { ...src.config } })
  }, [addWidget])

  const handleSelectWidget = useCallback((id: string) => {
    if (tool === 'connect') {
      if (connectFrom && connectFrom !== id) {
        const conn = createConnection(connectFrom, id, connectType)
        dispatch({ type: 'ADD_CONNECTION', connection: conn })
        setConnectFrom(null)
        setSelection({ kind: 'connection', id: conn.id })
      } else {
        setConnectFrom(id)
      }
      return
    }
    setSelection({ kind: 'widget', id })
  }, [tool, connectFrom, connectType])

  const handleWriteValue = useCallback(async (_widgetId: string, sensorId: number, value: number) => {
    try {
      const outcome = await wsWriteTagResult(sensorId, value)
      if (outcome === 'queued') toast.info('Command queued, the device has not answered yet')
      if (outcome === null) await writeTag(sensorId, value)
    } catch (err: any) {
      toast.error(err.message || 'Write failed')
    }
  }, [])

  /** Operator command from a faceplate. Errors are toasted; resolves whether the write was accepted. */
  const sendCommand = useCallback(async (sensorId: number, value: number): Promise<boolean> => {
    try {
      const outcome = await wsWriteTagResult(sensorId, value)
      if (outcome === null) await writeTag(sensorId, value)
      if (outcome === 'queued') toast.info('Command queued, the device has not answered yet')
      else toast.success('Command sent')
      return true
    } catch (err: any) {
      toast.error(err.message || 'Write failed')
      return false
    }
  }, [])

  const handleSave = useCallback(() => {
    if (!savingRef.current && loadedScreen.current) void saveLayout(loadedScreen.current, layout)
  }, [saveLayout, layout])

  const resetTransient = () => { setTool('select'); setConnectFrom(null); setPendingSensor(null); setSelection(null) }

  const toggleEdit = useCallback(async () => {
    if (!editMode && !loaded) return
    // Done saves; stay in edit mode if the save fails so nothing is lost.
    if (editMode && dirty && loadedScreen.current && !(await saveLayout(loadedScreen.current, layout))) return
    resetTransient()
    setFaceplateId(null)
    setZoom('fit')
    setEditMode(e => !e)
  }, [editMode, dirty, loaded, layout, saveLayout])

  const zoomBy = useCallback((dir: 1 | -1) => {
    const cur = zoom === 'fit' ? scale : zoom
    const next = dir > 0 ? ZOOM_STEPS.find(z => z > cur + 0.001) : [...ZOOM_STEPS].reverse().find(z => z < cur - 0.001)
    if (next) setZoom(next)
  }, [zoom, scale])

  // ── Keyboard ──

  useEffect(() => {
    if (!editMode) return
    const onKey = (e: KeyboardEvent) => {
      if (isTyping(e.target)) return
      const mod = e.ctrlKey || e.metaKey
      const k = e.key.toLowerCase()
      if (mod && k === 's') { e.preventDefault(); handleSave(); return }
      if (mod && k === 'z') { e.preventDefault(); dispatch({ type: e.shiftKey ? 'REDO' : 'UNDO' }); return }
      if (mod && k === 'y') { e.preventDefault(); dispatch({ type: 'REDO' }); return }
      if (mod && k === 'c' && selectedWidget) { clipboard.current = selectedWidget; return }
      if (mod && k === 'v' && clipboard.current) { e.preventDefault(); pasteWidget(clipboard.current); clipboard.current = { ...clipboard.current, x: clipboard.current.x + 20, y: clipboard.current.y + 20 }; return }
      if (mod && k === 'd' && selectedWidget) { e.preventDefault(); pasteWidget(selectedWidget); return }
      if (mod && (k === '=' || k === '+')) { e.preventDefault(); zoomBy(1); return }
      if (mod && k === '-') { e.preventDefault(); zoomBy(-1); return }
      if (mod && k === '0') { e.preventDefault(); setZoom('fit'); return }
      if (mod) return
      if (k === 'escape') { resetTransient(); return }
      if (k === 'delete' || k === 'backspace') { e.preventDefault(); deleteSelection(); return }
      if (k === 'v') { setTool('select'); setConnectFrom(null); return }
      if (k === 'c') { setTool('connect'); setSelection(null); return }
      if (selectedWidget && (k === ']' || k === '[')) { dispatch({ type: 'REORDER_WIDGET', id: selectedWidget.id, to: k === ']' ? 'front' : 'back' }); return }
      if (selectedWidget && k.startsWith('arrow')) {
        e.preventDefault()
        const step = e.shiftKey ? 10 : 1
        const dx = k === 'arrowleft' ? -step : k === 'arrowright' ? step : 0
        const dy = k === 'arrowup' ? -step : k === 'arrowdown' ? step : 0
        dispatch({
          type: 'MOVE_WIDGET', id: selectedWidget.id, coalesce: `nudge:${selectedWidget.id}`,
          x: Math.max(0, Math.min(STAGE_W - selectedWidget.width, selectedWidget.x + dx)),
          y: Math.max(0, Math.min(STAGE_H - selectedWidget.height, selectedWidget.y + dy)),
        })
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [editMode, selectedWidget, handleSave, deleteSelection, pasteWidget, zoomBy])

  // Warn before closing the tab with unsaved work.
  useEffect(() => {
    if (!editMode || !dirty) return
    const onUnload = (e: BeforeUnloadEvent) => { e.preventDefault() }
    window.addEventListener('beforeunload', onUnload)
    return () => window.removeEventListener('beforeunload', onUnload)
  }, [editMode, dirty])

  // ── Toolbar (rendered into the parent view toolbar) ──

  useEffect(() => {
    if (!toolbarSlotSetter) return
    if (!isAdmin) { toolbarSlotSetter(null); return }
    const sep = <div style={{ width: 1, height: 16, background: 'var(--border)', margin: '0 2px' }} />
    toolbarSlotSetter(
      <div style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
        {editMode && (
          <>
            {sep}
            <IconBtn title="Undo (Ctrl+Z)" disabled={history.past.length === 0} onClick={() => dispatch({ type: 'UNDO' })}>
              <path d="M9 14 4 9l5-5" /><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11" />
            </IconBtn>
            <IconBtn title="Redo (Ctrl+Shift+Z)" disabled={history.future.length === 0} onClick={() => dispatch({ type: 'REDO' })}>
              <path d="m15 14 5-5-5-5" /><path d="M20 9H9.5a5.5 5.5 0 0 0 0 11H13" />
            </IconBtn>
            {sep}
            <IconBtn title="Select (V)" active={tool === 'select'} onClick={() => { setTool('select'); setConnectFrom(null) }}>
              <path d="M3 3l7.07 16.97 2.51-7.39 7.39-2.51L3 3z" />
            </IconBtn>
            <IconBtn title="Connect (C) — click source, then target" active={tool === 'connect'} onClick={() => { setTool('connect'); setSelection(null) }}>
              <circle cx="5" cy="6" r="2.5" /><circle cx="19" cy="18" r="2.5" /><path d="M7.5 6H12v12h4.5" />
            </IconBtn>
            {tool === 'connect' && (
              <select value={connectType} onChange={e => setConnectType(e.target.value as ConnectionInstance['connectionType'])}
                style={{ fontSize: 11, padding: '2px 4px', borderRadius: 4, border: '1px solid var(--border)', background: 'var(--input-bg)', color: 'var(--fg)' }}>
                {CONNECTION_TYPES.map(t => <option key={t.value} value={t.value}>{t.label}</option>)}
              </select>
            )}
            {sep}
            <IconBtn title="Toggle grid" active={showGrid} onClick={() => setShowGrid(g => !g)}>
              <rect x="3" y="3" width="18" height="18" rx="2" /><path d="M3 9h18M3 15h18M9 3v18M15 3v18" />
            </IconBtn>
            <IconBtn title="Zoom out (Ctrl+-)" onClick={() => zoomBy(-1)}><path d="M5 12h14" /></IconBtn>
            <button onClick={() => setZoom('fit')} title="Fit to window (Ctrl+0)" style={{
              minWidth: 46, padding: '3px 4px', borderRadius: 5, border: '1px solid var(--border)', background: 'transparent',
              color: zoom === 'fit' ? 'var(--cyan)' : 'var(--fg)', fontSize: 11, fontFamily: 'var(--font-mono)', cursor: 'pointer',
            }}>{Math.round(scale * 100)}%</button>
            <IconBtn title="Zoom in (Ctrl++)" onClick={() => zoomBy(1)}><path d="M12 5v14M5 12h14" /></IconBtn>
            {sep}
            <button onClick={handleSave} disabled={!dirty || saving} title="Save (Ctrl+S)" style={{
              padding: '3px 10px', borderRadius: 5, fontSize: 11, fontWeight: 600,
              border: `1px solid ${dirty ? 'var(--cyan)' : 'var(--border)'}`,
              background: dirty ? 'color-mix(in srgb, var(--cyan) 14%, transparent)' : 'transparent',
              color: dirty ? 'var(--cyan)' : 'var(--muted)', cursor: dirty ? 'pointer' : 'default',
            }}>{saving ? 'Saving…' : dirty ? 'Save' : 'Saved'}</button>
          </>
        )}
        {sep}
        <button onClick={toggleEdit} disabled={!loaded || saving} title={editMode ? 'Save and leave edit mode' : loaded ? 'Edit screen' : 'Loading screen…'} style={{
          padding: '3px 10px', borderRadius: 5, fontSize: 11, fontWeight: 600, cursor: 'pointer',
          border: `1px solid ${editMode ? 'var(--warn)' : 'var(--border)'}`,
          background: editMode ? 'color-mix(in srgb, var(--warn) 12%, transparent)' : 'transparent',
          color: editMode ? 'var(--warn)' : 'var(--fg)',
        }}>{editMode ? 'Done' : 'Edit'}</button>
      </div>,
    )
    return () => toolbarSlotSetter(null)
  }, [editMode, tool, connectType, showGrid, zoom, scale, dirty, saving, loaded, history.past.length, history.future.length, isAdmin, toolbarSlotSetter, handleSave, toggleEdit, zoomBy])

  // The inspector column is always reserved in edit mode: opening it on select would rescale the stage under the pointer.
  const rightOpen = editMode
  const faceplateWidget = !editMode && faceplateId ? layout.widgets.find(w => w.id === faceplateId) : undefined

  return (
    <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
      <div style={{ flex: 1, position: 'relative', minHeight: 420 }}>
        {editMode && (
          <ScadaPalette
            sensors={sensors}
            onAddSymbol={key => addSymbol(key)}
            onAddWidget={type => addWidgetType(type)}
            onPickTag={id => setPendingSensor({ sensorId: id, ...placeAt(100, 100) })}
          />
        )}
        <div style={{ position: 'absolute', top: 0, bottom: 0, left: editMode ? PANEL_WIDTH : 0, right: rightOpen ? INSPECTOR_WIDTH : 0 }}>
          <ScadaCanvas
            layout={layout}
            sensors={sensors}
            currentValues={currentValues}
            editMode={editMode}
            selectedId={editMode ? (selection?.kind === 'widget' ? selection.id : connectFrom) : faceplateWidget?.id ?? null}
            zoom={editMode ? zoom : 'fit'}
            showGrid={showGrid}
            cursor={tool === 'connect' ? 'crosshair' : undefined}
            locked={tool === 'connect'}
            onScaleChange={setScale}
            onSelectWidget={editMode ? handleSelectWidget : (id => {
              const w = layout.widgets.find(x => x.id === id)
              if (w && w.sensorId != null && FACEPLATE_TYPES.has(w.type)) setFaceplateId(cur => (cur === id ? null : id))
            })}
            onMoveWidget={(id, x, y) => dispatch({ type: 'MOVE_WIDGET', id, x, y })}
            onResizeWidget={(id, x, y, width, height) => dispatch({ type: 'RESIZE_WIDGET', id, x, y, width, height })}
            onDrop={handleDrop}
            onWriteValue={handleWriteValue}
            onClickEmpty={() => { setSelection(null); setConnectFrom(null); setFaceplateId(null) }}
            onPointerMove={editMode && connectFrom ? (x, y) => setPointer({ x, y }) : undefined}
          >
            <ConnectionLayer
              widgets={layout.widgets}
              connections={layout.connections}
              currentValues={currentValues}
              editMode={editMode}
              selectedId={selection?.kind === 'connection' ? selection.id : null}
              onSelect={tool === 'select' ? id => setSelection({ kind: 'connection', id }) : undefined}
              pending={connectFrom && pointer ? { fromId: connectFrom, ...pointer } : null}
            />
          </ScadaCanvas>
          {faceplateWidget && (
            <Faceplate
              key={faceplateWidget.id}
              widget={faceplateWidget}
              sensors={sensors}
              currentValues={currentValues}
              onCommand={sendCommand}
              onOpenDetail={onSensorClick}
              onClose={() => setFaceplateId(null)}
            />
          )}
          {editMode && tool === 'connect' && (
            <div style={{
              position: 'absolute', top: 10, left: '50%', transform: 'translateX(-50%)', zIndex: 15,
              padding: '5px 12px', borderRadius: 999, fontSize: 11, background: 'var(--card)', border: '1px solid var(--border)',
              color: 'var(--text-dim)', boxShadow: 'var(--shadow)',
            }}>
              {connectFrom ? 'Now click the target equipment · Esc to cancel' : 'Click the source equipment'}
            </div>
          )}
          {editMode && layout.widgets.length === 0 && (
            <div style={{
              position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', pointerEvents: 'none',
              color: 'var(--muted)', fontSize: 13, textAlign: 'center', lineHeight: 1.6,
            }}>
              <div>Drag equipment from the <b>Symbols</b> tab to start this screen.<br />Bind tags in the inspector on the right.</div>
            </div>
          )}
        </div>

        {editMode && pendingSensor && (
          <WidgetTypeSidebar
            sensorId={pendingSensor.sensorId}
            sensors={sensors}
            onSelect={handlePickWidgetType}
            onSelectSymbol={handlePickSymbol}
            onClose={() => setPendingSensor(null)}
          />
        )}
        {editMode && !pendingSensor && selectedWidget && (
          <WidgetInspector
            key={selectedWidget.id}
            widget={selectedWidget}
            sensors={sensors}
            onUpdate={(updates, coalesce) => dispatch({ type: 'UPDATE_WIDGET', id: selectedWidget.id, updates, coalesce })}
            onUpdateConfig={(config, coalesce) => dispatch({ type: 'UPDATE_WIDGET_CONFIG', id: selectedWidget.id, config, coalesce })}
            onDuplicate={() => pasteWidget(selectedWidget)}
            onReorder={to => dispatch({ type: 'REORDER_WIDGET', id: selectedWidget.id, to })}
            onDelete={deleteSelection}
          />
        )}
        {editMode && !pendingSensor && !selectedWidget && !selectedConn && (
          <ScreenInspector
            widgets={layout.widgets.length}
            connections={layout.connections.length}
            tags={new Set(layout.widgets.flatMap(w => w.sensorIds ?? [])).size}
          />
        )}
        {editMode && !pendingSensor && selectedConn && (
          <ConnectionInspector
            connection={selectedConn}
            widgets={layout.widgets}
            onUpdate={updates => dispatch({ type: 'UPDATE_CONNECTION', id: selectedConn.id, updates })}
            onDelete={deleteSelection}
          />
        )}
      </div>
      {!editMode && <AlarmBanner />}
    </div>
  )
}

function IconBtn({ title, onClick, active, disabled, children }: {
  title: string; onClick: () => void; active?: boolean; disabled?: boolean; children: React.ReactNode
}) {
  return (
    <button onClick={onClick} title={title} aria-label={title} disabled={disabled} style={{
      width: 26, height: 24, display: 'inline-grid', placeItems: 'center', borderRadius: 5, padding: 0,
      border: `1px solid ${active ? 'var(--cyan)' : 'var(--border)'}`,
      background: active ? 'color-mix(in srgb, var(--cyan) 14%, transparent)' : 'transparent',
      color: active ? 'var(--cyan)' : 'var(--fg)', opacity: disabled ? 0.35 : 1, cursor: disabled ? 'default' : 'pointer',
    }}>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        {children}
      </svg>
    </button>
  )
}

// ── Right sidebar: pick a visualization for a dropped tag ──

const WidgetTypeSidebar: React.FC<{
  sensorId: number
  sensors: SensorConfig[]
  onSelect: (type: ScadaWidgetType) => void
  onSelectSymbol: (key: string) => void
  onClose: () => void
}> = ({ sensorId, sensors, onSelect, onSelectSymbol, onClose }) => {
  const sensor = sensors.find(s => s.id === sensorId)
  const isAnalog = sensor ? sensor.max_val !== sensor.min_val : true
  // Equipment is offered as library symbols (operator writes opt-in), not as the legacy writable widget types.
  const types = getCompatibleWidgetTypes(isAnalog).filter(t => !EQUIPMENT_TYPES.has(t))
  const symbols = SYMBOLS.filter(s => s.binding === 'state' || (isAnalog && s.binding !== 'none'))

  const sampleSensor: SensorConfig = sensor ?? {
    id: sensorId, key: 'sample', name: 'Sample', unit: '', min_val: 0, max_val: 100, warn_val: 75, crit_val: 90,
  } as SensorConfig
  const sampleData: SensorData = {
    id: sensorId, key: sampleSensor.key,
    value: ((sampleSensor.max_val ?? 100) - (sampleSensor.min_val ?? 0)) * 0.6 + (sampleSensor.min_val ?? 0),
    warn: sampleSensor.warn_val ?? 75, crit: sampleSensor.crit_val ?? 90, source_type: '', status: '',
  }
  const sampleValues: Record<string, SensorData> = { [String(sensorId)]: sampleData }

  return (
    <div style={{
      position: 'absolute', top: 0, right: 0, bottom: 0, width: INSPECTOR_WIDTH, zIndex: 20,
      background: 'var(--card)', borderLeft: '1px solid var(--border)', display: 'flex', flexDirection: 'column',
    }}>
      <div style={{ padding: '10px 12px', borderBottom: '1px solid var(--border)', display: 'flex', justifyContent: 'space-between', alignItems: 'center', flexShrink: 0 }}>
        <div>
          <div style={{ fontSize: 13, fontWeight: 600 }}>Choose visualization</div>
          <div style={{ fontSize: 11, color: 'var(--muted)', marginTop: 2 }}>{sensor?.name ?? `Tag #${sensorId}`}</div>
        </div>
        <button onClick={onClose} aria-label="Close" style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', fontSize: 18, lineHeight: 1 }}>×</button>
      </div>
      <div style={{ flex: 1, overflowY: 'auto', padding: 8 }}>
        <div style={{ fontSize: 10, fontWeight: 600, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.06em', margin: '2px 2px 6px' }}>Equipment</div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 4, marginBottom: 12 }}>
          {symbols.map(s => (
            <button key={s.key} onClick={() => onSelectSymbol(s.key)} title={s.name} style={{
              display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 3, padding: '6px 2px', borderRadius: 6, cursor: 'pointer',
              border: '1px solid var(--border)', background: 'var(--hmi-stage)', color: 'var(--text-dim)', fontSize: 9, lineHeight: 1.1,
            }}>
              <SymbolThumb def={s} size={34} />
              <span>{s.name}</span>
            </button>
          ))}
        </div>
        <div style={{ fontSize: 10, fontWeight: 600, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.06em', margin: '2px 2px 6px' }}>Widgets</div>
        {types.map(type => {
          const m = WIDGET_MANIFESTS[type]
          const Comp = getWidgetComponent(type)
          return (
            <div key={type} onClick={() => onSelect(type)}
              style={{
                display: 'flex', alignItems: 'center', gap: 10, padding: 8, borderRadius: 6, marginBottom: 4, cursor: 'pointer',
                border: '1px solid var(--border)',
              }}
              onMouseEnter={e => { e.currentTarget.style.borderColor = 'var(--cyan)' }}
              onMouseLeave={e => { e.currentTarget.style.borderColor = 'var(--border)' }}
            >
              <div style={{ width: 72, height: 52, flexShrink: 0, borderRadius: 4, overflow: 'hidden', background: 'var(--hmi-stage)', border: '1px solid var(--border)', pointerEvents: 'none' }}>
                {Comp && (
                  <Comp
                    instance={{ id: '__preview__', type, sensorId, sensorIds: [sensorId], x: 0, y: 0, width: 72, height: 52, config: {} }}
                    sensor={sampleSensor} data={sampleData} sensors={[sampleSensor]} currentValues={sampleValues}
                    editMode={false} selected={false}
                  />
                )}
              </div>
              <div>
                <div style={{ fontSize: 12, fontWeight: 500 }}>{m.displayName}</div>
                <div style={{ fontSize: 10, color: 'var(--muted)' }}>{m.compatibleDataTypes.join(' · ')}</div>
              </div>
            </div>
          )
        })}
      </div>
    </div>
  )
}
