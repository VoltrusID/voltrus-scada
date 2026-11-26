import React, { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { SensorConfig } from '../types'
import type { ConnectionInstance, ScadaWidgetType, WidgetInstance } from '../scada2/widgets/types'
import { WIDGET_MANIFESTS } from '../scada2/widgets/registry'
import { SYMBOL_MAP } from '../scada2/hmi/symbols'
import { loadLayoutFromAPI, saveLayoutToAPI } from '../scada2/engine/layout'
import { ScadaView } from '../scada2/ScadaView'
import { widgetTitle } from '../scada2/ScadaInspector'
import { STAGE_H, STAGE_W } from '../scada2/ScadaCanvas'
import { useQueryClient } from '@tanstack/react-query'
import { useBlocker } from '@tanstack/react-router'
import { useScreens, useSensors } from '../queries'
import {
  fetchScadaLayout, saveScadaLayout, useFaceplates, useScadaLayout, type ChangeSince, type LayoutBase, type ScadaLayout,
} from '../queries/scada'
import {
  invalidateDesign, revertScreen, useBlame, useDesignStatus, useScreenState, useVersions, type DesignChange, type RevertBody,
} from '../queries/designHistory'
import { useAuthMe, useUsers } from '../queries/users'
import { applyCommentEvent, commentKeys, deleteComment, mentionsIn, patchComment, postComment, useComments, useDesignSessions, type DesignComment } from '../queries/designComments'
import { useCollab } from './collab/useCollab'
import { CommentBox, CommentPins, CommentsPane, PeersOverlay, PresenceBar, ScreenEditors } from './collab/ui'
import { useRole } from '../RoleContext'
import { canConfigure as roleCanConfigure } from '../roles'
import { useModbusDevices } from '../queries/modbus'
import {
  updateInstance, useEquipmentInstances, useEquipmentTypes, useInstallLibrary, useInvalidateEquipment,
  type EquipmentInstance, type EquipmentInstanceInput,
} from '../queries/equipment'
import { queryKeys } from '../queries/keys'
import { useTrackedCurrentValues } from '../store/dashboardStore'
import { openTrend } from '../trend/TrendPopup'
import { toast } from '../components/Toast'
import { isSignatureCancelled } from '../signature/logic'
import { ThemeToggle } from '../components/ThemeToggle'
import {
  alignPatches, bindPrimaryPatch, boundsOf, distributePatches, duplicate, emptyHistory, expandToGroups, groupOf, groupPatches,
  historyReducer, isDirty, isLocked, lockPatches, matchSizePatches, movePatches, newId, ungroupPatches,
  type AlignMode, type Edit, type Patch,
} from './model'
import type { Guide } from './snap'
import { DesignerCanvas, type CanvasDrop } from './DesignerCanvas'
import { PalettePane, ProjectBrowser, TagBrowserPane } from './panes'
import { EquipmentTree, EquipmentTypesSection, InstancePicker, InstanceSection } from './equipment'
import { bindInstancePatch, configOf, instanceOf, widgetsByInstance } from './equipmentModel'
import { PropertyEditor } from './PropertyEditor'
import { ChoiceDialog, D, Dialog, DialogBtn, Icon, Pane, Sep, SmallBtn, Tabs, ToolBtn, input, mono } from './ui'
import { BlameInfo, HistoryList, type HistoryObject } from './history'
import { MobilePane } from './MobilePane'
import { mobileLabel } from '../scada2/mobile/OperatorView'
import { PublishDialog, VersionsList } from './versions'
import { CompareView } from './diffView'
import { StageView } from './StageView'
import {
  kindSummary, objectLabel, publishState, relativeTime, revertObjectRequest, revertScreenRequest, sourceLabel, withAlsoRestore, type Batch,
} from './historyModel'

interface Props {
  screen: string
  /** Navigate to another screen in the Designer. Unsaved changes are guarded by the router blocker here. */
  onOpenScreen: (id: string) => void
  onClose: () => void
  onOpenPage: (page: 'scripts' | 'alarms' | 'tags') => void
}

/** A save the server refused because the screen changed since it was loaded: who changed what since. */
interface Conflict { revision: string; seq: number | null; message: string; changesSince: ChangeSince[]; then?: () => void }

/** A revert waiting for the user: confirm it, or restore the widgets a connection needs too. */
interface RevertAsk { body: RevertBody; title: string; message: React.ReactNode }
interface Dangling { body: RevertBody; connection: string; missing: string[] }

type RightTab = 'properties' | 'history' | 'versions' | 'comments' | 'mobile'

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

function usePersisted<T>(key: string, initial: T): [T, (v: T | ((p: T) => T)) => void] {
  const [v, setV] = useState<T>(() => {
    try { const s = localStorage.getItem(key); return s ? JSON.parse(s) as T : initial } catch { return initial }
  })
  useEffect(() => { try { localStorage.setItem(key, JSON.stringify(v)) } catch { /* private mode */ } }, [key, v])
  return [v, setV]
}

export function Designer({ screen: requested, onOpenScreen, onClose, onOpenPage }: Props) {
  const { t } = useTranslation()
  const { data: screens = [] } = useScreens(true)
  const { data: sensors = [] } = useSensors(true)
  const screen = requested === 'all' && screens.length ? screens[0].id : requested
  const layoutQ = useScadaLayout(screen, true)
  const qc = useQueryClient()
  const { data: faceplates = [] } = useFaceplates()
  const { data: eqTypes = [] } = useEquipmentTypes()
  const { data: eqInstances = [] } = useEquipmentInstances()
  const { data: modbusDevices = [] } = useModbusDevices()
  const installLibrary = useInstallLibrary()
  const invalidateEquipment = useInvalidateEquipment()
  const role = useRole()
  const { data: me } = useAuthMe()
  const canConfigure = role.canConfigure || roleCanConfigure(me?.role)
  const isAdmin = role.isAdmin || me?.role === 'admin'
  const statusQ = useDesignStatus(screen)
  const status = statusQ.data
  const publishing = !!status?.publishing
  const { data: versions = [] } = useVersions(screen)
  /** An equipment type dropped on the stage: the instance picker is open for it. */
  const [picker, setPicker] = useState<{ type: string; x?: number; y?: number } | null>(null)

  const [h, dispatch] = useReducer(historyReducer, emptyHistory)

  // ── Live collaboration (phase 4): the draft is a shared document while the server offers it ──
  /** The server said collaboration is unavailable for this screen (off, refused, other protocol). */
  const [collabFallback, setCollabFallback] = useState<string | null>(null)
  useEffect(() => { setCollabFallback(null) }, [screen])
  /** The server has phase 4 (comments, sessions), whether or not collaboration is switched on. */
  const hasPhase4 = status?.collaboration != null
  const wantCollab = !!status?.collaboration?.enabled && !collabFallback && canConfigure
  const nameOf = useCallback((w: WidgetInstance) => sensors.find(s => s.id === w.sensorId)?.name || widgetTitle(w), [sensors])
  const lastRejoinToast = useRef(0)
  const collab = useCollab(screen, wantCollab, nameOf, {
    onFlushed: m => {
      baseSeq.current = m.seq
      // Only an acknowledgement: nothing was written.
      if (!m.users.length) return
      void invalidateDesign(qc, screen)
      void qc.invalidateQueries({ queryKey: queryKeys.scadaLayout(screen) })
    },
    onRejected: m => toast.error(`Not saved: ${m.objects.map(o => `${o.object_id} (${o.reason})`).join('; ')}. ${m.objects.length === 1 ? 'It was' : 'They were'} put back.`),
    onComment: m => applyCommentEvent(qc, screen, m.event, m.comment as unknown as DesignComment),
    onError: e => toast.error(`Live editing: ${e.message}`),
    onRejoin: () => {
      const now = Date.now()
      if (now - lastRejoinToast.current < 30_000) return
      lastRejoinToast.current = now
      toast.info('Live editing: reconnecting to the session; your edits are kept')
    },
    onReplayed: ({ objects }) => toast.info(`The live session restarted; your unsaved changes to ${objects} object${objects === 1 ? '' : 's'} were applied again`),
    onFallback: e => {
      setCollabFallback(e.code)
      if (e.code === 'protocol') toast.error(`${e.message}`)
      else if (e.code !== 'disabled') toast.info(`Live editing unavailable (${e.message}); saving whole screens`)
    },
  })
  const live = collab.active
  const liveRef = useRef(live)
  liveRef.current = live
  const collabRef = useRef(collab)
  collabRef.current = collab
  const doc = live ? collab.doc : h.present
  const dirty = live ? false : isDirty(h)
  const [selection, setSelectionRaw] = useState<string[]>([])
  const [selConn, setSelConn] = useState<string | null>(null)
  const [tool, setTool] = useState<'select' | 'connect' | 'comment'>('select')
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
  /** Revision of the server layout the document is based on (sent with a save). */
  const baseRevision = useRef<string | null>(null)
  /** Change-log head of the server layout the document is based on (sent with a save). */
  const baseSeq = useRef<number | null>(null)
  const dirtyRef = useRef(dirty)
  dirtyRef.current = dirty
  const [conflict, setConflict] = useState<Conflict | null>(null)
  /** The screen was saved elsewhere while this one has unsaved changes. */
  const [remoteChanged, setRemoteChanged] = useState(false)
  const [rightTab, setRightTab] = useState<RightTab>('properties')
  /** History of the whole screen rather than of the selected component. */
  const [wholeScreen, setWholeScreen] = useState(false)
  /** A past state shown read-only on the canvas (History pane): editing is off meanwhile. */
  const [previewAt, setPreviewAt] = useState<Batch | null>(null)
  const readOnlyRef = useRef(false)
  readOnlyRef.current = previewAt != null
  const [compare, setCompare] = useState<{ from: string; to: string } | null>(null)
  const [publishOpen, setPublishOpen] = useState<{ version?: { id: string; label: string } } | null>(null)
  const [revertAsk, setRevertAsk] = useState<RevertAsk | null>(null)
  const [dangling, setDangling] = useState<Dangling | null>(null)
  /** Something that works on the saved screen asked for unsaved edits to be saved or dropped first. */
  const [needSave, setNeedSave] = useState<{ what: string; then: () => void } | null>(null)

  const setSelection = useCallback((ids: string[]) => { setSelectionRaw(ids); if (ids.length) setSelConn(null) }, [])

  // ── Presence, follow mode and comments ──

  const hasPhase4Ref = useRef(hasPhase4)
  hasPhase4Ref.current = hasPhase4
  /** The awareness client id of the person whose view this one follows. */
  const [following, setFollowing] = useState<number | null>(null)
  const [viewTo, setViewTo] = useState<{ cx: number; cy: number; scale: number; n: number } | null>(null)
  const followed = following != null ? collab.peers.find(p => p.clientId === following) : undefined
  const followedView = followed?.viewport
  useEffect(() => {
    if (following != null && !followed && collab.status === 'live') setFollowing(null)
  }, [following, followed, collab.status])
  useEffect(() => {
    if (!followedView) return
    setViewTo(v => ({ cx: followedView.cx, cy: followedView.cy, scale: followedView.scale, n: (v?.n ?? 0) + 1 }))
  }, [followedView?.cx, followedView?.cy, followedView?.scale]) // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { if (live) collab.setPresence({ selection }) }, [live, selection]) // eslint-disable-line react-hooks/exhaustive-deps
  const onCursor = useCallback((pt: { x: number; y: number } | null) => {
    setCursor(pt)
    if (liveRef.current) collabRef.current.setPresence({ cursor: pt })
  }, [])
  const onViewport = useCallback((v: { cx: number; cy: number; scale: number }) => {
    if (liveRef.current) collabRef.current.setPresence({ viewport: v })
  }, [])

  const commentsQ = useComments(screen, hasPhase4)
  const comments = commentsQ.data ?? []
  const sessionsQ = useDesignSessions(hasPhase4 && !live)
  const sessions = collab.sessions ?? sessionsQ.data ?? null
  const { data: userList = [] } = useUsers(isAdmin && hasPhase4)
  const mentionUsers = useMemo(() => [...new Set([
    ...userList.map(u => u.username), ...comments.map(c => c.author), ...collab.peers.map(p => p.user),
    ...(sessions ?? []).flatMap(x => x.users.map(u => u.user)),
  ].filter(Boolean))].sort(), [userList, comments, collab.peers, sessions])
  const [activeComment, setActiveComment] = useState<number | null>(null)
  const [showResolvedPins, setShowResolvedPins] = useState(false)
  /** A new comment being written, pinned at this point (and component). */
  const [newComment, setNewComment] = useState<{ x: number; y: number; object_id: string | null; object_type: string | null } | null>(null)
  const refetchComments = useCallback(() => { void qc.invalidateQueries({ queryKey: commentKeys.screen(screen) }) }, [qc, screen])
  // Without a live session nothing pushes comment events: the tab refetches when opened.
  useEffect(() => { if (rightTab === 'comments' && !live) refetchComments() }, [rightTab, live, refetchComments])
  // Edits carry their time: the same kind of edit after a pause is a new undo step.
  // While a past state is previewed nothing edits the draft.
  const edit = useCallback((e: Edit, coalesce?: string) => {
    if (readOnlyRef.current) return
    if (liveRef.current) collabRef.current.edit(e, coalesce, Date.now())
    else dispatch({ type: 'edit', edit: e, coalesce, at: Date.now() })
  }, [])
  const patch = useCallback((patches: Record<string, Patch>, coalesce?: string) => {
    if (readOnlyRef.current) return
    if (!Object.keys(patches).length) return
    if (liveRef.current) collabRef.current.edit({ type: 'patch', patches }, coalesce, Date.now())
    else dispatch({ type: 'edit', edit: { type: 'patch', patches }, coalesce, at: Date.now() })
  }, [])
  const undo = useCallback(() => { if (liveRef.current) collabRef.current.undo(); else dispatch({ type: 'undo' }) }, [])
  const redo = useCallback(() => { if (liveRef.current) collabRef.current.redo(); else dispatch({ type: 'redo' }) }, [])
  const undoDepth = live ? collab.undoDepth : h.past.length
  const redoDepth = live ? collab.redoDepth : h.future.length

  // Drop selected ids that no longer exist (after undo, delete, load).
  useEffect(() => {
    const ids = new Set(doc.widgets.map(w => w.id))
    if (selection.some(id => !ids.has(id))) setSelectionRaw(s => s.filter(id => ids.has(id)))
    if (selConn && !doc.connections.some(c => c.id === selConn)) setSelConn(null)
  }, [doc, selection, selConn])

  // ── Load and save ──

  const loaded = layoutQ.isFetched && !layoutQ.isFetching

  const loadLayout = useCallback((data: ScadaLayout, id: string) => {
    loadedScreen.current = id
    baseRevision.current = data.revision
    baseSeq.current = typeof data.seq === 'number' ? data.seq : null
    setRemoteChanged(false)
    dispatch({ type: 'load', doc: loadLayoutFromAPI(data.equipment ?? [], data.connections ?? []) })
    setSelectionRaw([])
    setSelConn(null)
  }, [])

  useEffect(() => {
    const data = layoutQ.data
    // In a live session the shared document is the draft.
    if (live) { loadedScreen.current = screen; return }
    if (!loaded || !data || savingRef.current) return
    const same = loadedScreen.current === screen
    if (same && data.revision === baseRevision.current) return
    // Changed on the server. Unsaved work is kept; the user decides (banner, or the conflict on save).
    if (same && dirtyRef.current) { setRemoteChanged(true); return }
    loadLayout(data, screen)
  }, [screen, loaded, layoutQ.data, loadLayout, live])

  const invalidate = useCallback((id: string) => {
    void qc.invalidateQueries({ queryKey: queryKeys.scadaLayout(id) })
    void qc.invalidateQueries({ queryKey: queryKeys.scadaEquipment(id) })
    void qc.invalidateQueries({ queryKey: queryKeys.scadaConnections(id) })
    void invalidateDesign(qc, id)
  }, [qc])

  /**
   * Save equipment and connections together, based on the revision and seq
   * loaded. `base` overrides them (overwrite after a conflict: the head the
   * user saw in the conflict). Nothing to save is a no-op. `then` runs after a
   * save that first had to resolve a conflict.
   */
  const save = useCallback(async (opts: { base?: LayoutBase; then?: () => void } = {}): Promise<boolean> => {
    if (liveRef.current) { collabRef.current.flush(); return true }
    const id = loadedScreen.current
    if (!id || savingRef.current) return false
    if (!dirtyRef.current) return true
    savingRef.current = true
    setSaving(true)
    const snapshot = doc
    try {
      const rows = saveLayoutToAPI(snapshot, w => sensors.find(s => s.id === w.sensorId)?.name || widgetTitle(w))
      const r = await saveScadaLayout(id, rows.equipment, rows.connections, opts.base ?? { revision: baseRevision.current, seq: baseSeq.current })
      if (!r.ok) {
        setConflict({ revision: r.revision, seq: r.seq, message: r.message, changesSince: r.changesSince, then: opts.then })
        return false
      }
      baseRevision.current = r.revision
      if (r.seq != null) baseSeq.current = r.seq
      setRemoteChanged(false)
      dispatch({ type: 'saved', doc: snapshot })
      invalidate(id)
      toast.success('Screen saved')
      return true
    } catch (err: any) {
      toast.error(`Screen not saved: ${err?.message || 'request failed'}`)
      return false
    } finally {
      savingRef.current = false
      setSaving(false)
    }
  }, [doc, sensors, invalidate])

  const reloadFromServer = useCallback(async () => {
    const id = loadedScreen.current ?? screen
    const data = await qc.fetchQuery({ queryKey: queryKeys.scadaLayout(id), queryFn: () => fetchScadaLayout(id), staleTime: 0 })
    loadLayout(data, id)
  }, [qc, screen, loadLayout])

  // ── Version history ──

  useEffect(() => { setPreviewAt(null); setCompare(null) }, [screen])

  /** Run `then` on the saved screen: unsaved edits are saved or dropped first (the user picks). */
  const withSaved = useCallback((what: string, then: () => void) => {
    if (dirtyRef.current) setNeedSave({ what, then })
    else then()
  }, [])

  /** Send a revert; a dangling connection asks to restore its widgets too. Reverts are new changes, so the draft reloads. */
  const runRevert = useCallback(async (body: RevertBody) => {
    const id = loadedScreen.current ?? screen
    try {
      // Live: the server writes the session's edits first, under the screen lock, so no base is sent.
      const r = await revertScreen(id, { ...body, base_seq: liveRef.current ? null : baseSeq.current })
      if (r.ok) {
        setPreviewAt(null)
        await reloadFromServer()
        invalidate(id)
        const extra = [
          r.removed_connections.length ? `removed ${r.removed_connections.length} connection${r.removed_connections.length === 1 ? '' : 's'} that ended on it` : '',
          r.ungrouped.length ? `${r.ungrouped.join(', ')} restored ungrouped` : '',
          r.missing_tags.length ? `bound to ${r.missing_tags.length} tag${r.missing_tags.length === 1 ? '' : 's'} that no longer exist${r.missing_tags.length === 1 ? 's' : ''}` : '',
        ].filter(Boolean).join('; ')
        toast.success(`Reverted${extra ? `: ${extra}` : ''}`)
        return
      }
      if (r.code === 'dangling_connection') { setDangling({ body, connection: r.connection, missing: r.missing }); return }
      if (r.code === 'conflict') {
        await reloadFromServer()
        invalidate(id)
        toast.error('The screen changed since you loaded it. It was reloaded; check it and revert again.')
        return
      }
      toast.error(`Not reverted: ${r.message}`)
    } catch (e: any) {
      if (!isSignatureCancelled(e)) toast.error(`Not reverted: ${e?.message || 'request failed'}`)
    }
  }, [screen, reloadFromServer, invalidate])

  const askRevert = useCallback((a: RevertAsk) => {
    withSaved('A revert writes new changes to the saved screen.', () => setRevertAsk(a))
  }, [withSaved])

  /**
   * One object as it was after the whole save it changed in (the save's last
   * seq, not the row's: a connection and the widgets it ends on, added in one
   * save, exist together only after all of it).
   */
  const revertObject = useCallback((c: DesignChange, b: Batch) => {
    const name = objectLabel(c.after ?? c.before, c.object_id)
    askRevert({
      body: revertObjectRequest(c.object_id, c.object_type, b.maxSeq),
      title: `Revert ${name}?`,
      message: <>Put <b style={{ color: 'var(--fg)' }}>{name}</b> back as it was after change #{b.maxSeq}{c.after ? '' : ' (when it did not exist: it is removed, with the connections that end on it)'}. Nothing else on the screen changes; history is kept.</>,
    })
  }, [askRevert])

  const revertWholeScreen = useCallback((to: number | string, label: string) => {
    askRevert({
      body: revertScreenRequest(to),
      title: 'Revert the whole screen?',
      message: <>Put every component and connection back as {label}. It is saved as new changes on the draft; history is kept and operators keep seeing what is published.</>,
    })
  }, [askRevert])

  const historyObject: HistoryObject | null = useMemo(() => {
    if (selConn) return { id: selConn, type: 'connection', label: selConn }
    if (selection.length !== 1) return null
    const w = doc.widgets.find(x => x.id === selection[0])
    return w ? { id: w.id, type: 'widget', label: String(w.config?.label || w.name || widgetTitle(w)) } : null
  }, [selection, selConn, doc.widgets])
  const blameQ = useBlame(screen, rightTab === 'properties' && historyObject != null)
  const blame = historyObject ? blameQ.data?.find(b => b.object_id === historyObject.id && b.object_type === historyObject.type) : undefined
  const pubState = publishState(status)

  // Leaving with unsaved changes (Close, another screen, Back, a link in the
  // Project pane, an app shortcut) asks first: Save, Discard or Cancel.
  // Reload and closing the tab get the browser's own prompt.
  const blocker = useBlocker({
    shouldBlockFn: ({ current, next }) => {
      if (!dirtyRef.current) return false
      const screenOf = (l: { search: unknown }) => (l.search as { screen?: string } | undefined)?.screen ?? 'all'
      return !(next.pathname === current.pathname && screenOf(next) === screenOf(current))
    },
    enableBeforeUnload: () => dirtyRef.current || (liveRef.current && collabRef.current.hasUnsaved()),
    withResolver: true,
  })

  const openScreen = useCallback((id: string) => { if (id !== screen) onOpenScreen(id) }, [screen, onOpenScreen])
  const close = useCallback(() => { onClose() }, [onClose])

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

  /** Dropping an equipment type asks which instance to place (or creates one). */
  const addEquipment = useCallback((key: string, x?: number, y?: number) => {
    if (!eqTypes.some(t => t.id === key)) { toast.error(`No equipment type '${key}'`); return }
    setPicker({ type: key, x, y })
  }, [eqTypes])

  /** Place a symbol bound to an instance, exactly as the server binds it. */
  const placeInstance = useCallback((inst: EquipmentInstance) => {
    const at = picker
    setPicker(null)
    invalidateEquipment()
    const symbol = (inst.resolved.widget.config.symbol as string | undefined) ?? inst.resolved.symbol
    const def = SYMBOL_MAP[symbol]
    const [sw, sh] = def?.size ?? [90, 100]
    const w = make('symbol', symbol, at?.x != null ? at.x - sw / 2 : undefined, at?.y != null ? at.y - sh / 2 : undefined)
    if (!w) { toast.error(`Unknown symbol '${symbol}'`); return }
    const p = bindInstancePatch(null, inst.name, inst.resolved.widget)
    const config = { ...w.config, ...configOf(p) }
    // An instrument bubble takes its function letters and loop number from the instance name (PT-501: PT, 501).
    const tag = /^([A-Z]{1,5})-?(\d+[A-Z]?)$/.exec(inst.name)
    if (tag && def?.fields?.some(f => f.key === 'tagFunction')) Object.assign(config, { tagFunction: tag[1], loopNumber: tag[2] })
    add({ ...w, type: 'symbol', sensorId: p.sensorId ?? null, sensorIds: p.sensorIds ?? [], config })
  }, [picker, make, add, invalidateEquipment])

  /**
   * Change an instance from the property editor. The server re-binds every
   * symbol of it on every screen (this one included), so unsaved work is saved
   * first and the screen reloads from the server afterwards.
   */
  const applyInstance = useCallback(async (name: string, body: EquipmentInstanceInput) => {
    if (dirtyRef.current && !(await save())) throw new Error('Save the screen first')
    await updateInstance(name, body, false)
    invalidateEquipment()
    const id = loadedScreen.current
    if (id) {
      const fresh = await fetchScadaLayout(id)
      if (!dirtyRef.current && !liveRef.current) loadLayout(fresh, id)
      void qc.invalidateQueries({ queryKey: queryKeys.scadaLayout(id) })
    }
    toast.success(`${name} changed; its symbols follow`)
  }, [save, invalidateEquipment, loadLayout, qc])

  const onScreenInstances = useMemo(() => widgetsByInstance(doc.widgets), [doc.widgets])

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
    setFollowing(null)
    const cur = zoom === 'fit' ? scale : zoom
    const next = dir > 0 ? ZOOM_STEPS.find(z => z > cur + 0.001) : [...ZOOM_STEPS].reverse().find(z => z < cur - 0.001)
    if (next) { setZoom(next); setScale(next) }
  }, [zoom, scale])

  // ── Keyboard ──

  const keys = useRef<Record<string, () => void>>({})
  keys.current = {
    save: () => { if (dirtyRef.current || liveRef.current) void save() }, undo, redo,
    copy, paste, cut: () => { copy(); removeSelection() }, duplicate: duplicateSel, remove: removeSelection,
    all: () => setSelection(expandToGroups(doc, doc.widgets.filter(w => !isLocked(w)).map(w => w.id))),
    group, ungroup, lock: toggleLock, front: () => order('front'), back: () => order('back'), forward: () => order('forward'), backward: () => order('backward'),
    preview: () => { if (!readOnlyRef.current) setPreview(p => !p) }, fit: () => setZoom('fit'), zoomIn: () => zoomBy(1), zoomOut: () => zoomBy(-1),
    escape: () => { setSelection([]); setSelConn(null); setConnectFrom(null); setTool('select'); setDropMenu(null); setFollowing(null) },
  }
  const nudge = useRef<(dx: number, dy: number) => void>(() => {})
  nudge.current = (dx, dy) => patch(movePatches(doc, selection, dx, dy, { w: STAGE_W, h: STAGE_H }), `nudge:${selection.join(',')}`)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isTyping(e.target)) return
      const k = keys.current
      // Viewing a past state: nothing edits; Escape goes back to the draft.
      if (readOnlyRef.current) { if (e.key === 'Escape') setPreviewAt(null); return }
      const mod = e.ctrlKey || e.metaKey
      const key = e.key.toLowerCase()
      // F5 toggles preview (Ctrl+P stays the browser's print).
      if (e.key === 'F5') { e.preventDefault(); k.preview(); return }
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
      if (key === 'm' && hasPhase4Ref.current) { setTool(t => (t === 'comment' ? 'select' : 'comment')); return }
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
        {pubState && (
          <>
            <Sep />
            <button type="button" data-testid="designer-publish-state" data-tone={pubState.tone} title={`${pubState.title}. Open the versions.`}
              onClick={() => setRightTab('versions')} style={{
                display: 'inline-flex', alignItems: 'center', gap: 6, height: 22, padding: '0 8px', borderRadius: 11, cursor: 'pointer', fontSize: D.small,
                border: `1px solid color-mix(in srgb, ${TONES[pubState.tone]} 45%, transparent)`, background: `color-mix(in srgb, ${TONES[pubState.tone]} 10%, transparent)`, color: TONES[pubState.tone],
              }}>
              <span style={{ width: 6, height: 6, borderRadius: 3, background: TONES[pubState.tone] }} />{pubState.text}
            </button>
            {canConfigure && (
              <button type="button" data-testid="designer-publish" onClick={() => setPublishOpen({})} disabled={!!previewAt}
                title={status?.approval === 'required' ? 'Ask an administrator to publish the draft to operators' : 'Publish the draft: operators see it from now on'} style={{
                  display: 'inline-flex', alignItems: 'center', gap: 5, height: 24, padding: '0 10px', borderRadius: 4, cursor: previewAt ? 'default' : 'pointer', fontSize: D.small, fontWeight: 600,
                  border: `1px solid ${D.accent}`, background: D.accentBg, color: D.accent, opacity: previewAt ? 0.5 : 1,
                }}>
                <Icon name="publish" size={12} />{status?.approval === 'required' ? 'Request publish' : 'Publish'}
              </button>
            )}
          </>
        )}
        <div style={{ flex: 1 }} />
        {wantCollab && (
          <PresenceBar status={live ? collab.status : 'off'} peers={collab.peers} me={collab.me} pending={collab.pending}
            following={following} onFollow={id => { setFollowing(id); if (id == null) setViewTo(null) }} />
        )}
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
        <fieldset disabled={!!previewAt} style={{ border: 'none', margin: 0, padding: 0, minWidth: 0, opacity: previewAt ? 0.55 : 1 }}>
        <div role="toolbar" aria-label="Designer tools" style={{ height: 32, flexShrink: 0, display: 'flex', alignItems: 'center', gap: 2, padding: '0 6px', borderBottom: `1px solid ${D.line}`, background: D.chrome, overflowX: 'auto' }}>
          {live ? (
            <ToolBtn icon="save" title="Edits are saved to the draft as you work (Ctrl+S: now)" label={collab.pending ? (collab.status === 'live' ? 'Saving…' : 'Waiting') : 'Saved'}
              onClick={() => { void save() }} disabled={collab.status !== 'live'} testId="designer-save" />
          ) : (
            <ToolBtn icon="save" title="Save (Ctrl+S)" label={saving ? 'Saving…' : dirty ? 'Save' : 'Saved'} onClick={() => { void save() }} disabled={!dirty || saving} active={dirty} testId="designer-save" />
          )}
          <ToolBtn icon="undo" title={`Undo (Ctrl+Z) · ${undoDepth} step${undoDepth === 1 ? '' : 's'}${live ? ' of yours' : ''}`} onClick={undo} disabled={!undoDepth} testId="designer-undo" />
          <ToolBtn icon="redo" title="Redo (Ctrl+Y)" onClick={redo} disabled={!redoDepth} testId="designer-redo" />
          <Sep />
          <ToolBtn icon="select" title="Select (V)" onClick={() => { setTool('select'); setConnectFrom(null) }} active={tool === 'select'} />
          <ToolBtn icon="connect" title="Connect (C): click source, then target" onClick={() => { setTool('connect'); setSelection([]) }} active={tool === 'connect'} />
          {hasPhase4 && <ToolBtn icon="edit" title="Comment (M): click a component or the stage" onClick={() => setTool(t => (t === 'comment' ? 'select' : 'comment'))} active={tool === 'comment'} testId="designer-comment-tool" />}
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
        </fieldset>
      )}

      {/* Body */}
      <div style={{ flex: 1, minHeight: 0, display: 'flex' }}>
        {!preview && (
          <aside style={{ width: 264, flexShrink: 0, display: 'flex', flexDirection: 'column', borderRight: `1px solid ${D.line}`, background: D.pane, ...(previewAt ? { pointerEvents: 'none', opacity: 0.55 } : {}) }}
            aria-disabled={previewAt ? true : undefined}>
            <Pane title="Project" icon="folder" grow={1} testId="designer-project">
              <ProjectBrowser screens={screens} active={screen} onOpenScreen={id => { void openScreen(id) }} doc={doc} selection={selection} onSelect={setSelection}
                onToggleLock={id => patch(lockPatches(doc, [id], !isLocked(doc.widgets.find(w => w.id === id)!)))}
                onToggleVisible={id => { const w = doc.widgets.find(x => x.id === id)!; patch({ [id]: { config: { visible: w.config?.visible === false ? undefined : false } } }) }}
                faceplates={faceplates} onOpenPage={onOpenPage}
                editors={hasPhase4 ? id => <ScreenEditors screen={id} sessions={sessions} me={collab.me.user || me?.username || ''} /> : undefined}
                equipment={<EquipmentTree types={eqTypes} instances={eqInstances} onScreen={onScreenInstances}
                  selected={selection.length ? instanceOf(doc.widgets.find(w => w.id === selection[0]) ?? { config: {} }) : null}
                  onSelectInstance={name => { const ids = onScreenInstances.get(name); if (ids?.length) setSelection(ids) }} />} />
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
          ) : previewAt ? (
            <PastState screen={screen} batch={previewAt} sensors={sensors} canRevert={canConfigure}
              onBack={() => setPreviewAt(null)} onRevert={() => revertWholeScreen(previewAt.maxSeq, `it was after change #${previewAt.maxSeq}`)} />
          ) : (live ? !collab.synced : !loaded && loadedScreen.current !== screen) ? (
            <div style={{ flex: 1, display: 'grid', placeItems: 'center', color: 'var(--text-muted)' }}>Loading {screenName}…</div>
          ) : (
            <DesignerCanvas
              doc={doc} sensors={sensors} selection={selection} scale={scale} zoom={zoom}
              onScaleChange={setScale} onZoomTo={s => { setZoom(s); setScale(s) }}
              showGrid={showGrid} snap={snap} showRulers={showRulers}
              guides={guides} onGuidesChange={g => setGuidesByScreen(all => ({ ...all, [screen]: g }))}
              tool={tool} connectFrom={connectFrom} onConnectPick={connectPick}
              selectedConnection={selConn} onSelectConnection={id => { setSelConn(id); setSelectionRaw([]) }}
              onSelect={setSelection} onEdit={edit} onDrop={onDrop} onCursor={onCursor}
              onViewport={live ? onViewport : undefined} viewTo={following != null ? viewTo : null} onUserView={() => setFollowing(null)}
              onCommentAt={(pt, target) => {
                const w = target ? doc.widgets.find(x => x.id === target) : undefined
                setNewComment({ x: pt.x, y: pt.y, object_id: w?.id ?? null, object_type: w ? 'widget' : null })
                setTool('select')
              }}
              overlay={sx => (
                <>
                  {hasPhase4 && <CommentPins comments={comments} doc={doc} sx={sx} active={activeComment} showResolved={showResolvedPins}
                    onOpen={id => { setActiveComment(id); setRightTab('comments') }} />}
                  {live && <PeersOverlay peers={collab.peers} doc={doc} sx={sx} />}
                </>
              )}
            />
          )}
          {!preview && !previewAt && tool === 'connect' && (
            <div style={{ position: 'absolute', top: 30, left: '50%', transform: 'translateX(-50%)', zIndex: 5, padding: '4px 12px', borderRadius: 999, fontSize: D.small, background: D.pane, border: `1px solid ${D.line}`, color: 'var(--text-dim)' }}>
              {connectFrom ? 'Now click the target component · Esc to cancel' : 'Click the source component'}
            </div>
          )}
          {!preview && !previewAt && tool === 'comment' && (
            <div style={{ position: 'absolute', top: 30, left: '50%', transform: 'translateX(-50%)', zIndex: 5, padding: '4px 12px', borderRadius: 999, fontSize: D.small, background: D.pane, border: `1px solid ${D.line}`, color: 'var(--text-dim)' }}>
              Click a component or a point to comment · Esc to cancel
            </div>
          )}
          {following != null && followed && (
            <div data-testid="designer-following" style={{ position: 'absolute', bottom: 10, left: '50%', transform: 'translateX(-50%)', zIndex: 5, padding: '4px 12px', borderRadius: 999, fontSize: D.small, background: D.pane, border: `1px solid ${followed.color}`, color: 'var(--fg)', display: 'flex', gap: 8, alignItems: 'center' }}>
              Following {followed.user}
              <SmallBtn onClick={() => setFollowing(null)}>Stop</SmallBtn>
            </div>
          )}
          {!preview && !previewAt && loaded && doc.widgets.length === 0 && (
            <div style={{ position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', pointerEvents: 'none', color: 'var(--text-muted)', textAlign: 'center', lineHeight: 1.7 }}>
              <div>Drag a tag from the <b>Tag browser</b> onto the screen for a bound component,<br />or a symbol from <b>Components</b>.</div>
            </div>
          )}
          {dropMenu && (
            <DropMenu at={{ x: dropMenu.cx, y: dropMenu.cy }} tag={sensors.find(s => s.id === dropMenu.tagId)} onPick={pickDrop} onClose={() => setDropMenu(null)} />
          )}
        </main>

        {!preview && (
          <aside style={{ width: rightTab === 'properties' ? 312 : 380, flexShrink: 0, display: 'flex', flexDirection: 'column', borderLeft: `1px solid ${D.line}`, background: D.pane }}>
            <Pane title="Components" icon="layers" grow={rightTab === 'properties' ? 0.9 : 0.35} testId="designer-components">
              <PalettePane top={q => (
                <EquipmentTypesSection types={eqTypes} query={q} onAdd={key => addEquipment(key)} installing={installLibrary.isPending}
                  onInstall={() => installLibrary.mutate(undefined, {
                    onSuccess: r => toast.success(`Added ${r.installed.length} equipment types`),
                    onError: (e: any) => toast.error(`Starter types not added: ${e?.message || 'request failed'}`),
                  })} />
              )} onAdd={it => add(make(it.kind, it.key))} />
            </Pane>
            <section aria-label="Properties and history" data-testid="designer-properties" style={{ display: 'flex', flexDirection: 'column', minHeight: 0, flex: 1.6 }}>
              <header style={{ height: 26, flexShrink: 0, display: 'flex', alignItems: 'stretch', gap: 6, padding: '0 6px 0 2px', background: D.chrome, borderBottom: `1px solid ${D.line}` }}>
                <Tabs<RightTab> testId="designer-tab" value={rightTab} onChange={setRightTab} tabs={[
                  { id: 'properties', label: 'Properties', icon: 'faceplate', disabled: !!previewAt },
                  { id: 'history', label: 'History', icon: 'history' },
                  { id: 'versions', label: 'Versions', icon: 'version' },
                  { id: 'mobile', label: t('designerMobile.title'), icon: 'phone' },
                  ...(hasPhase4 ? [{ id: 'comments' as const, label: `Comments${comments.some(c => c.parent_id == null && !c.resolved) ? ` (${comments.filter(c => c.parent_id == null && !c.resolved).length})` : ''}`, icon: 'edit' }] : []),
                ]} />
                <div style={{ flex: 1 }} />
                {rightTab === 'properties' && n > 0 && <span style={{ ...mono, color: 'var(--text-muted)', alignSelf: 'center' }}>{n} selected</span>}
              </header>
              <div style={{ flex: 1, minHeight: 0, overflow: rightTab === 'properties' ? 'auto' : 'hidden' }}>
                {rightTab === 'properties' && (
                  <>
                    {historyObject && (
                      <BlameInfo entry={blame} onHistory={() => { setWholeScreen(false); setRightTab('history') }} />
                    )}
                    {selConn ? (
                      <ConnectionProps conn={doc.connections.find(c => c.id === selConn)!} widgets={doc.widgets}
                        onChange={p => edit({ type: 'patchConnection', id: selConn, patch: p })} onDelete={removeSelection} />
                    ) : (
                      <PropertyEditor doc={doc} selection={selection} sensors={sensors} screenName={screenName} onPatch={patch} arrange={arrangeButtons}
                        equipment={w => {
                          const name = instanceOf(w)
                          if (!name) return null
                          const inst = eqInstances.find(i => i.name === name)
                          return (
                            <InstanceSection key={`${w.id}:${name}`} w={w} name={name} instance={inst} type={eqTypes.find(t => t.id === inst?.type)} sensors={sensors}
                              onApply={applyInstance}
                              onUnbind={() => patch({ [w.id]: { config: { equipment: undefined } } })}
                              onRebind={i => patch({ [w.id]: bindInstancePatch(w, i.name, i.resolved.widget) })} />
                          )
                        }} />
                    )}
                  </>
                )}
                {rightTab === 'history' && (
                  <HistoryList screen={screen} object={historyObject} wholeScreen={wholeScreen} onWholeScreen={setWholeScreen}
                    previewSeq={previewAt?.maxSeq ?? null} onPreview={setPreviewAt} canRevert={canConfigure}
                    onRevertObject={revertObject}
                    onRevertScreen={b => revertWholeScreen(b.maxSeq, `it was after change #${b.maxSeq} (${b.username}, ${relativeTime(b.at_ms)})`)}
                    comments={hasPhase4 ? comments : undefined} onOpenComment={c => { setActiveComment(c.parent_id ?? c.id); setRightTab('comments') }} />
                )}
                {rightTab === 'comments' && (
                  <CommentsPane comments={comments} me={me?.username ?? collab.me.user} isAdmin={isAdmin} users={mentionUsers}
                    active={activeComment} onActive={id => { setActiveComment(id); setShowResolvedPins(true) }}
                    labelOf={c => {
                      if (!c.object_id) return `the stage at ${Math.round(c.x)}, ${Math.round(c.y)}`
                      const w = doc.widgets.find(x => x.id === c.object_id)
                      return w ? String(w.config?.label || w.name || widgetTitle(w)) : `${c.object_id} (removed)`
                    }}
                    onReply={async (root, body) => {
                      try { const c = await postComment(screen, { body, parent_id: root.id, object_id: root.object_id, object_type: root.object_type, x: root.x, y: root.y }); applyCommentEvent(qc, screen, 'created', c) }
                      catch (e: any) { toast.error(`Reply not sent: ${e?.message || 'request failed'}`) }
                    }}
                    onResolve={(root, resolved) => {
                      patchComment(screen, root.id, { resolved }).then(c => applyCommentEvent(qc, screen, 'updated', c), (e: any) => toast.error(`Not changed: ${e?.message || 'request failed'}`))
                    }}
                    onDelete={c => {
                      deleteComment(screen, c.id).then(() => applyCommentEvent(qc, screen, 'deleted', c), (e: any) => toast.error(`Not deleted: ${e?.message || 'request failed'}`))
                    }}
                    onNew={() => {
                      const w = selWidgets[0]
                      if (w) setNewComment({ x: w.x + w.width, y: w.y, object_id: w.id, object_type: 'widget' })
                      else setTool('comment')
                    }}
                    hint="Select a component and press New comment, or press M and click the stage." />
                )}
                {rightTab === 'mobile' && (
                  <MobilePane widgets={doc.widgets} sensors={sensors} selection={selection} onSelect={setSelection} onPatch={patch}
                    labelOf={w => mobileLabel(w, sensors.find(s => s.id === w.sensorId)) || nameOf(w)}
                    readOnly={!!previewAt} screenName={screenName} />
                )}
                {rightTab === 'versions' && (
                  <VersionsList screen={screen} me={{ username: me?.username ?? '', isAdmin }} canConfigure={canConfigure} publishing={publishing && !!status?.published}
                    dirty={dirty} ensureSaved={() => save()} onCompare={(from, to) => setCompare({ from, to })}
                    onChanged={() => { void invalidateDesign(qc, screen) }} />
                )}
              </div>
            </section>
          </aside>
        )}
      </div>

      {picker && (() => {
        const t = eqTypes.find(x => x.id === picker.type)
        if (!t) return null
        const devices = [...new Set([...modbusDevices.map(d => d.id), ...sensors.map(s => s.device_id).filter(Boolean) as string[]])].sort()
        return (
          <InstancePicker type={t} instances={eqInstances} onScreen={onScreenInstances} devices={devices}
            modbusDevices={new Set(modbusDevices.map(d => d.id))} onPick={placeInstance} onCancel={() => setPicker(null)} />
        )
      })()}

      {remoteChanged && !conflict && (
        <div role="status" data-testid="designer-remote-changed" style={{
          position: 'absolute', top: 72, left: '50%', transform: 'translateX(-50%)', zIndex: 20, display: 'flex', alignItems: 'center', gap: 8,
          padding: '5px 8px 5px 12px', borderRadius: 4, fontSize: D.small, background: D.pane, border: '1px solid var(--warn)', boxShadow: '0 6px 20px rgba(0,0,0,.35)',
        }}>
          <span>This screen was saved elsewhere while you were editing.</span>
          <SmallBtn onClick={() => { void reloadFromServer() }} testId="designer-remote-reload">Load theirs</SmallBtn>
          <SmallBtn onClick={() => setRemoteChanged(false)}>Keep editing</SmallBtn>
        </div>
      )}

      {blocker.status === 'blocked' && !conflict && (
        <ChoiceDialog title="Unsaved changes" testId="designer-leave-dialog" onCancel={blocker.reset}
          message={<>Save the changes to <b style={{ color: 'var(--fg)' }}>{screenName}</b> before leaving?</>}
          choices={[
            { label: 'Save', primary: true, testId: 'designer-leave-save', onClick: () => {
              const proceed = blocker.proceed
              // A conflict on this save opens the conflict dialog; leaving waits for its answer.
              void save({ then: proceed }).then(ok => { if (ok) proceed() })
            } },
            { label: 'Discard', danger: true, testId: 'designer-leave-discard', onClick: () => { blocker.proceed() } },
            { label: 'Cancel', testId: 'designer-leave-cancel', onClick: () => { blocker.reset() } },
          ]} />
      )}

      {conflict && (
        <ChoiceDialog title="Screen changed elsewhere" testId="designer-conflict-dialog"
          onCancel={() => { setConflict(null); if (blocker.status === 'blocked') blocker.reset() }}
          message={<>
            {conflict.message}. Load their version (your changes here are lost), or overwrite it with yours?
            {conflict.changesSince.length > 0 && <ChangesSince batches={conflict.changesSince} />}
          </>}
          choices={[
            { label: 'Load theirs', testId: 'designer-conflict-reload', onClick: () => {
              const then = conflict.then
              setConflict(null)
              void reloadFromServer().then(() => then?.())
            } },
            { label: 'Overwrite with mine', danger: true, testId: 'designer-conflict-overwrite', onClick: () => {
              const { revision, seq, then } = conflict
              setConflict(null)
              void save({ base: { revision, seq }, then }).then(ok => { if (ok) then?.() })
            } },
            { label: 'Cancel', testId: 'designer-conflict-cancel', onClick: () => { setConflict(null); if (blocker.status === 'blocked') blocker.reset() } },
          ]} />
      )}

      {needSave && (
        <ChoiceDialog title="Unsaved changes" testId="designer-needsave-dialog" onCancel={() => setNeedSave(null)}
          message={<>{needSave.what} Save your changes to <b style={{ color: 'var(--fg)' }}>{screenName}</b> first, or discard them?</>}
          choices={[
            { label: 'Save', primary: true, testId: 'designer-needsave-save', onClick: () => {
              const then = needSave.then
              setNeedSave(null)
              void save().then(ok => { if (ok) then() })
            } },
            { label: 'Discard', danger: true, testId: 'designer-needsave-discard', onClick: () => {
              const then = needSave.then
              setNeedSave(null)
              void reloadFromServer().then(then)
            } },
            { label: 'Cancel', testId: 'designer-needsave-cancel', onClick: () => setNeedSave(null) },
          ]} />
      )}

      {revertAsk && <RevertDialog ask={revertAsk} onCancel={() => setRevertAsk(null)}
        onConfirm={note => { const body = { ...revertAsk.body, ...(note.trim() ? { message: note.trim() } : {}) }; setRevertAsk(null); void runRevert(body) }} />}

      {dangling && (
        <ChoiceDialog title="Restore the widgets it connects?" testId="designer-dangling-dialog" onCancel={() => setDangling(null)}
          message={<>Connection <b style={{ color: 'var(--fg)' }}>{dangling.connection}</b> would end on {dangling.missing.join(', ')}, which no longer exist{dangling.missing.length === 1 ? 's' : ''}. Restore {dangling.missing.length === 1 ? 'it' : 'them'} too, as {dangling.missing.length === 1 ? 'it was' : 'they were'} at the same point?</>}
          choices={[
            { label: `Also restore ${dangling.missing.join(', ')}`, primary: true, testId: 'designer-revert-also-restore', onClick: () => {
              const body = withAlsoRestore(dangling.body, dangling.missing)
              setDangling(null)
              void runRevert(body)
            } },
            { label: 'Cancel', testId: 'designer-dangling-cancel', onClick: () => setDangling(null) },
          ]} />
      )}

      {publishOpen && (
        <PublishDialog screen={screen} status={status} versions={versions} version={publishOpen.version} dirty={dirty} live={live}
          save={() => save()} baseSeq={() => baseSeq.current} onClose={() => setPublishOpen(null)}
          onDone={() => { void invalidateDesign(qc, screen); invalidate(screen) }} />
      )}

      {compare && (
        <CompareView screen={screen} from={compare.from} to={compare.to} versions={versions} sensors={sensors}
          canConfigure={canConfigure} publishing={publishing} onClose={() => setCompare(null)}
          onRevertTo={(spec, label) => { setCompare(null); revertWholeScreen(spec, label) }}
          onPublishVersion={(id, label) => { setCompare(null); setPublishOpen({ version: { id, label } }) }} />
      )}

      {newComment && (
        <Dialog title={newComment.object_id ? `Comment on ${(() => { const w = doc.widgets.find(x => x.id === newComment.object_id); return w ? String(w.config?.label || w.name || widgetTitle(w)) : newComment.object_id })()}` : 'Comment on the stage'}
          testId="designer-comment-dialog" onCancel={() => setNewComment(null)} actions={<DialogBtn onClick={() => setNewComment(null)}>Cancel</DialogBtn>}>
          <CommentBox users={mentionUsers} placeholder="Write a comment… (@ to mention)" autoFocus testId="designer-comment-body"
            onSend={async body => {
              try {
                const c = await postComment(screen, { body, x: newComment.x, y: newComment.y, object_id: newComment.object_id, object_type: newComment.object_type })
                applyCommentEvent(qc, screen, 'created', c)
                setNewComment(null)
                setActiveComment(c.id)
                setRightTab('comments')
                const named = mentionsIn(body, mentionUsers)
                toast.success(named.length ? `Comment added; ${named.join(', ')} notified` : 'Comment added')
              } catch (e: any) { toast.error(`Comment not added: ${e?.message || 'request failed'}`) }
            }} />
        </Dialog>
      )}

      {/* Status bar */}
      <div data-testid="designer-status" style={{ height: 22, flexShrink: 0, display: 'flex', alignItems: 'center', gap: 14, padding: '0 10px', borderTop: `1px solid ${D.line}`, background: D.chrome, ...mono, color: 'var(--text-dim)' }}>
        <span style={{ color: preview || previewAt ? D.accent : undefined }}>{preview ? 'PREVIEW · live data · F5 or Esc to edit' : previewAt ? `HISTORY · after change #${previewAt.maxSeq} · read only · Esc for the draft` : 'DESIGN'}</span>
        {!preview && <span>{cursor ? `x ${cursor.x}  y ${cursor.y}` : 'x —  y —'}</span>}
        {!preview && <span>{n ? `${n} selected · ${selBox!.x}, ${selBox!.y} · ${selBox!.width} × ${selBox!.height}` : `${doc.widgets.length} components`}</span>}
        <span>{guides.length} guide{guides.length === 1 ? '' : 's'}</span>
        <div style={{ flex: 1 }} />
        <LiveCount sensors={sensors} />
        <span>{undoDepth} undo · {redoDepth} redo</span>
        {live ? (
          <span style={{ color: collab.status !== 'live' ? 'var(--warn)' : undefined }} data-testid="designer-dirty">
            {collab.status !== 'live' ? (collab.pending ? 'Offline: your edits are kept and sent on reconnect' : 'Offline') : collab.pending ? 'Saving…' : 'All changes saved'}
          </span>
        ) : (
          <span style={{ color: dirty ? 'var(--warn)' : undefined }} data-testid="designer-dirty">{dirty ? 'Unsaved changes' : 'All changes saved'}</span>
        )}
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

const TONES = { ok: 'var(--ok)', warn: 'var(--warn)', muted: 'var(--text-dim)', info: 'var(--cyan)' } as const

/** The screen as it was after a batch, read only, with a banner back to the draft. */
function PastState({ screen, batch, sensors, canRevert, onBack, onRevert }: {
  screen: string; batch: Batch; sensors: SensorConfig[]; canRevert: boolean; onBack: () => void; onRevert: () => void
}) {
  const q = useScreenState(screen, `seq:${batch.maxSeq}`)
  return (
    <div data-testid="designer-history-preview" style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
      <div role="status" data-testid="designer-preview-banner" style={{
        flexShrink: 0, display: 'flex', alignItems: 'center', gap: 8, padding: '5px 10px', fontSize: D.font,
        background: 'color-mix(in srgb, var(--cyan) 12%, var(--card))', borderBottom: `1px solid color-mix(in srgb, var(--cyan) 40%, transparent)`,
      }}>
        <Icon name="history" size={13} />
        <span style={{ flex: 1, minWidth: 0 }}>
          Viewing the state after change <b>#{batch.maxSeq}</b> by <b>{batch.username}</b> ({sourceLabel(batch.source)}, {relativeTime(batch.at_ms)}) · read only
        </span>
        {canRevert && <SmallBtn onClick={onRevert} testId="designer-preview-revert">Revert screen to here</SmallBtn>}
        <SmallBtn onClick={onBack} testId="designer-preview-back">Back to draft</SmallBtn>
      </div>
      {q.data ? (
        <StageView rows={q.data} sensors={sensors} testId="designer-preview-stage" />
      ) : (
        <div style={{ flex: 1, display: 'grid', placeItems: 'center', color: q.isError ? 'var(--crit)' : 'var(--text-muted)' }}>
          {q.isError ? `Not loaded: ${(q.error as Error)?.message}` : 'Loading that state…'}
        </div>
      )}
    </div>
  )
}

/** Who changed what since the editor loaded the screen (a 409's `changes_since`). */
function ChangesSince({ batches }: { batches: ChangeSince[] }) {
  const shown = batches.slice(-6).reverse()
  return (
    <div data-testid="designer-conflict-changes" style={{ marginTop: 8, display: 'flex', flexDirection: 'column', gap: 4, maxHeight: 180, overflow: 'auto' }}>
      <div style={{ fontWeight: 600, color: 'var(--fg)' }}>Changed since you loaded it:</div>
      {shown.map(b => (
        <div key={b.batch} data-testid="designer-conflict-change" style={{ display: 'flex', flexDirection: 'column', gap: 1, paddingLeft: 8, borderLeft: `2px solid ${D.line}` }}>
          <span><b style={{ color: 'var(--fg)' }}>{b.username}</b> via {sourceLabel(b.source)} · {relativeTime(b.at_ms)}</span>
          <span>
            {kindSummary(b.objects.map(o => o.kind))}: {b.objects.slice(0, 6).map(o => o.object_id).join(', ')}{b.objects.length > 6 ? ` and ${b.objects.length - 6} more` : ''}
          </span>
          {b.note && <span style={{ fontStyle: 'italic' }}>{b.note}</span>}
        </div>
      ))}
      {batches.length > shown.length && <span>…and {batches.length - shown.length} earlier saves</span>}
    </div>
  )
}

function RevertDialog({ ask, onConfirm, onCancel }: { ask: RevertAsk; onConfirm: (note: string) => void; onCancel: () => void }) {
  const [note, setNote] = useState('')
  return (
    <Dialog title={ask.title} onCancel={onCancel} testId="designer-revert-dialog" actions={<>
      <DialogBtn primary onClick={() => onConfirm(note)} testId="designer-revert-confirm" autoFocus>Revert</DialogBtn>
      <DialogBtn onClick={onCancel} testId="designer-revert-cancel">Cancel</DialogBtn>
    </>}>
      <div style={{ lineHeight: 1.5, color: 'var(--text-dim)' }}>{ask.message}</div>
      <input value={note} onChange={e => setNote(e.target.value)} placeholder="Note for the history (optional)" aria-label="Note"
        data-testid="designer-revert-note" maxLength={2000} style={input} />
    </Dialog>
  )
}
