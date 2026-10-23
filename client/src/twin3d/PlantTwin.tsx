import React, { useCallback, useMemo, useState } from 'react'
import { Canvas, useFrame } from '@react-three/fiber'
import { Html, Line, OrbitControls } from '@react-three/drei'
import * as THREE from 'three'
import type { SensorConfig, SensorData } from '../types'
import type { ConnectionInstance, WidgetInstance } from '../scada2/widgets/types'
import { useDashboardSelector } from '../store/dashboardStore'
import { symbolLiveState, stateText, stateColor, type SymbolLiveState } from '../scada2/hmi/SymbolView'
import { Faceplate } from '../scada2/hmi/Faceplate'
import { AlarmBanner } from '../scada2/AlarmBanner'
import { isRunning } from '../scada2/connections/ConnectionLayer'
import { wsWriteTag } from '../api/websocket'
import { writeTag } from '../api/commands'
import { toast } from '../components/Toast'
import { placeLayout, bounds, type PlacedItem, type PlacedArea } from './placement'
import { Pipes3D, type PipeRun } from './Pipes3D'
import { TwinEnvironment, CameraRig, type CameraGoal } from './Stage'
import { MAT } from './materials'
import * as M from './models'

interface Props {
  widgets: WidgetInstance[]
  connections: ConnectionInstance[]
  sensors: SensorConfig[]
  screenName: string
  onOpenDetail: (sensorId: number) => void
  /** Switch to the hand-placed free layout. */
  onFreeLayout?: () => void
}

const fmt = (d: SensorData | undefined, s: SensorConfig | undefined) => (d?.value == null ? '' : d.value.toFixed(s?.decimals ?? 1))

function EquipmentModel({ it, live, value }: { it: PlacedItem; live: SymbolLiveState; value: string }) {
  const p: M.ModelProps = { w: it.w, d: it.d, h: it.h, state: live.state, runState: live.runState, level: live.level, value, config: it.config }
  switch (it.key) {
    case 'pump': return <M.PumpModel {...p} />
    case 'dosing_pump': return <M.DosingPumpModel {...p} />
    case 'fan': return <M.BlowerModel {...p} />
    case 'motor': return <M.MotorModel {...p} />
    case 'compressor': return <M.CompressorModel {...p} />
    case 'conveyor': return <M.ConveyorModel {...p} />
    case 'valve': return <M.ValveModel {...p} kind="gate" />
    case 'control_valve': return <M.ValveModel {...p} kind="control" />
    case 'check_valve': return <M.ValveModel {...p} kind="check" />
    case 'tank': return <M.TankModel {...p} kind="tank" />
    case 'mixer': return <M.TankModel {...p} kind="mixer" />
    case 'silo': return <M.TankModel {...p} kind="silo" />
    case 'tank_horizontal': return <M.HorizontalTankModel {...p} />
    case 'clarifier': return <M.ClarifierModel {...p} />
    case 'gravity_filter': return <M.GravityFilterModel {...p} />
    case 'heat_exchanger': return <M.HeatExchangerModel {...p} />
    case 'filter': return <M.CartridgeFilterModel {...p} />
    case 'breaker': return <M.MccSectionModel {...p} />
    case 'transformer': return <M.TransformerModel {...p} />
    case 'generator': return <M.GeneratorModel {...p} />
    case 'solar_panel': return <M.SolarArrayModel {...p} />
    case 'battery': return <M.BatteryModel {...p} />
    case 'inverter': return <M.CabinetModel {...p} screen />
    case 'meter': return <M.MeterModel {...p} />
    case 'busbar': return <M.BusbarTrayModel {...p} />
    case 'instrument': return <M.InstrumentModel {...p} />
    case 'flow_meter': return <M.FlowMeterModel {...p} />
    case 'pipe': case 'pipe_vertical': case 'pipe_elbow': case 'pipe_tee': return <M.PipeSpoolModel {...p} />
    default: return <M.GenericModel {...p} />
  }
}

