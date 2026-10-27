import { useState, useCallback, useMemo, useEffect, useRef, lazy, Suspense, type ReactNode } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate, useRouterState, Outlet } from '@tanstack/react-router'
import { SensorConfig, ScreenLayouts, CanvasLayoutItem, LayoutItem } from './types'
import { LoginScreen } from './LoginScreen'
import { ChangePasswordScreen } from './ChangePasswordScreen'
import { AddSensorDialog } from './AddSensorDialog'
import { ConfirmDialog, PromptDialog } from './ScreenDialog'
import { useAlarmAudio } from './useAlarmAudio'
import { AppHeader } from './components/AppHeader'
import { ToastContainer } from './components/Toast'
import { ViewToolbar } from './components/ViewToolbar'
import { LayoutBreakpointSelector } from './components/LayoutBreakpointSelector'
import { Footer } from './components/Footer'
import { MainContent } from './components/MainContent'
import { useDashboardData } from './hooks/useDashboardData'
import { useKeyboardNavigation } from './hooks/useKeyboardNavigation'
import { useKioskMode } from './hooks/useKioskMode'
import { useDashboardSystemInfo, useDashboardStoreActions } from './store/dashboardStore'
import { createDashboardStore, DashboardStoreProvider } from './store/dashboardStore'
import { useSensors, useScreenSensors, useScreens, useLayouts, useTwinLayouts, useTwinConnections, useCanvasLayouts, useShortcuts, useNavShortcuts, useActiveAlarmCount, useUpdateSensor, useDeleteSensor, useAddSensor, useSaveLayout, useSaveTwinLayout, useSaveTwinConnections, useSaveCanvasLayout, useSaveShortcuts, useSaveNavShortcuts, useCreateScreen, useRenameScreen, useDeleteScreen, useAddSensorToScreen, useRemoveSensorFromScreen, useScadaEquipment, useScadaConnections, useSaveScadaEquipment, useSaveScadaConnections, fetchOk, fetchJson } from './queries'
import { LicenseContext } from './LicenseContext'
import { useLicenseStatus } from './queries/license'
import { edition, useEditionBranding, DEFAULT_BRANDING } from './edition'
import { OverlayScreen } from './components/OverlayScreen'
import { VoltrusWebSocket, setWsInstance } from './api/websocket'
import { ThemeProvider } from './ThemeContext'
import { useTranslation } from 'react-i18next'
import { duplicateName } from './duplicateName'

// Lazy-loaded heavy components — only fetched when opened
const DashboardSettings = lazy(() => import('./DashboardSettings').then(m => ({ default: m.DashboardSettings })))
const SensorDetail = lazy(() => import('./SensorDetail').then(m => ({ default: m.SensorDetail })))
const DataManager = lazy(() => import('./DataManager').then(m => ({ default: m.DataManager })))
const AlarmPanel = lazy(() => import('./AlarmPanel').then(m => ({ default: m.AlarmPanel })))
const ShiftHandoverPanel = lazy(() => import('./ShiftHandoverPanel').then(m => ({ default: m.ShiftHandoverPanel })))
const DataExportPanel = lazy(() => import('./DataExportPanel').then(m => ({ default: m.DataExportPanel })))
const ShortcutsConfigPanel = lazy(() => import('./ShortcutsConfigPanel').then(m => ({ default: m.ShortcutsConfigPanel })))
const CommandPanel = lazy(() => import('./components/CommandPanel').then(m => ({ default: m.CommandPanel })))
const SystemPanel = lazy(() => import('./components/SystemPanel').then(m => ({ default: m.SystemPanel })))
const FlowManager = lazy(() => import('./FlowManager').then(m => ({ default: m.FlowManager })))
const TagBrowser = lazy(() => import('./TagBrowser').then(m => ({ default: m.TagBrowser })))
const AssetInventory = lazy(() => import('./views/AssetInventory').then(m => ({ default: m.AssetInventory })))
const ConfigSnapshotPanel = lazy(() => import('./components/ConfigSnapshotPanel').then(m => ({ default: m.ConfigSnapshotPanel })))
const AuditLogPanel = lazy(() => import('./AuditLogPanel').then(m => ({ default: m.AuditLogPanel })))
const DataImportWizard = lazy(() => import('./DataImportWizard').then(m => ({ default: m.DataImportWizard })))
const NodeRedImportWizard = lazy(() => import('./NodeRedImportWizard').then(m => ({ default: m.NodeRedImportWizard })))
const MqttBrokerPanel = lazy(() => import('./MqttBrokerPanel').then(m => ({ default: m.MqttBrokerPanel })))
const EdrCompatPanel = lazy(() => import('./EdrCompatPanel').then(m => ({ default: m.EdrCompatPanel })))
const ScriptEditor = lazy(() => import('./components/ScriptEditor').then(m => ({ default: m.ScriptEditor })))

