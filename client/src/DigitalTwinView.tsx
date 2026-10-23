import { useState, useEffect, useCallback, useRef, useMemo, Suspense } from 'react'
import { Canvas, useThree } from '@react-three/fiber'
import { OrbitControls } from '@react-three/drei'
import * as THREE from 'three'
import { SensorConfig, SensorData, TwinEquipment, EquipmentType } from './types'
import { SolarArrayModel, CabinetModel, type ModelProps } from './twin3d/models'
import { TwinEnvironment } from './twin3d/Stage'
import { PlantTwin } from './twin3d/PlantTwin'
import { loadLayoutFromAPI } from './scada2/engine/layout'
import { resolveSymbol } from './scada2/hmi/resolve'
import type { ScadaEquipmentItem, ScadaConnectionItem } from './types'
import { ConnectionLines } from './twin/ConnectionLines'
import { EquipmentLabel } from './twin/EquipmentLabel'
import { EquipmentPalette } from './twin/EquipmentPalette'
import { DEFAULT_EQUIPMENT } from './twin/defaults'
import { useDashboardSelector } from './store/dashboardStore'

interface FlowConnection {
  from: string
  to: string
}

interface Props {
  sensors: SensorConfig[]
  activeScreen: string
  twinLayouts: any[]
  twinCamera?: { x: number; y: number; z: number }
  twinConnections: { from_id: string; to_id: string }[]
  onEquipmentClick: (equipmentId: string) => void
  onSwitchToCards?: () => void
  onSensorUpdate: (sensor: SensorConfig) => void
  onSaveLayouts: (equipment: TwinEquipment[]) => void
  onSaveConnections: (connections: FlowConnection[]) => void
  onSaveCamera: (camera: { x: number; y: number; z: number }) => void
  /** The screen's SCADA layout: when it has equipment, the 3D view builds the plant twin from it. */
  scadaEquipment?: ScadaEquipmentItem[]
  scadaConnections?: ScadaConnectionItem[]
  screenName?: string
  onSensorClick?: (sensorId: number) => void
}

const ROTATION_STEP = Math.PI / 12 // 15 degrees

function DraggableEquipment({ equipment, onDragEnd, onRotate, onClick, sensors, currentValues, editable, highlight, children }: {
  equipment: TwinEquipment
  onDragEnd: (id: string, pos: [number, number, number]) => void
  onRotate: (id: string, rotation: number) => void
  onClick: () => void
  sensors: SensorConfig[]
  currentValues: Record<string, SensorData>
  editable: boolean
  highlight?: boolean
  children: React.ReactNode
}) {
  const [hovered, setHovered] = useState(false)
  const [dragging, setDragging] = useState(false)
  const { camera, raycaster, gl } = useThree()
  const plane = useRef(new THREE.Plane(new THREE.Vector3(0, 1, 0), 0))
  const intersection = useRef(new THREE.Vector3())
  const offset = useRef(new THREE.Vector3())
  const groupRef = useRef<THREE.Group>(null)

  const handlePointerDown = useCallback((e: any) => {
    if (!editable) return
    e.stopPropagation()
    setDragging(true)
    raycaster.ray.intersectPlane(plane.current, intersection.current)
    offset.current.copy(intersection.current).sub(groupRef.current!.position)
    offset.current.y = 0
  }, [editable, raycaster])

  const handlePointerMove = useCallback((e: any) => {
    if (!dragging || !groupRef.current) return
    e.stopPropagation()
    raycaster.ray.intersectPlane(plane.current, intersection.current)
    const newPos = intersection.current.sub(offset.current)
    groupRef.current.position.x = Math.round(newPos.x * 2) / 2
    groupRef.current.position.z = Math.round(newPos.z * 2) / 2
  }, [dragging, raycaster])

  const handlePointerUp = useCallback((e: any) => {
    if (!dragging || !groupRef.current) return
    setDragging(false)
    onDragEnd(equipment.id, [groupRef.current.position.x, 0, groupRef.current.position.z])
  }, [dragging, equipment.id, onDragEnd])

  // Rotation via mouse wheel in edit mode
  useEffect(() => {
    if (!editable || !hovered) return
    const handler = (e: WheelEvent) => {
      e.preventDefault()
      e.stopPropagation()
      const delta = e.deltaY > 0 ? ROTATION_STEP : -ROTATION_STEP
      onRotate(equipment.id, equipment.rotation + delta)
    }
    const canvas = gl.domElement
    canvas.addEventListener('wheel', handler, { passive: false })
    return () => canvas.removeEventListener('wheel', handler)
  }, [editable, hovered, equipment.id, equipment.rotation, onRotate, gl])

  const showHighlight = editable && (hovered || highlight)

  return (
    <group
      ref={groupRef}
      position={equipment.position}
      rotation={[0, equipment.rotation, 0]}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      onPointerOver={() => setHovered(true)}
      onPointerOut={() => setHovered(false)}
      onClick={(e: any) => { e.stopPropagation(); onClick() }}
    >
      {children}
      {showHighlight && (
        <mesh>
          <sphereGeometry args={[1.5, 16, 16]} />
          <meshBasicMaterial color={highlight ? '#22c55e' : '#06b6d4'} wireframe transparent opacity={0.15} />
        </mesh>
      )}
      {editable && hovered && (
        <mesh position={[0, 0.05, 0]} rotation={[-Math.PI / 2, 0, 0]}>
          <ringGeometry args={[1.2, 1.35, 32]} />
          <meshBasicMaterial color="#06b6d4" transparent opacity={0.3} side={THREE.DoubleSide} />
        </mesh>
      )}
      <EquipmentLabel
        name={equipment.name}
        sensors={sensors}
        currentValues={currentValues}
      />
    </group>
  )
}