/** Floor tag plate above a piece of equipment: name, live value or state, alarm-coloured. */
function TagPlate({ it, live, value, unit, selected }: { it: PlacedItem; live: SymbolLiveState; value: string; unit: string; selected: boolean }) {
  const label: string = it.config.label || ''
  const showValue = it.widget.sensorId != null && it.def.binding !== 'state' && value !== ''
  const showState = it.def.binding === 'state' && it.widget.sensorId != null
  if (!label && !showValue) return null
  const alarm = live.alarm
  const border = alarm === 'crit' ? 'var(--hmi-fault)' : alarm === 'warn' ? 'var(--hmi-warn)' : selected ? 'var(--cyan)' : 'rgba(255,255,255,.14)'
  return (
    <Html position={[0, it.top + 0.35, 0]} center zIndexRange={[10, 0]} style={{ pointerEvents: 'none' }}>
      <div className={alarm === 'crit' || live.state === 'fault' ? 'hmi-blink' : undefined} style={{
        display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 1, padding: '3px 7px', borderRadius: 4, whiteSpace: 'nowrap',
        background: 'rgba(13,15,18,.86)', border: `1px solid ${live.state === 'fault' ? 'var(--hmi-fault)' : border}`,
        fontFamily: 'Inter, sans-serif', boxShadow: '0 4px 14px rgba(0,0,0,.4)',
      }}>
        {label && <div style={{ fontSize: 11, fontWeight: 700, color: '#e8edf2', letterSpacing: '.02em' }}>{label}</div>}
        {showValue && (
          <div style={{ fontFamily: 'var(--font-mono)', fontSize: 11 }}>
            <span style={{ color: alarm ? border : '#e8edf2', fontWeight: 700 }}>{value}</span>
            {unit && <span style={{ color: '#8b95a1', fontSize: 10, marginLeft: 3 }}>{unit}</span>}
          </div>
        )}
        {showState && <div style={{ fontSize: 9, fontWeight: 800, letterSpacing: '.1em', color: stateColor(live.state) }}>{stateText(it.def, live.state)}</div>}
      </div>
    </Html>
  )
}

function Equipment({ it, live, value, unit, selected, hovered, showLabels, onSelect, onHover }: {
  it: PlacedItem; live: SymbolLiveState; value: string; unit: string; selected: boolean; hovered: boolean; showLabels: boolean
  onSelect: (id: string) => void; onHover: (id: string | null) => void
}) {
  const ring = Math.max(it.hx, it.hz) + 0.35
  const interactive = it.widget.sensorId != null
  return (
    <group position={[it.x, 0, it.z]}>
      <group
        rotation={[0, it.rotY, 0]}
        onClick={interactive ? e => { e.stopPropagation(); onSelect(it.id) } : undefined}
        onPointerOver={interactive ? e => { e.stopPropagation(); onHover(it.id); document.body.style.cursor = 'pointer' } : undefined}
        onPointerOut={interactive ? () => { onHover(null); document.body.style.cursor = '' } : undefined}
      >
        <EquipmentModel it={it} live={live} value={value} />
      </group>
      {(selected || hovered) && (
        <mesh rotation={[-Math.PI / 2, 0, 0]} position={[0, 0.03, 0]}>
          <ringGeometry args={[ring, ring + (selected ? 0.12 : 0.07), 64]} />
          <meshBasicMaterial color={selected ? '#22d3ee' : '#7dd3fc'} transparent opacity={selected ? 0.9 : 0.5} />
        </mesh>
      )}
      {live.state === 'fault' && <FaultRing radius={ring} />}
      {showLabels && <TagPlate it={it} live={live} value={value} unit={unit} selected={selected} />}
    </group>
  )
}

/** Pulsing red ground ring under tripped equipment, visible from across the plant. */
function FaultRing({ radius }: { radius: number }) {
  const ref = React.useRef<THREE.Mesh>(null)
  const mat = useMemo(() => new THREE.MeshBasicMaterial({ color: '#f04646', transparent: true, depthWrite: false }), [])
  useFrame(({ clock }) => {
    const k = (clock.elapsedTime * 1.2) % 1
    if (ref.current) ref.current.scale.setScalar(1 + k * 0.8)
    mat.opacity = 0.75 * (1 - k)
  })
  return (
    <mesh ref={ref} rotation={[-Math.PI / 2, 0, 0]} position={[0, 0.04, 0]} material={mat}>
      <ringGeometry args={[radius, radius + 0.18, 64]} />
    </mesh>
  )
}