export function AppInner() {
  const queryClient = useQueryClient()
  const { t, ready: i18nReady } = useTranslation()

  // ── Auth state ───────────────────────────────────────────────────
  const [authed, setAuthed] = useState(false)
  const [authRole, setAuthRole] = useState<string>('user')
  const [authUser, setAuthUser] = useState<string>('')
  const [authLoading, setAuthLoading] = useState(true)
  const [loginError, setLoginError] = useState('')
  const [mustChangePassword, setMustChangePassword] = useState(false)

  // ── UI state ─────────────────────────────────────────────────────
  const [editMode, setEditMode] = useState(false)
  const [editingId, setEditingId] = useState<number | null>(null)
  const [dashSettings, setDashSettings] = useState({ gap: 12, refreshInterval: 1000, historyAutoAgg: 'sampling' as const })
  const [showDashSettings, setShowDashSettings] = useState(false)
  // ── Router-backed navigation ────────────────────────────────────
  // selectedSensor / activeScreen / mode / viewMode are derived from the URL
  // (see router.tsx). The old useState + window.history.replaceState hack is
  // gone — the router owns the URL, so deep-links + back/forward just work.
  const navigate = useNavigate()
  const routerLocation = useRouterState({ select: s => s.location })
  const searchParams = (routerLocation.search ?? {}) as { screen?: string; view?: string; mode?: string }
  const activeScreen = searchParams.screen ?? 'all'
  const viewMode: 'cards' | 'canvas' | 'twin' | 'scada' =
    searchParams.view === 'canvas' || searchParams.view === 'twin' || searchParams.view === 'scada' ? searchParams.view : 'cards'
  const liveMode: 'live' | 'history' = searchParams.mode === 'history' ? 'history' : 'live'
  const _pathSegs = routerLocation.pathname.split('/').filter(Boolean)
  const _routePage = _pathSegs[0] ?? ''
  const selectedSensor = _routePage === 'sensor' && _pathSegs[1] ? Number(_pathSegs[1]) : null
  const _overlayPage = _routePage === 'system' || _routePage === 'flows' || _routePage === 'tags' || _routePage === 'assets' || _routePage === 'scripts'
    ? (_routePage as 'system' | 'flows' | 'tags' | 'assets' | 'scripts') : null
  const mode: 'live' | 'history' | 'system' | 'flows' | 'tags' | 'assets' | 'scripts' = _overlayPage ?? liveMode
  const [historyRange, setHistoryRange] = useState('30m')
  const [historyDetail, setHistoryDetail] = useState<'auto' | 'full'>('auto')
  const [showAddDialog, setShowAddDialog] = useState(false)
  const [screenPrompt, setScreenPrompt] = useState<{ title: string; label: string; placeholder?: string; initialValue?: string; shortcutValue?: string; okLabel?: string; onSubmit: (v: string, shortcut?: string | null) => void } | null>(null)
  const [screenConfirm, setScreenConfirm] = useState<{ title: string; message: string; onConfirm: () => void } | null>(null)
  const [showBrandingDialog, setShowBrandingDialog] = useState(false)
  const [layoutBreakpoint, setLayoutBreakpoint] = useState<string | null>(null)
  // Setters navigate instead of setState — each change is a real history
  // entry, so deep-links + browser back/forward work end to end.
  const setActiveScreen = useCallback((id: string) => navigate({ to: '/', search: prev => ({ ...prev, screen: id }) }), [navigate])
  const setViewMode = useCallback((v: 'cards' | 'canvas' | 'twin' | 'scada') => navigate({ to: '/', search: prev => ({ ...prev, view: v }) }), [navigate])
  const setSelectedSensor = useCallback((id: number | null) => {
    if (id == null) navigate({ to: '/' })
    else navigate({ to: '/sensor/$id', params: { id: String(id) } })
  }, [navigate])
  const setMode = useCallback((m: 'live' | 'history' | 'system' | 'flows' | 'tags' | 'assets' | 'scripts') => {
    if (m === 'system' || m === 'flows' || m === 'tags' || m === 'assets' || m === 'scripts') navigate({ to: `/${m}`, search: prev => ({ ...prev, screen: 'all' }) })
    else navigate({ to: '/', search: prev => ({ ...prev, mode: m }) })
  }, [navigate])
  const toggleAlarms = useCallback(() => navigate({ to: _routePage === 'alarms' ? '/' : '/alarms' }), [navigate, _routePage])
  const [scadaToolbarSlot, setScadaToolbarSlot] = useState<React.ReactNode>(null)
  const [localLayouts, setLocalLayouts] = useState<ScreenLayouts | null>(null)
  const [localCanvasLayouts, setLocalCanvasLayouts] = useState<CanvasLayoutItem[] | null>(null)
  const [showShortcutsPanel, setShowShortcutsPanel] = useState(false)
  const [showExportPanel, setShowExportPanel] = useState(false)
  const [showSnapshotPanel, setShowSnapshotPanel] = useState(false)
  const [showAuditPanel, setShowAuditPanel] = useState(false)
  const [showImportWizard, setShowImportWizard] = useState(false)
  const [showNodeRedWizard, setShowNodeRedWizard] = useState(false)
  const [showMqttPanel, setShowMqttPanel] = useState(false)
  const [showEdrPanel, setShowEdrPanel] = useState(false)
  const [alarmMuted, setAlarmMuted] = useState(false)
  const [turbo, setTurbo] = useState(false)

  // ── WebSocket ──────────────────────────────────────────────────
  const wsRef = useRef<VoltrusWebSocket | null>(null)

  const connectWS = useCallback(() => {
    if (wsRef.current) return
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const ws = new VoltrusWebSocket()
    // A raise, acknowledge, shelve or resolve from anywhere: fetch the alarm lists again.
    const refreshAlarms = () => { queryClient.invalidateQueries({ queryKey: ['alarms'] }) }
    ws.onAlarm(refreshAlarms)
    ws.onAlarmUpdate(refreshAlarms)
    ws.connect(`${protocol}//${window.location.host}/api/v1/ws`)
    wsRef.current = ws
    setWsInstance(ws)
  }, [queryClient])

  const disconnectWS = useCallback(() => {
    wsRef.current?.disconnect()
    wsRef.current = null
    setWsInstance(null)
  }, [])

  // cleanup on unmount only
  useEffect(() => () => { wsRef.current?.disconnect(); wsRef.current = null; setWsInstance(null) }, [])

  // ── URL sync is owned by @tanstack/react-router (see router.tsx) ──

  // ── Query hooks ──────────────────────────────────────────────────
  const { data: sensors = [] } = useSensors(authed)
  const { data: screenSensors = [] } = useScreenSensors(activeScreen, authed)
  const { data: screens = [] } = useScreens(authed)
  const { data: serverLayouts } = useLayouts(activeScreen, authed)
  const screenLayouts = localLayouts ?? (serverLayouts ?? { lg: [], md: [], sm: [], xs: [] })
  const { data: twinLayoutsData } = useTwinLayouts(activeScreen, authed)
  const twinLayouts = twinLayoutsData?.items ?? []
  const twinCamera = twinLayoutsData?.camera
  const { data: twinConnections = [] } = useTwinConnections(activeScreen, authed)
  const { data: scadaEquipment = [], isFetched: scadaEquipmentFetched } = useScadaEquipment(activeScreen, authed && (viewMode === 'scada' || viewMode === 'twin'))
  const { data: scadaConnections = [], isFetched: scadaConnectionsFetched } = useScadaConnections(activeScreen, authed && (viewMode === 'scada' || viewMode === 'twin'))
  const { data: serverCanvasLayouts = [] } = useCanvasLayouts(activeScreen, authed)
  const canvasLayouts = localCanvasLayouts ?? serverCanvasLayouts
  const { data: shortcuts = {} } = useShortcuts(authed)
  const { data: navShortcuts = {} } = useNavShortcuts(authed)
  const licenseInfo = useLicenseStatus()
  const branding = useEditionBranding(licenseInfo) ?? DEFAULT_BRANDING
  const { data: activeAlarms = { count: 0 } } = useActiveAlarmCount(authed)

  // ── Mutation hooks ───────────────────────────────────────────────
  const updateSensorMut = useUpdateSensor()
  const deleteSensorMut = useDeleteSensor()
  const addSensorMut = useAddSensor()
  const saveLayoutMut = useSaveLayout()
  const saveTwinLayoutMut = useSaveTwinLayout()
  const saveTwinConnectionsMut = useSaveTwinConnections()
  const saveCanvasLayoutMut = useSaveCanvasLayout()
  const saveScadaEquipmentMut = useSaveScadaEquipment()
  const saveScadaConnectionsMut = useSaveScadaConnections()
  const saveShortcutsMut = useSaveShortcuts()
  const saveNavShortcutsMut = useSaveNavShortcuts()
  const createScreenMut = useCreateScreen()
  const renameScreenMut = useRenameScreen()
  const deleteScreenMut = useDeleteScreen()
  const addSensorToScreenMut = useAddSensorToScreen()
  const removeSensorFromScreenMut = useRemoveSensorFromScreen()

  // ── Dashboard data hook (now store-backed) ───────────────────────
  const detailSensor = selectedSensor != null ? sensors.find(x => x.id === selectedSensor) : undefined
  const { systemInfo } = useDashboardData(authed, mode, activeScreen, sensors, screenSensors, historyRange, historyDetail, dashSettings.historyAutoAgg, turbo, detailSensor)
  const storeSystemInfo = useDashboardSystemInfo()
  const storeActions = useDashboardStoreActions()
  const effectiveSystemInfo = systemInfo ?? storeSystemInfo

  // ── Derived values ───────────────────────────────────────────────
  const isAdmin = authRole === 'admin'
  const isDemo = authRole === 'demo'
  const isAdminOrDemo = isAdmin || isDemo

  const effectiveNavShortcuts = useMemo(() => {
    const defaults: Record<string, string> = {
      'screen:all': 'Mod+1', 'screen:system': 'Mod+0',
      'view:cards': '', 'view:canvas': '', 'view:twin': '', 'view:scada': '',
      'mode:live': '', 'mode:history': '', 'action:alarms': '',
    }
    screens.forEach((s, i) => { defaults[`screen:${s.id}`] = `Mod+${i + 2}` })
    Object.entries(navShortcuts).forEach(([k, v]) => { if (v) defaults[k] = v })
    return defaults
  }, [screens, navShortcuts])

  const DEFAULT_COLS: Record<string, number> = { lg: 24, md: 12, sm: 4, xs: 2 }
  function generateDefaultLayout(sensorIds: number[], bp: string): LayoutItem[] {
    const cols = DEFAULT_COLS[bp] || 24
    const defaultW = bp === 'lg' ? 6 : bp === 'md' ? 6 : bp === 'xs' ? 2 : 4
    const defaultH = 4
    let x = 0, y = 0
    return sensorIds.map(id => {
      if (x + defaultW > cols) { x = 0; y += defaultH }
      const item = { i: String(id), x, y, w: defaultW, h: defaultH }
      x += defaultW
      return item
    })
  }

  const CHART_SIZES: Record<string, { w: number; h: number }> = {
    gauge: { w: 4, h: 4 },
    sparkline: { w: 6, h: 3 },
    bignumber: { w: 4, h: 3 },
    image: { w: 6, h: 4 },
    text: { w: 6, h: 4 },
  }

  function autoArrangeLayout(sensors: SensorConfig[], bp: string): LayoutItem[] {
    const cols = DEFAULT_COLS[bp] || 24
    const groups = new Map<string, SensorConfig[]>()
    for (const s of sensors) {
      const key = s.device_id || '_'
      if (!groups.has(key)) groups.set(key, [])
      groups.get(key)!.push(s)
    }

    const items: LayoutItem[] = []
    let globalY = 0

    for (const [, group] of groups) {
      let x = 0, rowY = globalY, maxRowH = 0
      for (const s of group) {
        const ct = s.chart_type || 'sparkline'
        let { w, h } = CHART_SIZES[ct] || { w: 6, h: 4 }
        if (bp === 'sm') { w = Math.min(w, 4); h = Math.max(h, 3) }
        if (bp === 'xs') { w = 2; h = 3 }
        if (x + w > cols) { x = 0; rowY += maxRowH; maxRowH = 0 }
        items.push({ i: String(s.id), x, y: rowY, w, h })
        maxRowH = Math.max(maxRowH, h)
        x += w
      }
      globalY = rowY + maxRowH
    }
    return items
  }

  function autoArrangeAll(sensors: SensorConfig[]): ScreenLayouts {
    return { lg: autoArrangeLayout(sensors, 'lg'), md: autoArrangeLayout(sensors, 'md'), sm: autoArrangeLayout(sensors, 'sm'), xs: autoArrangeLayout(sensors, 'xs') }
  }

  const defaultLayouts = useMemo(() => {
    if (screenLayouts.lg.length > 0) return null
    const ids = screenSensors.map(s => s.id)
    if (ids.length === 0) return null
    return { lg: generateDefaultLayout(ids, 'lg'), md: generateDefaultLayout(ids, 'md'), sm: generateDefaultLayout(ids, 'sm'), xs: generateDefaultLayout(ids, 'xs') }
  }, [screenSensors, screenLayouts])

  const effectiveLayouts = defaultLayouts ?? screenLayouts

  // ── Auth ─────────────────────────────────────────────────────────
  useEffect(() => {
    fetchJson<any>('/api/v1/auth/me').then(d => {
      if (d.authenticated) connectWS()
      setAuthed(d.authenticated === true); setAuthRole(d.role || 'user'); setAuthUser(d.username || ''); setAuthLoading(false)
    }).catch(() => { setAuthed(false); setAuthRole('user'); setAuthUser(''); setAuthLoading(false) })
  }, [])

  const handleLogin = async (username: string, password: string) => {
    setLoginError('')
    try {
      // Raw fetch, not fetchOk: the refresh wrapper would rewrite the
      // login endpoint's own 401 ("Invalid credentials") into "Session expired".
      const r = await fetch('/api/v1/auth/login', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ username, password }), credentials: 'include' })
      if (!r.ok) {
        const body = await r.json().catch(() => ({}))
        throw new Error(body.error || `Login failed: ${r.status}`)
      }
      const d = await r.json(); connectWS(); setAuthed(true); setAuthRole(d.role || 'user'); setAuthUser(d.username || username); setMustChangePassword(!!d.must_change_password)
    } catch (e: any) { setLoginError(e.message || 'Login failed') }
  }

  const handleExternalLogin = (d: { username?: string; role?: string; must_change_password?: boolean }) => {
    connectWS(); setAuthed(true); setAuthRole(d.role || 'user'); setAuthUser(d.username || ''); setMustChangePassword(!!d.must_change_password)
  }

  const handleLogout = async () => { disconnectWS(); storeActions.clearData(); await fetchOk('/api/v1/auth/logout', { method: 'POST' }).catch(() => {}); setAuthed(false); setAuthRole('user'); setAuthUser('') }

  // ── Handlers ─────────────────────────────────────────────────────

  const handleSave = async (s: SensorConfig, shortcut?: string | null) => {
    await updateSensorMut.mutateAsync(s)
    if (shortcut !== undefined) {
      const prev = shortcuts[s.id] ?? null
      if (shortcut !== prev) {
        await saveShortcutsMut.mutateAsync({ ...shortcuts, [s.id]: shortcut })
      }
    }
    setEditingId(null)
  }

  const handleDelete = async (id: number) => {
    await deleteSensorMut.mutateAsync(id)
    setEditingId(null)
  }

  const handleAddSensorToManager = async (form: Partial<SensorConfig>) => {
    await addSensorMut.mutateAsync(form)
  }

  const handleDuplicateSensor = async (s: SensorConfig) => {
    const dup = {
      key: s.key + '_copy', name: duplicateName(s.name), unit: s.unit,
      min_val: s.min_val, max_val: s.max_val, warn_val: s.warn_val, crit_val: s.crit_val,
      base_val: s.base_val, decimals: s.decimals, source_type: s.source_type,
      color: s.color, chart_type: s.chart_type, source_config: s.source_config,
      device_id: s.device_id, sort_order: s.sort_order,
      on_delay_secs: s.on_delay_secs, off_delay_secs: s.off_delay_secs,
    }
    await addSensorMut.mutateAsync(dup)
  }

  const handleLayoutChange = useCallback((currentLayout: any[], allLayouts: { [bp: string]: any[] }) => {
    const normalize = (items: any[]) => items.map((l: any) => ({ i: String(l.i), x: l.x, y: l.y, w: l.w, h: l.h }))
    const newLayouts: ScreenLayouts = {
      lg: allLayouts.lg?.length > 0 ? normalize(allLayouts.lg) : [],
      md: allLayouts.md?.length > 0 ? normalize(allLayouts.md) : [],
      sm: allLayouts.sm?.length > 0 ? normalize(allLayouts.sm) : [],
      xs: allLayouts.xs?.length > 0 ? normalize(allLayouts.xs) : [],
    }
    setLocalLayouts(prev => {
      const next = JSON.stringify(newLayouts)
      if (prev && JSON.stringify(prev) === next) return prev
      return newLayouts
    })
  }, [])

  const handleCanvasLayoutChange = useCallback((items: CanvasLayoutItem[]) => {
    setLocalCanvasLayouts(items)
  }, [])

  const handleAutoArrange = useCallback(() => {
    setLocalLayouts(autoArrangeAll(screenSensors))
  }, [screenSensors])

  const handleAddScreen = () => {
    const defaultShortcut = `Mod+${screens.length + 2}`
    setScreenPrompt({
      title: 'New Screen', label: 'Screen name', placeholder: 'My Screen', shortcutValue: defaultShortcut, okLabel: 'Create',
      onSubmit: async (name, shortcut) => {
        const created = await createScreenMut.mutateAsync(name)
        if (shortcut) {
          await saveNavShortcutsMut.mutateAsync({ ...navShortcuts, [`screen:${created.id}`]: shortcut })
        }
      }
    })
  }

  const handleRenameScreen = (id: string, oldName: string) => {
    const currentShortcut = navShortcuts[`screen:${id}`] || effectiveNavShortcuts[`screen:${id}`] || ''
    setScreenPrompt({
      title: 'Rename Screen', label: 'Screen name', initialValue: oldName, shortcutValue: currentShortcut, okLabel: 'Rename',
      onSubmit: async (name, shortcut) => {
        if (name !== oldName) renameScreenMut.mutate({ id, name })
        const target = `screen:${id}`
        if (shortcut !== undefined) {
          const updated = { ...navShortcuts }
          if (shortcut) updated[target] = shortcut; else delete updated[target]
          await saveNavShortcutsMut.mutateAsync(updated)
        }
      }
    })
  }

  const handleDeleteScreen = (id: string, name: string) => {
    setScreenConfirm({
      title: 'Delete Screen', message: `Delete screen "${name}"? This cannot be undone.`,
      onConfirm: () => { deleteScreenMut.mutate(id); if (activeScreen === id) setActiveScreen('all') }
    })
  }

  const handleRemoveSensorFromScreen = (sensorId: number) => {
    removeSensorFromScreenMut.mutate({ screenId: activeScreen, sensorId })
  }

  const handleToggleEditMode = () => {
    if (editMode) {
      if (localLayouts) { saveLayoutMut.mutate({ screenId: activeScreen, layouts: localLayouts }); setLocalLayouts(null) }
      if (localCanvasLayouts) { saveCanvasLayoutMut.mutate({ screenId: activeScreen, layouts: localCanvasLayouts }); setLocalCanvasLayouts(null) }
    }
    setEditMode(!editMode); setEditingId(null); setLayoutBreakpoint(null)
  }

  // ── Keyboard navigation ──────────────────────────────────────────
  useKeyboardNavigation({
    shortcuts, sensors, screens, navShortcuts, effectiveNavShortcuts,
    setSelectedSensor, setActiveScreen, setViewMode, setMode, setShowAlarmPanel: toggleAlarms, mode,
  })

  // ── Kiosk mode ──────────────────────────────────────────
  const kiosk = useKioskMode()

  // ── Alarm audio ──────────────────────────────────────────────────
  const { startAlarm, stopAlarm, toggleMute } = useAlarmAudio()
  useEffect(() => {
    if (activeAlarms.count > 0) startAlarm(); else stopAlarm()
  }, [activeAlarms.count, startAlarm, stopAlarm])

  // ── Render gates ─────────────────────────────────────────────────
  if (!i18nReady) return <div style={{ height: '100vh', display: 'flex', alignItems: 'center', justifyContent: 'center', background: 'var(--bg)', color: 'var(--muted)', fontFamily: 'var(--font-ui)' }}>Loading...</div>
  if (authLoading) return <div style={{ height: '100vh', display: 'flex', alignItems: 'center', justifyContent: 'center', background: 'var(--bg)', color: 'var(--muted)', fontFamily: 'var(--font-ui)' }}>{t('common.loading')}</div>
  if (!authed) return <LoginScreen onLogin={handleLogin} onExternalLogin={handleExternalLogin} error={loginError} branding={branding} />

  if (mustChangePassword) {
    const handleChangePassword = async (oldPassword: string, newPassword: string) => {
      await fetchOk('/api/v1/auth/change-password', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ old_password: oldPassword, new_password: newPassword }) })
      setMustChangePassword(false)
    }
    return <ChangePasswordScreen onSubmit={handleChangePassword} />
  }

  return (
    <LicenseContext.Provider value={licenseInfo}>
    <div style={{ height: '100vh', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
      <a href="#main-content" className="skip-to-content">Skip to content</a>
      {edition.Banners && <edition.Banners />}
      <header role="banner">
      <AppHeader
        branding={branding} editMode={editMode}
        onBrandingClick={() => { if (editMode && edition.branding) setShowBrandingDialog(true) }}
        activeAlarmCount={activeAlarms.count}
        effectiveNavShortcuts={effectiveNavShortcuts}
        showShortcutsPanel={showShortcutsPanel}
        onToggleShortcutsPanel={() => setShowShortcutsPanel(v => !v)}
        shortcutsPanel={showShortcutsPanel ? <ShortcutsConfigPanel effectiveNavShortcuts={effectiveNavShortcuts} navShortcuts={navShortcuts} saveNavShortcutsMut={saveNavShortcutsMut} onClose={() => setShowShortcutsPanel(false)} /> : null}
        mode={mode}
        onSettingsClick={() => setShowDashSettings(true)}
        onDataManagerClick={() => navigate({ to: '/data' })}
        onFlowsClick={() => setMode('flows')}
        onTagsClick={() => setMode('tags')}
        onAssetsClick={() => setMode('assets')}
        isAdmin={isAdminOrDemo}
        username={authUser}
        onLogout={handleLogout}
        onAlarmPanelToggle={toggleAlarms}
        onCommandPanelToggle={() => navigate({ to: _routePage === 'commands' ? '/' : '/commands' })}
        onExportClick={() => setShowExportPanel(true)}
        onShiftHandoverToggle={() => navigate({ to: _routePage === 'shift' ? '/' : '/shift' })}
        onSnapshotsClick={() => setShowSnapshotPanel(v => !v)}
        onAuditPanelToggle={() => setShowAuditPanel(v => !v)}
        onImportWizardClick={() => setShowImportWizard(true)}
        onNodeRedWizardClick={() => setShowNodeRedWizard(true)}
        onMqttPanelClick={() => setShowMqttPanel(true)}
        onEdrPanelClick={() => setShowEdrPanel(true)}
        onSystemClick={() => setMode('system')}
        onScriptsClick={() => setMode('scripts')}
        screens={screens}
        activeScreen={activeScreen}
        onTabClick={(id) => navigate({ to: '/', search: prev => ({ ...prev, screen: id, ...(_overlayPage ? { mode: 'live' as const } : {}) }) })}
        onRenameScreen={handleRenameScreen}
        onDeleteScreen={handleDeleteScreen}
        onAddScreen={handleAddScreen}
        onAddSource={() => setShowAddDialog(true)}
        canAddSource={activeScreen !== 'all'}
        onAutoArrange={handleAutoArrange}
        editLabel={editMode ? 'Done' : 'Edit'}
        onToggleEditMode={handleToggleEditMode}
      />
      </header>

      <ViewToolbar
        viewMode={viewMode} onViewModeChange={setViewMode}
        mode={mode} onModeChange={setMode}
        turbo={turbo} onToggleTurbo={() => setTurbo(v => !v)}
        historyRange={historyRange} onHistoryRangeChange={setHistoryRange}
        historyDetail={historyDetail} onHistoryDetailChange={setHistoryDetail}
        historyAutoAgg={dashSettings.historyAutoAgg}
        scadaToolbarSlot={viewMode === 'scada' ? scadaToolbarSlot : null}
      />

      <main id="main-content" role="main" aria-label="Dashboard" style={{
        flex: 1, overflow: 'auto',
        padding: (viewMode === 'scada' || viewMode === 'canvas' || viewMode === 'twin') ? '0 0 38px 0' : '0 24px 38px',
        display: 'flex', flexDirection: 'column', position: 'relative',
      }} className="main-content">
        {editMode && viewMode === 'cards' && (
          <LayoutBreakpointSelector layoutBreakpoint={layoutBreakpoint} onChange={setLayoutBreakpoint} />
        )}

        <div style={{
          flex: 1,
          display: 'flex',
          flexDirection: 'column',
        }}>
          {mode === 'system' && (
            <OverlayScreen title="System" onClose={() => setMode('live')}>
              <Suspense fallback={null}><SystemPanel systemInfo={effectiveSystemInfo} /></Suspense>
            </OverlayScreen>
          )}
          {mode === 'flows' && (
            <OverlayScreen title="Flow Manager" onClose={() => setMode('live')}>
              <Suspense fallback={null}><FlowManager isAdmin={isAdminOrDemo} /></Suspense>
            </OverlayScreen>
          )}
          {mode === 'tags' && (
            <OverlayScreen title="Tag Browser" onClose={() => setMode('live')}>
              <Suspense fallback={null}><TagBrowser isAdmin={isAdminOrDemo} /></Suspense>
            </OverlayScreen>
          )}
          {mode === 'assets' && (
            <OverlayScreen title="Asset Inventory" onClose={() => setMode('live')}>
              <Suspense fallback={null}><AssetInventory /></Suspense>
            </OverlayScreen>
          )}
          {mode === 'scripts' && (
            <OverlayScreen title="Script Editor" onClose={() => setMode('live')}>
              <Suspense fallback={null}><ScriptEditor /></Suspense>
            </OverlayScreen>
          )}
          {mode !== 'system' && mode !== 'flows' && mode !== 'tags' && mode !== 'assets' && mode !== 'scripts' && selectedSensor == null && (
            <MainContent
              viewMode={viewMode} mode={mode}
              screenSensors={screenSensors}
              effectiveLayouts={effectiveLayouts}
              canvasLayouts={canvasLayouts}
              editMode={editMode} activeScreen={activeScreen}
              gap={dashSettings.gap}
              editingId={editingId}
              setEditingId={setEditingId}
              setSelectedSensor={setSelectedSensor}
              onLayoutChange={handleLayoutChange}
              onCanvasLayoutChange={handleCanvasLayoutChange}
              onSave={handleSave}
              onDelete={handleDelete}
              forcedBreakpoint={layoutBreakpoint}
              shortcuts={shortcuts}
              scadaEquipment={scadaEquipment}
              scadaConnections={scadaConnections}
              scadaLoaded={scadaEquipmentFetched && scadaConnectionsFetched}
              twinLayouts={twinLayouts}
              twinCamera={twinCamera}
              twinConnections={twinConnections}
              sensors={sensors}
              saveScadaEquipmentMut={saveScadaEquipmentMut}
              scadaToolbarSlotSetter={setScadaToolbarSlot}
              screenName={screens.find(sc => sc.id === activeScreen)?.name}
              saveScadaConnectionsMut={saveScadaConnectionsMut}
              saveTwinLayoutMut={saveTwinLayoutMut}
              saveTwinConnectionsMut={saveTwinConnectionsMut}
              updateSensorMut={updateSensorMut}
              fetchOk={fetchOk}
              isAdmin={isAdminOrDemo}
            />
          )}
        </div>

        {/* Detail view */}
        {selectedSensor != null && (
          <div style={{
            position: 'absolute',
            top: 0,
            left: 0,
            right: 0,
            bottom: 0,
            zIndex: 2,
            display: 'flex',
            flexDirection: 'column',
            padding: '0 20px 20px',
            overflow: 'auto',
            background: 'var(--bg)',
          }}>
            {(() => {
              const s = sensors.find(x => x.id === selectedSensor)
              return s ? <Suspense fallback={null}><SensorDetail sensor={s} onBack={() => setSelectedSensor(null)} historyRange={historyRange} onRangeChange={setHistoryRange} historyDetail={historyDetail} onDetailChange={setHistoryDetail} historyAutoAgg={dashSettings.historyAutoAgg} shortcuts={shortcuts} mode={mode === 'history' ? 'history' : 'live'} onModeChange={setMode} allSensors={sensors} isAdmin={isAdminOrDemo} /></Suspense> : null
            })()}
          </div>
        )}



        {showDashSettings && <Suspense fallback={null}><DashboardSettings settings={dashSettings} onChange={setDashSettings} onClose={() => setShowDashSettings(false)} alarmMuted={alarmMuted} onToggleMute={() => { toggleMute(); setAlarmMuted(!alarmMuted) }} kioskMode={kiosk.enabled} onToggleKiosk={kiosk.toggle} /></Suspense>}
        {_routePage === 'data' && <Suspense fallback={null}><DataManager sensors={sensors} onSave={handleSave} onDelete={handleDelete} onAdd={handleAddSensorToManager} onDuplicate={handleDuplicateSensor} onClose={() => navigate({ to: '/' })} shortcuts={shortcuts} isAdmin={isAdminOrDemo} isStrictAdmin={isAdmin} /></Suspense>}
        {showAddDialog && activeScreen !== 'all' && (
          <AddSensorDialog screenId={activeScreen} sensors={sensors} screenSensorIds={screenSensors.map(s => s.id)} onClose={() => setShowAddDialog(false)} onDone={() => { setShowAddDialog(false); queryClient.invalidateQueries({ queryKey: ['screenDataSources', activeScreen] }); queryClient.invalidateQueries({ queryKey: ['sensors'] }) }} />
        )}
        {screenPrompt && <PromptDialog {...screenPrompt} onClose={() => setScreenPrompt(null)} />}
        {screenConfirm && <ConfirmDialog {...screenConfirm} onClose={() => setScreenConfirm(null)} />}
        {showBrandingDialog && edition.branding && <edition.branding.Dialog branding={branding} onClose={() => setShowBrandingDialog(false)} />}
        {_routePage === 'alarms' && <Suspense fallback={null}><AlarmPanel onClose={() => navigate({ to: '/' })} /></Suspense>}
        {_routePage === 'commands' && <Suspense fallback={null}><CommandPanel onClose={() => navigate({ to: '/' })} isAdmin={isAdminOrDemo} /></Suspense>}
        {showExportPanel && <Suspense fallback={null}><DataExportPanel onClose={() => setShowExportPanel(false)} /></Suspense>}
        {_routePage === 'shift' && <Suspense fallback={null}><ShiftHandoverPanel onClose={() => navigate({ to: '/' })} /></Suspense>}
        {showSnapshotPanel && (
          <OverlayScreen title={t('snapshots.title')} onClose={() => setShowSnapshotPanel(false)}>
            <Suspense fallback={null}><ConfigSnapshotPanel /></Suspense>
          </OverlayScreen>
        )}
        {showAuditPanel && (
          <Suspense fallback={null}><AuditLogPanel onClose={() => setShowAuditPanel(false)} /></Suspense>
        )}
        {showImportWizard && <Suspense fallback={null}><DataImportWizard onClose={() => setShowImportWizard(false)} /></Suspense>}
        {showNodeRedWizard && <Suspense fallback={null}><NodeRedImportWizard onClose={() => setShowNodeRedWizard(false)} /></Suspense>}
        {showMqttPanel && <Suspense fallback={null}><MqttBrokerPanel onClose={() => setShowMqttPanel(false)} /></Suspense>}
        {showEdrPanel && (
          <OverlayScreen title="EDR Compatibility" onClose={() => setShowEdrPanel(false)}>
            <Suspense fallback={null}><EdrCompatPanel onClose={() => setShowEdrPanel(false)} /></Suspense>
          </OverlayScreen>
        )}
      </main>

      <Footer activeAlarmCount={activeAlarms.count} viewMode={viewMode} activeScreen={activeScreen} screens={screens} />
      <ToastContainer />
      {/* router outlet (child route components render null; AppInner owns all chrome) */}
      <span style={{ display: 'none' }}><Outlet /></span>
    </div>
    </LicenseContext.Provider>
  )
}

export default function App({ children }: { children: ReactNode }) {
  const storeRef = useRef(createDashboardStore())
  return (
    <DashboardStoreProvider value={storeRef.current}>
      <ThemeProvider>
        {children}
      </ThemeProvider>
    </DashboardStoreProvider>
  )
}
