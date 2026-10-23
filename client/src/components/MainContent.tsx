import { lazy, Suspense, useCallback } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { SensorGrid } from '../SensorGrid'
import { CanvasView } from '../CanvasView'
import { ScreenLayouts, CanvasLayoutItem, SensorConfig } from '../types'
import type { LayoutItem } from '../types'

const DigitalTwinView = lazy(() => import('../DigitalTwinView').then(m => ({ default: m.DigitalTwinView })))
const ScadaView = lazy(() => import('../scada2/ScadaView').then(m => ({ default: m.ScadaView })))

interface Props {
  viewMode: 'cards' | 'canvas' | 'twin' | 'scada'
  mode: 'live' | 'history' | 'system' | 'flows' | 'tags' | 'assets' | 'scripts'
  screenSensors: SensorConfig[]
  effectiveLayouts: ScreenLayouts
  canvasLayouts: CanvasLayoutItem[]
  editMode: boolean
  activeScreen: string
  gap: number
  editingId: number | null
  setEditingId: (id: number | null) => void
  setSelectedSensor: (id: number | null) => void
  onLayoutChange: (current: any[], all: { [bp: string]: any[] }) => void
  onCanvasLayoutChange: (items: CanvasLayoutItem[]) => void
  onSave: (s: SensorConfig, shortcut?: string | null) => void
  onDelete: (id: number) => void
  forcedBreakpoint: string | null
  shortcuts: Record<number, string>
  scadaEquipment: any[]
  scadaConnections: any[]
  scadaLoaded: boolean
  twinLayouts: any[]
  twinCamera: any
  twinConnections: any[]
  sensors: SensorConfig[]
  saveScadaEquipmentMut: any
  saveScadaConnectionsMut: any
  saveTwinLayoutMut: any
  saveTwinConnectionsMut: any
  updateSensorMut: any
  fetchOk: any
  isAdmin: boolean
  scadaToolbarSlotSetter?: (slot: React.ReactNode) => void
  screenName?: string
}

export function MainContent({
  viewMode, mode, screenSensors, effectiveLayouts, canvasLayouts,
  editMode, activeScreen, gap, editingId, setEditingId, setSelectedSensor,
  onLayoutChange, onCanvasLayoutChange, onSave, onDelete,
  forcedBreakpoint, shortcuts, scadaEquipment, scadaConnections, scadaLoaded,
  twinLayouts, twinCamera, twinConnections, sensors,
  saveScadaEquipmentMut, saveScadaConnectionsMut, saveTwinLayoutMut,
  saveTwinConnectionsMut, updateSensorMut, fetchOk, isAdmin,
  scadaToolbarSlotSetter, screenName,
}: Props) {
  const queryClient = useQueryClient()

  const loadSensorsForScreen = useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ['screenDataSources', activeScreen] })
    queryClient.invalidateQueries({ queryKey: ['sensors'] })
  }, [queryClient, activeScreen])

  if (mode === 'system') return null

  if (viewMode === 'cards') {
    return (
      <SensorGrid
        screenLayouts={effectiveLayouts}
        screenSensors={screenSensors}
        editMode={editMode}
        activeScreen={activeScreen}
        gap={gap}
        editingId={editingId}
        setEditingId={setEditingId}
        setSelectedSensor={setSelectedSensor}
        loadSensorsForScreen={loadSensorsForScreen}
        onLayoutChange={onLayoutChange}
        onSave={onSave}
        onDelete={onDelete}
        forcedBreakpoint={forcedBreakpoint}
        shortcuts={shortcuts}
        mode={mode}
      />
    )
  }

  if (viewMode === 'canvas') {
    return (
      <CanvasView
        screenSensors={screenSensors}
        canvasLayouts={canvasLayouts}
        editMode={editMode}
        activeScreen={activeScreen}
        editingId={editingId}
        setEditingId={setEditingId}
        setSelectedSensor={setSelectedSensor}
        loadSensorsForScreen={loadSensorsForScreen}
        onLayoutChange={onCanvasLayoutChange}
        onSave={onSave}
        onDelete={onDelete}
        shortcuts={shortcuts}
        mode={mode}
      />
    )
  }

  if (viewMode === 'scada') {
    return (
      <Suspense fallback={<div style={{ height: 400, display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--muted)' }}>Loading 2D View...</div>}>
        <ScadaView
          sensors={screenSensors}
          activeScreen={activeScreen}
          scadaEquipment={scadaEquipment}
          scadaConnections={scadaConnections}
          loaded={scadaLoaded}
          onSave={async (screenId: string, equipment: any[], connections: any[]) => {
            await Promise.all([
              saveScadaEquipmentMut.mutateAsync({ screenId, items: equipment }),
              saveScadaConnectionsMut.mutateAsync({ screenId, items: connections }),
            ])
          }}
          onSensorClick={(id: number) => setSelectedSensor(id)}
          isAdmin={isAdmin}
          toolbarSlotSetter={scadaToolbarSlotSetter}
        />
      </Suspense>
    )
  }

  return (
    <Suspense fallback={<div style={{ height: 400, display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--muted)' }}>Loading 3D View...</div>}>
      <DigitalTwinView
        sensors={screenSensors}
        activeScreen={activeScreen}
        twinLayouts={twinLayouts}
        twinCamera={twinCamera}
        twinConnections={twinConnections}
        onSwitchToCards={() => setSelectedSensor(null)}
        onEquipmentClick={(eqId: string) => {
          const sensor = sensors.find(s => s.equipment_id === eqId)
          if (sensor) setSelectedSensor(sensor.id)
        }}
        onSensorUpdate={(s: SensorConfig) => updateSensorMut.mutate(s)}
        onSaveLayouts={(equipment: any[]) => {
          saveTwinLayoutMut.mutate({
            screenId: activeScreen,
            items: equipment.map((e: any) => ({
              equipment_id: e.id, equipment_type: e.type, equipment_name: e.name,
              position_x: e.position[0], position_z: e.position[2], rotation_y: e.rotation,
            }))
          })
        }}
        onSaveConnections={(connections: any[]) => {
          saveTwinConnectionsMut.mutate({
            screenId: activeScreen,
            connections: connections.map((c: any) => ({ from_id: c.from, to_id: c.to })),
          })
        }}
        scadaEquipment={scadaEquipment}
        scadaConnections={scadaConnections}
        screenName={screenName}
        onSensorClick={(id: number) => setSelectedSensor(id)}
        onSaveCamera={(camera: any) => {
          const actualId = activeScreen === 'all' ? '__overview' : activeScreen
          fetchOk(`/api/v1/screens/${actualId}/twin-camera`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(camera) })
        }}
      />
    </Suspense>
  )
}