function textTexture(title: string, subtitle: string): THREE.CanvasTexture {
  const c = document.createElement('canvas')
  c.width = 1024
  c.height = 160
  const g = c.getContext('2d')!
  g.fillStyle = 'rgba(214,160,30,0.9)'
  g.font = '700 72px Inter, sans-serif'
  g.fillText(title.toUpperCase(), 8, 80)
  if (subtitle) {
    g.fillStyle = 'rgba(200,200,200,0.55)'
    g.font = '500 44px Inter, sans-serif'
    g.fillText(subtitle, 10, 140)
  }
  const t = new THREE.CanvasTexture(c)
  t.anisotropy = 8
  return t
}

/** Concrete pad per process area, outlined in safety yellow with its name painted on the floor. */
function AreaPad({ a }: { a: PlacedArea }) {
  const tex = useMemo(() => textTexture(a.title, a.subtitle), [a.title, a.subtitle])
  const y = 0.025
  const x0 = a.x - a.w / 2 + 0.2, x1 = a.x + a.w / 2 - 0.2, z0 = a.z - a.d / 2 + 0.2, z1 = a.z + a.d / 2 - 0.2
  const tw = Math.min(a.w * 0.8, 9)
  return (
    <group>
      <mesh position={[a.x, 0.01, a.z]} receiveShadow material={MAT.concrete}><boxGeometry args={[a.w - 0.1, 0.02, a.d - 0.1]} /></mesh>
      <Line points={[[x0, y, z0], [x1, y, z0], [x1, y, z1], [x0, y, z1], [x0, y, z0]]} color="#d6a01e" lineWidth={2} transparent opacity={0.7} />
      <mesh rotation={[-Math.PI / 2, 0, 0]} position={[x0 + tw / 2 + 0.1, y + 0.002, z0 + (tw * 160) / 1024 / 2 + 0.15]}>
        <planeGeometry args={[tw, (tw * 160) / 1024]} />
        <meshBasicMaterial map={tex} transparent depthWrite={false} />
      </mesh>
    </group>
  )
}