export function DigitalTwinView(props: Props) {
  const { scadaEquipment = [], scadaConnections = [], sensors, screenName, onSensorClick } = props
  const plant = useMemo(() => loadLayoutFromAPI(scadaEquipment, scadaConnections), [scadaEquipment, scadaConnections])
  const hasPlant = plant.widgets.some(w => resolveSymbol(w) != null)
  const [free, setFree] = useState(false)
  if (hasPlant && !free) {
    return (
      <div style={{ display: 'flex', flexDirection: 'column', height: 'calc(100vh - 150px)', minHeight: 560 }}>
        <PlantTwin
          widgets={plant.widgets}
          connections={plant.connections}
          sensors={sensors}
          screenName={screenName ?? '3D Plant Twin'}
          onOpenDetail={id => onSensorClick?.(id)}
          onFreeLayout={() => setFree(true)}
        />
      </div>
    )
  }
  return <FreeLayoutTwin {...props} onBackToPlant={hasPlant ? () => setFree(false) : undefined} />
}

/** Hand-placed equipment twin for screens without a SCADA layout. */
function FreeLayoutTwin({ sensors, twinLayouts, twinCamera, twinConnections, onEquipmentClick, onSwitchToCards, onSensorUpdate, onSaveLayouts, onSaveConnections, onSaveCamera, onBackToPlant }: Props & { onBackToPlant?: () => void }) {
  const currentValues = useDashboardSelector(s => s.currentValues)
  const [equipment, setEquipment] = useState<TwinEquipment[]>(DEFAULT_EQUIPMENT.map(e => ({ ...e })))
  const [twinEditMode, setTwinEditMode] = useState(false)
  const [bindingEquipmentId, setBindingEquipmentId] = useState<string | null>(null)
  const [connections, setConnections] = useState<FlowConnection[]>([])
  const [connectFrom, setConnectFrom] = useState<string | null>(null)
  const cameraRef = useRef<THREE.Camera | null>(null)
  const [cameraMoved, setCameraMoved] = useState(false)
  const savedCameraPos = useRef(twinCamera ? [twinCamera.x, twinCamera.y, twinCamera.z] : [6, 6, 6])

  const [webglSupported] = useState(() => {
    try {
      const canvas = document.createElement('canvas')
      return !!(canvas.getContext('webgl') || canvas.getContext('experimental-webgl'))
    } catch {
      return false
    }
  })

  useEffect(() => {
    if (twinCamera) {
      savedCameraPos.current = [twinCamera.x, twinCamera.y, twinCamera.z]
    }
    setCameraMoved(false)
  }, [twinCamera])

  useEffect(() => {
    if (twinLayouts.length > 0) {
      setEquipment(twinLayouts.map((l: any) => ({
        id: l.equipment_id,
        type: l.equipment_type as EquipmentType,
        name: l.equipment_name || l.equipment_id,
        position: [l.position_x, 0, l.position_z] as [number, number, number],
        rotation: l.rotation_y,
      })))
    }
  }, [twinLayouts])

  useEffect(() => {
    if (twinConnections.length > 0) {
      setConnections(twinConnections.map(c => ({ from: c.from_id, to: c.to_id })))
    }
  }, [twinConnections])

  const handleDragEnd = useCallback((id: string, pos: [number, number, number]) => {
    setEquipment(prev => prev.map(eq => eq.id === id ? { ...eq, position: pos } : eq))
  }, [])

  const handleRotate = useCallback((id: string, rotation: number) => {
    setEquipment(prev => prev.map(eq => eq.id === id ? { ...eq, rotation } : eq))
  }, [])

  const handleAddEquipment = useCallback((type: EquipmentType) => {
    const id = `${type}_${Date.now()}`
    const names: Record<EquipmentType, string> = { solar_panel: 'Solar Panel', inverter: 'Inverter', gateway: 'Gateway' }
    setEquipment(prev => [...prev, { id, type, name: names[type], position: [Math.random() * 4 - 2, 0, Math.random() * 4 - 2] as [number, number, number], rotation: 0 }])
  }, [])

  const handleRemoveEquipment = useCallback((id: string) => {
    setEquipment(prev => prev.filter(eq => eq.id !== id))
    setConnections(prev => prev.filter(c => c.from !== id && c.to !== id))
  }, [])

  const handleRenameEquipment = useCallback((id: string, name: string) => {
    setEquipment(prev => prev.map(eq => eq.id === id ? { ...eq, name } : eq))
  }, [])

  const getSensorsForEquipment = useCallback((eqId: string) => {
    return sensors.filter(s => s.equipment_id === eqId)
  }, [sensors])

  const toggleSensorBinding = useCallback((sensorId: number) => {
    const sensor = sensors.find(s => s.id === sensorId)
    if (!sensor) return
    const newEquipmentId = sensor.equipment_id === bindingEquipmentId ? null : bindingEquipmentId
    onSensorUpdate({ ...sensor, equipment_id: newEquipmentId })
  }, [sensors, bindingEquipmentId, onSensorUpdate])

  const handleRemoveConnection = useCallback((from: string, to: string) => {
    setConnections(prev => prev.filter(c => !(c.from === from && c.to === to)))
  }, [])

  if (!webglSupported) {
    return (
      <div style={{ height: 400, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, gap: 12, color: 'var(--fg)' }}>
        <div style={{ fontSize: 16, fontWeight: 600 }}>WebGL Not Supported</div>
        <div style={{ fontSize: 13, color: 'var(--muted)' }}>Your browser does not support WebGL, which is required for the 3D view.</div>
        {onSwitchToCards && (
          <button onClick={onSwitchToCards} style={{ padding: '8px 16px', borderRadius: 6, border: '1px solid var(--cyan)', background: 'transparent', color: 'var(--cyan)', cursor: 'pointer', fontSize: 13 }}>Switch to Card View</button>
        )}
      </div>
    )
  }

  const renderModel = (eq: TwinEquipment) => {
    const eqSensors = getSensorsForEquipment(eq.id)
    let statusColor: string | undefined
    if (eqSensors.length > 0) {
      const hasCrit = eqSensors.some(s => {
        const cv = currentValues[s.id]
        return cv && cv.value >= cv.crit
      })
      const hasWarn = eqSensors.some(s => {
        const cv = currentValues[s.id]
        return cv && cv.value >= cv.warn
      })
      statusColor = hasCrit ? '#ef4444' : hasWarn ? '#eab308' : undefined
    }
    const p: ModelProps = {
      w: 3, d: 2, h: 2, level: null, value: '', config: {}, runState: 'idle',
      state: statusColor === '#ef4444' ? 'fault' : eqSensors.length ? 'running' : 'idle',
    }
    switch (eq.type) {
      case 'solar_panel': return <SolarArrayModel {...p} />
      case 'inverter': return <CabinetModel {...p} w={1.6} d={1.4} screen />
      case 'gateway': return <CabinetModel {...p} w={1} d={1} />
    }
  }

  const eqMap = new Map(equipment.map(e => [e.id, e]))

  return (
    <div style={{ position: 'relative', width: '100%', height: 'calc(100vh - 180px)', minHeight: 500, background: '#0b1210', borderRadius: 10, overflow: 'hidden', border: '1px solid var(--border)' }}>
      {twinEditMode && <EquipmentPalette equipment={equipment} onAdd={handleAddEquipment} onRemove={handleRemoveEquipment} />}
      <div style={{ position: 'absolute', top: 8, right: 8, zIndex: 10, display: 'flex', gap: 6, alignItems: 'center' }}>
        {onBackToPlant && !twinEditMode && (
          <button onClick={onBackToPlant} style={{
            padding: '6px 14px', borderRadius: 6, fontSize: 12, fontWeight: 600, cursor: 'pointer',
            border: '1px solid #06b6d4', background: 'rgba(6,182,212,0.15)', color: '#06b6d4', fontFamily: 'Inter, sans-serif',
          }}>Plant twin</button>
        )}
        <button onClick={() => {
          if (twinEditMode) {
            onSaveLayouts(equipment)
            onSaveConnections(connections)
          }
          setTwinEditMode(!twinEditMode)
          setConnectFrom(null)
        }} style={{
          padding: '6px 14px', borderRadius: 6, fontSize: 12, fontWeight: 600, cursor: 'pointer',
          border: twinEditMode ? '1px solid #06b6d4' : '1px solid rgba(255,255,255,0.2)',
          background: twinEditMode ? 'rgba(6,182,212,0.15)' : 'rgba(15,15,20,0.9)',
          color: twinEditMode ? '#06b6d4' : 'rgba(255,255,255,0.6)',
          fontFamily: 'Inter, sans-serif',
        }}>
          {twinEditMode ? 'Done Editing' : 'Edit Layout'}
        </button>
        {twinEditMode && (
          <button onClick={() => { setConnectFrom(connectFrom ? null : '__pick__'); if (connectFrom === '__pick__') setConnectFrom(null) }} style={{
            padding: '6px 14px', borderRadius: 6, fontSize: 12, fontWeight: 600, cursor: 'pointer',
            border: connectFrom === '__pick__' ? '1px solid #22c55e' : '1px solid rgba(255,255,255,0.2)',
            background: connectFrom === '__pick__' ? 'rgba(34,197,94,0.15)' : 'rgba(15,15,20,0.9)',
            color: connectFrom === '__pick__' ? '#22c55e' : 'rgba(255,255,255,0.6)',
            fontFamily: 'Inter, sans-serif',
          }}>
            {connectFrom === '__pick__' ? 'Click source...' : 'Connect Flow'}
          </button>
        )}
        {twinEditMode && <span style={{ fontSize: 10, color: 'rgba(255,255,255,0.35)', fontFamily: 'Inter, sans-serif' }}>Drag to move · Scroll to rotate</span>}
        {!twinEditMode && cameraMoved && (
          <>
            <button onClick={() => {
              if (!cameraRef.current) return
              const p = cameraRef.current.position
              onSaveCamera({ x: Math.round(p.x * 100) / 100, y: Math.round(p.y * 100) / 100, z: Math.round(p.z * 100) / 100 })
              savedCameraPos.current = [p.x, p.y, p.z]
              setCameraMoved(false)
            }} style={{
              padding: '6px 14px', borderRadius: 6, fontSize: 12, fontWeight: 600, cursor: 'pointer',
              border: '1px solid #06b6d4', background: 'rgba(6,182,212,0.15)', color: '#06b6d4',
              fontFamily: 'Inter, sans-serif',
            }}>Save View</button>
            <button onClick={() => {
              if (!cameraRef.current) return
              const [sx, sy, sz] = savedCameraPos.current
              cameraRef.current.position.set(sx, sy, sz)
              cameraRef.current.lookAt(0, 0, 0)
              setCameraMoved(false)
            }} style={{
              padding: '6px 14px', borderRadius: 6, fontSize: 12, fontWeight: 600, cursor: 'pointer',
              border: '1px solid rgba(255,255,255,0.2)', background: 'rgba(15,15,20,0.9)', color: 'rgba(255,255,255,0.6)',
              fontFamily: 'Inter, sans-serif',
            }}>Reset View</button>
          </>
        )}
      </div>

      {/* Connections list in edit mode */}
      {twinEditMode && connections.length > 0 && (
        <div style={{
          position: 'absolute', top: 228, left: 8, zIndex: 10,
          background: 'rgba(15,15,20,0.9)', border: '1px solid rgba(255,255,255,0.1)',
          borderRadius: 8, padding: 12, width: 220, fontFamily: 'Inter, sans-serif',
        }}>
          <div style={{ fontSize: 11, fontWeight: 600, color: 'rgba(255,255,255,0.6)', textTransform: 'uppercase', letterSpacing: '0.05em', marginBottom: 8 }}>
            Flow Connections
          </div>
          {connections.map((c, i) => {
            const fromEq = eqMap.get(c.from)
            const toEq = eqMap.get(c.to)
            return (
              <div key={i} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 4 }}>
                <span style={{ fontSize: 11, color: '#e5e7eb' }}>
                  {fromEq?.name || c.from} → {toEq?.name || c.to}
                </span>
                <button onClick={() => handleRemoveConnection(c.from, c.to)} style={{ background: 'none', border: 'none', color: '#ef4444', cursor: 'pointer', fontSize: 14, lineHeight: 1 }}>&times;</button>
              </div>
            )
          })}
        </div>
      )}

      {connectFrom && connectFrom !== '__pick__' && (
        <div style={{
          position: 'absolute', top: 44, right: 8, zIndex: 10,
          background: 'rgba(34,197,94,0.15)', border: '1px solid rgba(34,197,94,0.3)',
          borderRadius: 6, padding: '6px 12px', fontSize: 11, color: '#22c55e',
          fontFamily: 'Inter, sans-serif',
        }}>
          Source: {eqMap.get(connectFrom)?.name || connectFrom} — click target equipment
        </div>
      )}

      <Suspense fallback={<div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--muted)' }}>Loading 3D...</div>}>
        <Canvas
          camera={{ position: twinCamera ? [twinCamera.x, twinCamera.y, twinCamera.z] : [6, 6, 6], fov: 50 }}
          frameloop="always"
          shadows
          dpr={[1, 2]}
          onCreated={({ camera }) => { camera.lookAt(0, 0, 0); cameraRef.current = camera }}
        >
          <TwinEnvironment size={14} />
          <OrbitControls makeDefault enabled={!twinEditMode} enableDamping dampingFactor={0.1} maxPolarAngle={Math.PI / 2.2} onChange={() => {
            if (!cameraRef.current || twinEditMode) return
            const [sx, sy, sz] = savedCameraPos.current
            const p = cameraRef.current.position
            if (Math.abs(p.x - sx) > 0.05 || Math.abs(p.y - sy) > 0.05 || Math.abs(p.z - sz) > 0.05) {
              setCameraMoved(true)
            }
          }} />
          <ConnectionLines equipment={equipment} connections={connections} />
          {equipment.map(eq => (
            <DraggableEquipment
              key={eq.id}
              equipment={eq}
              onDragEnd={handleDragEnd}
              onRotate={handleRotate}
              onClick={() => {
                if (twinEditMode && connectFrom === '__pick__') {
                  setConnectFrom(eq.id)
                } else if (twinEditMode && connectFrom && connectFrom !== '__pick__') {
                  // Connect from previously selected to this one
                  if (eq.id !== connectFrom) {
                    const exists = connections.some(c => c.from === connectFrom && c.to === eq.id)
                    if (!exists) {
                      setConnections(prev => [...prev, { from: connectFrom, to: eq.id }])
                    }
                  }
                  setConnectFrom(null)
                } else if (twinEditMode) {
                  setBindingEquipmentId(eq.id)
                } else {
                  onEquipmentClick(eq.id)
                }
              }}
              sensors={getSensorsForEquipment(eq.id)}
              currentValues={currentValues}
              editable={twinEditMode}
              highlight={connectFrom === eq.id}
            >
              {renderModel(eq)}
            </DraggableEquipment>
          ))}
        </Canvas>
      </Suspense>
      {twinEditMode && bindingEquipmentId && (() => {
        const eq = equipment.find(e => e.id === bindingEquipmentId)
        if (!eq) return null
        const boundIds = new Set(sensors.filter(s => s.equipment_id === bindingEquipmentId).map(s => s.id))
        return (
          <div style={{
            position: 'absolute', bottom: 16, right: 16, background: 'rgba(15,15,20,0.95)',
            border: '1px solid rgba(255,255,255,0.15)', borderRadius: 10, padding: 16,
            fontFamily: 'Inter, sans-serif', width: 280, maxHeight: '60%', overflow: 'auto',
          }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 12 }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <input
                  value={eq.name}
                  onChange={e => handleRenameEquipment(eq.id, e.target.value)}
                  style={{
                    fontSize: 13, fontWeight: 600, color: '#e5e7eb',
                    background: 'rgba(255,255,255,0.05)', border: '1px solid rgba(255,255,255,0.15)',
                    borderRadius: 4, padding: '2px 6px', width: '100%', outline: 'none',
                  }}
                />
                <div style={{ fontSize: 10, color: 'rgba(255,255,255,0.35)', marginTop: 4 }}>Select sensors to display</div>
              </div>
              <button onClick={() => setBindingEquipmentId(null)} style={{ background: 'none', border: 'none', color: '#9ca3af', cursor: 'pointer', fontSize: 18, lineHeight: 1, marginLeft: 8 }}>&times;</button>
            </div>
            {sensors.length === 0 ? (
              <div style={{ fontSize: 11, color: 'rgba(255,255,255,0.3)' }}>No sensors available</div>
            ) : sensors.map(s => {
              const bound = boundIds.has(s.id)
              const cv = currentValues[s.id]
              return (
                <label key={s.id} style={{
                  display: 'flex', alignItems: 'center', gap: 10, padding: '6px 8px', marginBottom: 2,
                  borderRadius: 6, cursor: 'pointer',
                  background: bound ? 'rgba(6,182,212,0.1)' : 'transparent',
                  border: `1px solid ${bound ? 'rgba(6,182,212,0.3)' : 'rgba(255,255,255,0.05)'}`,
                }}>
                  <input type="checkbox" checked={bound} onChange={() => toggleSensorBinding(s.id)} style={{ accentColor: '#06b6d4' }} />
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontSize: 12, color: bound ? '#e5e7eb' : 'rgba(255,255,255,0.4)' }}>{s.name}</div>
                    <div style={{ fontSize: 10, color: 'rgba(255,255,255,0.25)' }}>{s.key} · {s.unit || '—'}</div>
                  </div>
                  {cv && (
                    <div style={{ fontFamily: 'var(--font-mono)', fontSize: 11, color: s.color, fontWeight: 600 }}>
                      {cv.value != null && !Number.isNaN(cv.value) ? cv.value.toFixed(s.decimals) : '—'}
                    </div>
                  )}
                </label>
              )
            })}
          </div>
        )
      })()}
    </div>
  )
}