export const PlantTwin: React.FC<Props> = ({ widgets, connections, sensors, screenName, onOpenDetail, onFreeLayout }) => {
  const currentValues = useDashboardSelector(s => s.currentValues) as Record<string, SensorData>
  const { items, areas } = useMemo(() => placeLayout(widgets), [widgets])
  const box = useMemo(() => bounds(items), [items])
  const byId = useMemo(() => new Map(items.map(i => [i.id, i])), [items])
  const sensorById = useMemo(() => new Map(sensors.map(s => [s.id, s])), [sensors])
  const widgetById = useMemo(() => new Map(widgets.map(w => [w.id, w])), [widgets])

  const [selected, setSelected] = useState<string | null>(null)
  const [hovered, setHovered] = useState<string | null>(null)
  const [showLabels, setShowLabels] = useState(true)
  const [orbit, setOrbit] = useState(false)

  const overview = useMemo<CameraGoal>(() => {
    const dist = Math.max(box.sx, box.sz * 1.7, 12) * 0.66
    return { pos: [box.cx + dist * 0.18, dist * 0.62, box.cz + dist * 0.82], target: [box.cx, 0, box.cz + box.sz * 0.05] }
  }, [box])
  const [goal, setGoal] = useState<CameraGoal | null>(null)
  const arrive = useCallback(() => setGoal(null), [])

  const flyToItem = useCallback((it: PlacedItem) => {
    const dist = Math.max(6, Math.max(it.hx, it.hz) * 5, it.top * 2.2)
    setGoal({ pos: [it.x + dist * 0.45, it.top * 0.5 + dist * 0.5, it.z + dist * 0.85], target: [it.x, it.top * 0.45, it.z] })
  }, [])
  const flyToArea = useCallback((a: PlacedArea) => {
    const dist = Math.max(a.w, a.d * 1.6, 8) * 0.95
    setGoal({ pos: [a.x + dist * 0.15, dist * 0.6, a.z + dist * 0.85], target: [a.x, 0, a.z] })
  }, [])

  const select = useCallback((id: string) => {
    setSelected(id)
    const it = byId.get(id)
    if (it) flyToItem(it)
  }, [byId, flyToItem])

  const live = (it: PlacedItem) => {
    const s = it.widget.sensorId != null ? sensorById.get(it.widget.sensorId) : undefined
    const d = it.widget.sensorId != null ? currentValues[String(it.widget.sensorId)] : undefined
    return { live: symbolLiveState(it.def, it.config, it.widget.sensorId, s, d, currentValues), value: fmt(d, s), unit: s?.unit ?? '' }
  }

  const runs: PipeRun[] = useMemo(() => connections.flatMap(c => {
    const from = byId.get(c.fromWidgetId)
    const to = byId.get(c.toWidgetId)
    if (!from || !to) return []
    const flowing = c.connectionType !== 'data' && (isRunning(widgetById.get(c.fromWidgetId), currentValues) || isRunning(widgetById.get(c.toWidgetId), currentValues))
    return [{ id: c.id, kind: c.connectionType, from, to, waypoints: c.waypoints, flowing }]
  }), [connections, byId, widgetById, currentValues])

  const sendCommand = useCallback(async (sensorId: number, value: number) => {
    try {
      if (!wsWriteTag(sensorId, value)) await writeTag(sensorId, value)
      toast.success('Command sent')
      return true
    } catch (err: any) {
      toast.error(err.message || 'Write failed')
      return false
    }
  }, [])

  // KPI strip: the big numbers placed on the 2D screen.
  const kpis = widgets.filter(w => w.type === 'big_number' && w.sensorId != null).slice(0, 4)
  const header = widgets.find(w => w.type === 'label' && w.config?.bold)?.config?.text ?? screenName
  const selectedWidget = selected ? widgetById.get(selected) : undefined
  const counts = items.reduce((acc, it) => {
    if (it.def.binding !== 'state' || it.widget.sensorId == null) return acc
    const st = live(it).live.state
    acc[st] = (acc[st] ?? 0) + 1
    return acc
  }, {} as Record<string, number>)

  return (
    <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 520, overflow: 'hidden' }}>
      <div style={{ flex: 1, position: 'relative', minHeight: 420, background: '#10151b' }}>
        <Canvas
          shadows
          dpr={[1, 2]}
          gl={{ antialias: true }}
          camera={{ position: overview.pos, fov: 42, near: 0.1, far: 600 }}
          onCreated={({ camera }) => camera.lookAt(...overview.target)}
          onPointerMissed={() => setSelected(null)}
        >
          <TwinEnvironment size={Math.max(box.sx, box.sz, 20)} center={[box.cx, box.cz]} />
          <OrbitControls makeDefault target={overview.target} enableDamping dampingFactor={0.08} maxPolarAngle={1.42} minDistance={2.5} maxDistance={180}
            autoRotate={orbit} autoRotateSpeed={0.35} />
          <CameraRig goal={goal} onArrive={arrive} />
          {areas.map(a => <AreaPad key={a.id} a={a} />)}
          <Pipes3D runs={runs} />
          {items.map(it => {
            const l = live(it)
            return (
              <Equipment key={it.id} it={it} live={l.live} value={l.value} unit={l.unit} selected={selected === it.id} hovered={hovered === it.id}
                showLabels={showLabels} onSelect={select} onHover={setHovered} />
            )
          })}
        </Canvas>

        {/* Title + KPIs */}
        <div style={{ position: 'absolute', zIndex: 20, top: 12, left: 14, right: selectedWidget ? 350 : 14, display: 'flex', alignItems: 'flex-start', gap: 16, pointerEvents: 'none' }}>
          <div style={{ padding: '8px 12px', borderRadius: 6, background: 'rgba(13,15,18,.78)', border: '1px solid rgba(255,255,255,.08)', backdropFilter: 'blur(4px)' }}>
            <div style={{ fontSize: 16, fontWeight: 700, color: '#e8edf2' }}>{String(header).replace('Process Overview', '3D Plant Twin')}</div>
            <div style={{ marginTop: 3, display: 'flex', alignItems: 'center', gap: 6, fontSize: 11, color: '#8b95a1' }}>
              <span style={{ width: 7, height: 7, borderRadius: 4, background: 'var(--hmi-run)' }} /> live · {items.length} assets · {runs.filter(r => r.kind === 'flow').length} pipe runs
            </div>
          </div>
          <span style={{ flex: 1 }} />
          {kpis.map(w => {
            const s = sensorById.get(w.sensorId!)
            const d = currentValues[String(w.sensorId)]
            return (
              <div key={w.id} style={{ minWidth: 130, padding: '7px 12px', borderRadius: 6, background: 'rgba(13,15,18,.78)', border: '1px solid rgba(255,255,255,.08)', backdropFilter: 'blur(4px)' }}>
                <div style={{ fontSize: 9, fontWeight: 700, letterSpacing: '.08em', color: '#8b95a1', textTransform: 'uppercase', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', maxWidth: 170 }}>{s?.name}</div>
                <div style={{ fontFamily: 'var(--font-mono)', fontSize: 20, fontWeight: 700, color: '#e8edf2' }}>
                  {fmt(d, s) || '----'} <span style={{ fontSize: 11, color: '#8b95a1', fontWeight: 500 }}>{s?.unit}</span>
                </div>
              </div>
            )
          })}
        </div>

        {/* Area navigation + view toggles */}
        <div style={{ position: 'absolute', zIndex: 20, left: 14, top: 86, display: 'flex', flexDirection: 'column', gap: 4, width: 190 }}>
          <NavBtn onClick={() => { setSelected(null); setGoal({ ...overview }) }}>Plant overview</NavBtn>
          {areas.map(a => <NavBtn key={a.id} onClick={() => { setSelected(null); flyToArea(a) }}>{a.title}</NavBtn>)}
          <div style={{ height: 6 }} />
          <NavBtn active={showLabels} onClick={() => setShowLabels(v => !v)}>Tag plates</NavBtn>
          <NavBtn active={orbit} onClick={() => setOrbit(v => !v)}>Auto orbit</NavBtn>
          {onFreeLayout && <NavBtn onClick={onFreeLayout}>Free layout</NavBtn>}
        </div>

        {/* Legend */}
        <div style={{ position: 'absolute', zIndex: 20, left: 14, bottom: 12, display: 'flex', gap: 12, padding: '6px 10px', borderRadius: 6, background: 'rgba(13,15,18,.78)', border: '1px solid rgba(255,255,255,.08)', fontSize: 11, color: '#c4ccd4' }}>
          <Legend color="var(--hmi-run)">Running {counts.running ?? 0}</Legend>
          <Legend color="#7d8792">Stopped {counts.stopped ?? 0}</Legend>
          <Legend color="var(--hmi-fault)">Tripped {counts.fault ?? 0}</Legend>
          <Legend color="#4aa3ff">Flow</Legend>
          <span style={{ color: '#6b7480' }}>Drag to orbit · scroll to zoom · click equipment</span>
        </div>

        {selectedWidget && (
          <Faceplate
            key={selectedWidget.id}
            widget={selectedWidget}
            sensors={sensors}
            currentValues={currentValues}
            onCommand={sendCommand}
            onOpenDetail={onOpenDetail}
            onClose={() => setSelected(null)}
          />
        )}
      </div>
      <AlarmBanner />
    </div>
  )
}

function NavBtn({ children, onClick, active }: { children: React.ReactNode; onClick: () => void; active?: boolean }) {
  return (
    <button onClick={onClick} style={{
      textAlign: 'left', padding: '6px 10px', borderRadius: 5, fontSize: 12, fontWeight: 600, cursor: 'pointer',
      color: active ? '#22d3ee' : '#c4ccd4', background: active ? 'rgba(34,211,238,.12)' : 'rgba(13,15,18,.78)',
      border: `1px solid ${active ? 'rgba(34,211,238,.5)' : 'rgba(255,255,255,.08)'}`, backdropFilter: 'blur(4px)',
      whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
    }}>{children}</button>
  )
}

function Legend({ color, children }: { color: string; children: React.ReactNode }) {
  return <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5 }}><span style={{ width: 8, height: 8, borderRadius: 2, background: color }} />{children}</span>
}
