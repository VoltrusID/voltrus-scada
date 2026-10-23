import { useLayoutEffect, useMemo, useRef } from 'react'
import { useFrame } from '@react-three/fiber'
import { RoundedBox, Sparkles } from '@react-three/drei'
import * as THREE from 'three'
import type { EquipState } from '../scada2/hmi/symbols'
import { MAT, STATE_COLOR, lamp } from './materials'

/** Height of process pipework above grade (m). Inline equipment (valves, meters) sits on it. */
export const PIPE_Y = 0.55
export const PIPE_R = 0.11

export interface ModelProps {
  /** Footprint along x and z, and a height hint (m). */
  w: number
  d: number
  h: number
  state: EquipState
  runState: EquipState
  /** 0..1 for level-bound vessels. */
  level: number | null
  value: string
  config: Record<string, any>
}

// ── Building blocks ──

/** Every mesh below casts and receives shadows. */
export function Shadowed({ children }: { children: React.ReactNode }) {
  const ref = useRef<THREE.Group>(null)
  useLayoutEffect(() => {
    ref.current?.traverse(o => {
      if ((o as THREE.Mesh).isMesh) { o.castShadow = true; o.receiveShadow = true }
    })
  })
  return <group ref={ref}>{children}</group>
}

/** Rotates its children about `axis` while active, easing to a stop when not. */
function Spin({ active, axis = 'y', speed = 8, children }: { active: boolean; axis?: 'x' | 'y' | 'z'; speed?: number; children: React.ReactNode }) {
  const ref = useRef<THREE.Group>(null)
  const v = useRef(0)
  useFrame((_, dt) => {
    v.current += ((active ? speed : 0) - v.current) * Math.min(1, dt * 2.5)
    if (ref.current) ref.current.rotation[axis] += v.current * dt
  })
  return <group ref={ref}>{children}</group>
}

/** Status beacon on a short mast: steady when running, pulsing when tripped. */
export function Beacon({ state, position }: { state: EquipState; position: [number, number, number] }) {
  const halo = useRef<THREE.Mesh>(null)
  const haloMat = useMemo(() => new THREE.MeshBasicMaterial({ color: STATE_COLOR.fault, transparent: true, opacity: 0, depthWrite: false }), [])
  useFrame(({ clock }) => {
    if (!halo.current) return
    const on = state === 'fault'
    const k = on ? (Math.sin(clock.elapsedTime * 7) + 1) / 2 : 0
    haloMat.opacity = on ? 0.15 + 0.35 * k : 0
    halo.current.scale.setScalar(1 + k * 0.9)
  })
  const color = STATE_COLOR[state]
  return (
    <group position={position}>
      <mesh position={[0, 0.18, 0]} material={MAT.steelDark}><cylinderGeometry args={[0.015, 0.015, 0.36, 8]} /></mesh>
      <mesh position={[0, 0.4, 0]} material={lamp(color, state === 'running' || state === 'fault' ? 2.2 : 0.3)}>
        <sphereGeometry args={[0.06, 16, 12]} />
      </mesh>
      <mesh ref={halo} position={[0, 0.4, 0]} material={haloMat}><sphereGeometry args={[0.12, 16, 12]} /></mesh>
    </group>
  )
}

function Flange({ x = 0, y = PIPE_Y, z = 0, r = PIPE_R * 1.7, axis = 'x' as 'x' | 'z' }) {
  return (
    <mesh position={[x, y, z]} rotation={axis === 'x' ? [0, 0, Math.PI / 2] : [Math.PI / 2, 0, 0]} material={MAT.steel}>
      <cylinderGeometry args={[r, r, 0.05, 20]} />
    </mesh>
  )
}

function PipeStub({ from, to, y = PIPE_Y, r = PIPE_R }: { from: number; to: number; y?: number; r?: number }) {
  const len = Math.abs(to - from)
  return (
    <mesh position={[(from + to) / 2, y, 0]} rotation={[0, 0, Math.PI / 2]} material={MAT.steel}>
      <cylinderGeometry args={[r, r, len, 16]} />
    </mesh>
  )
}

function Plinth({ w, d, h = 0.12 }: { w: number; d: number; h?: number }) {
  return <mesh position={[0, h / 2, 0]} material={MAT.concreteLight}><boxGeometry args={[w, h, d]} /></mesh>
}

/** Motor frame along x: finned body, fan cowl, terminal box. Returns its length. */
function MotorBody({ r, len, x, y, running }: { r: number; len: number; x: number; y: number; running: boolean }) {
  return (
    <group position={[x, y, 0]}>
      <mesh rotation={[0, 0, Math.PI / 2]} material={MAT.motor}><cylinderGeometry args={[r, r, len, 28]} /></mesh>
      {Array.from({ length: 7 }).map((_, i) => (
        <mesh key={i} position={[-len / 2 + (len * (i + 0.5)) / 7, 0, 0]} rotation={[0, 0, Math.PI / 2]} material={MAT.motor}>
          <cylinderGeometry args={[r * 1.07, r * 1.07, len * 0.04, 28]} />
        </mesh>
      ))}
      <mesh position={[-len / 2 - r * 0.25, 0, 0]} rotation={[0, 0, Math.PI / 2]} material={MAT.paintDark}>
        <cylinderGeometry args={[r * 0.98, r * 0.98, r * 0.5, 28]} />
      </mesh>
      {/* Cooling fan behind the cowl grille: visibly turns when the motor runs */}
      <group position={[-len / 2 - r * 0.52, 0, 0]}>
        <Spin active={running} axis="x" speed={18}>
          {[0, 1, 2, 3].map(i => (
            <mesh key={i} rotation={[(i * Math.PI) / 2, 0, 0]} position={[0, 0, 0]} material={MAT.steelDark}>
              <boxGeometry args={[0.01, r * 1.6, r * 0.22]} />
            </mesh>
          ))}
        </Spin>
      </group>
      <mesh position={[0, r * 1.05, 0]} material={MAT.motor}><boxGeometry args={[len * 0.32, r * 0.4, r * 0.7]} /></mesh>
      {[-1, 1].map(s => (
        <mesh key={s} position={[0, -r * 0.85, s * r * 0.75]} material={MAT.motor}><boxGeometry args={[len * 0.8, r * 0.3, r * 0.25]} /></mesh>
      ))}
    </group>
  )
}

/** Shaft coupling with a contrasting stripe so rotation is obvious. */
function Coupling({ x, y, r, running }: { x: number; y: number; r: number; running: boolean }) {
  return (
    <group position={[x, y, 0]}>
      <mesh material={lamp('#d6a01e', 0.05)} rotation={[0, 0, Math.PI / 2]}><cylinderGeometry args={[r * 1.35, r * 1.35, r * 1.8, 16, 1, true]} /></mesh>
      <Spin active={running} axis="x" speed={22}>
        <mesh rotation={[0, 0, Math.PI / 2]} material={MAT.steel}><cylinderGeometry args={[r * 0.7, r * 0.7, r * 1.5, 16]} /></mesh>
        <mesh position={[0, r * 0.7, 0]} material={lamp(running ? STATE_COLOR.running : '#9aa3ad', running ? 1.4 : 0.1)}>
          <boxGeometry args={[r * 1.5, r * 0.12, r * 0.25]} />
        </mesh>
      </Spin>
    </group>
  )
}

// ── Rotating equipment ──

export function PumpModel({ w, d, state }: ModelProps) {
  const s = Math.min(w, d) * 0.95
  const running = state === 'running'
  const r = s * 0.13
  const axisY = 0.14 + r * 1.25
  return (
    <Shadowed>
      <Plinth w={s * 1.15} d={s * 0.55} />
      <mesh position={[0, 0.16, 0]} material={MAT.paintDark}><boxGeometry args={[s * 1.05, 0.06, s * 0.42]} /></mesh>
      <MotorBody r={r} len={s * 0.42} x={-s * 0.2} y={axisY} running={running} />
      <Coupling x={s * 0.06} y={axisY} r={r * 0.45} running={running} />
      {/* Volute casing, end suction along +x, discharge up */}
      <group position={[s * 0.3, axisY, 0]}>
        <mesh rotation={[0, 0, Math.PI / 2]} material={MAT.paint}><cylinderGeometry args={[r * 1.35, r * 1.35, s * 0.14, 32]} /></mesh>
        <mesh position={[0, r * 1.2, 0]} material={MAT.paint}><cylinderGeometry args={[r * 0.5, r * 0.6, r * 1.2, 20]} /></mesh>
        <mesh position={[0, r * 1.85, 0]} material={MAT.steel}><cylinderGeometry args={[r * 0.75, r * 0.75, 0.04, 20]} /></mesh>
        <mesh position={[s * 0.12, 0, 0]} rotation={[0, 0, Math.PI / 2]} material={MAT.paint}><cylinderGeometry args={[r * 0.55, r * 0.55, s * 0.12, 20]} /></mesh>
        <mesh position={[s * 0.18, 0, 0]} rotation={[0, 0, Math.PI / 2]} material={MAT.steel}><cylinderGeometry args={[r * 0.8, r * 0.8, 0.04, 20]} /></mesh>
        <mesh position={[0, -axisY + 0.2 + (axisY - 0.2) / 2 - 0.02, 0]} material={MAT.paint}><boxGeometry args={[s * 0.1, axisY - 0.2, s * 0.22]} /></mesh>
      </group>
      <Beacon state={state} position={[-s * 0.5, 0.18, s * 0.18]} />
    </Shadowed>
  )
}

export function DosingPumpModel({ w, d, state }: ModelProps) {
  const s = Math.min(w, d) * 0.9
  const running = state === 'running'
  return (
    <Shadowed>
      <Plinth w={s} d={s * 0.8} />
      {/* Chemical day tank behind the pump */}
      <mesh position={[0, 0.12 + s * 0.35, -s * 0.18]} material={MAT.shell}><cylinderGeometry args={[s * 0.22, s * 0.22, s * 0.7, 24]} /></mesh>
      <mesh position={[0, 0.12 + s * 0.22, -s * 0.18]} material={lamp('#d9e36a', 0.15)}><cylinderGeometry args={[s * 0.2, s * 0.2, s * 0.44, 24]} /></mesh>
      <group position={[0, 0.12, s * 0.2]}>
        <MotorBody r={s * 0.07} len={s * 0.26} x={-s * 0.12} y={s * 0.12} running={running} />
        <mesh position={[s * 0.1, s * 0.12, 0]} material={MAT.paint}><boxGeometry args={[s * 0.14, s * 0.2, s * 0.16]} /></mesh>
        <mesh position={[s * 0.22, s * 0.12, 0]} rotation={[0, 0, Math.PI / 2]} material={lamp(running ? STATE_COLOR.running : '#9aa3ad', running ? 0.9 : 0.05)}>
          <cylinderGeometry args={[s * 0.07, s * 0.07, s * 0.08, 20]} />
        </mesh>
      </group>
      <Beacon state={state} position={[-s * 0.4, 0.12, s * 0.32]} />
    </Shadowed>
  )
}

export function BlowerModel({ w, d, state }: ModelProps) {
  const s = Math.min(w, d) * 0.95
  const running = state === 'running'
  return (
    <Shadowed>
      <Plinth w={s * 1.1} d={s * 0.6} />
      <MotorBody r={s * 0.12} len={s * 0.38} x={-s * 0.24} y={0.12 + s * 0.16} running={running} />
      <group position={[s * 0.2, 0.12 + s * 0.3, 0]}>
        <mesh rotation={[0, 0, Math.PI / 2]} material={MAT.paint}><cylinderGeometry args={[s * 0.3, s * 0.3, s * 0.2, 32]} /></mesh>
        <group position={[-s * 0.105, 0, 0]}>
          <Spin active={running} axis="x" speed={14}>
            {Array.from({ length: 8 }).map((_, i) => (
              <mesh key={i} rotation={[(i * Math.PI) / 4, 0, 0]} material={MAT.steelDark}>
                <boxGeometry args={[0.012, s * 0.5, s * 0.06]} />
              </mesh>
            ))}
          </Spin>
        </group>
        <mesh position={[0, s * 0.3, s * 0.1]} material={MAT.paint}><boxGeometry args={[s * 0.18, s * 0.3, s * 0.18]} /></mesh>
      </group>
      <Beacon state={state} position={[-s * 0.5, 0.12, s * 0.22]} />
    </Shadowed>
  )
}

export function MotorModel({ w, d, state }: ModelProps) {
  const s = Math.min(w, d * 1.6) * 0.8
  return (
    <Shadowed>
      <Plinth w={s * 0.9} d={s * 0.45} />
      <MotorBody r={s * 0.16} len={s * 0.55} x={0} y={0.14 + s * 0.2} running={state === 'running'} />
      <Beacon state={state} position={[s * 0.4, 0.12, s * 0.15]} />
    </Shadowed>
  )
}

export function CompressorModel({ w, d, state }: ModelProps) {
  const s = Math.min(w, d)
  return (
    <Shadowed>
      <Plinth w={s} d={s * 0.7} />
      <RoundedBox args={[s * 0.9, s * 0.6, s * 0.6]} radius={0.06} position={[0, 0.12 + s * 0.3, 0]} material={MAT.paint} />
      <mesh position={[s * 0.3, 0.12 + s * 0.45, s * 0.301]} material={MAT.lcd}><planeGeometry args={[s * 0.15, s * 0.1]} /></mesh>
      <Beacon state={state} position={[-s * 0.35, 0.12 + s * 0.6, 0]} />
    </Shadowed>
  )
}

export function ConveyorModel({ w, d, state }: ModelProps) {
  const len = w * 0.95
  const running = state === 'running'
  const tex = useMemo(() => {
    const c = document.createElement('canvas'); c.width = 64; c.height = 8
    const g = c.getContext('2d')!; g.fillStyle = '#1c1f22'; g.fillRect(0, 0, 64, 8); g.fillStyle = '#3a3f45'; g.fillRect(0, 0, 8, 8)
    const t = new THREE.CanvasTexture(c); t.wrapS = THREE.RepeatWrapping; t.repeat.set(len * 2, 1); return t
  }, [len])
  useFrame((_, dt) => { if (running) tex.offset.x -= dt * 0.8 })
  return (
    <Shadowed>
      {[-1, 1].map(s => (
        <mesh key={s} position={[0, 0.7, s * d * 0.25]} material={MAT.paintDark}><boxGeometry args={[len, 0.12, 0.05]} /></mesh>
      ))}
      <mesh position={[0, 0.78, 0]}><boxGeometry args={[len, 0.03, d * 0.45]} /><meshStandardMaterial map={tex} roughness={0.8} /></mesh>
      {Array.from({ length: Math.max(2, Math.round(len / 1.5)) }).map((_, i, a) => (
        <mesh key={i} position={[-len / 2 + (len * i) / (a.length - 1), 0.35, 0]} material={MAT.steelDark}><boxGeometry args={[0.06, 0.7, d * 0.5]} /></mesh>
      ))}
      <Beacon state={state} position={[-len / 2, 0.8, d * 0.3]} />
    </Shadowed>
  )
}

// ── Valves & inline instruments ──

export function ValveModel({ w, d, state, value, kind }: ModelProps & { kind: 'gate' | 'control' | 'check' }) {
  const s = Math.min(w, d) * 0.85
  const open = state === 'running'
  const ind = kind === 'check' ? '#9aa3ad' : open ? STATE_COLOR.running : state === 'fault' ? STATE_COLOR.fault : '#9aa3ad'
  const pos = Math.max(0, Math.min(100, parseFloat(value) || 0))
  return (
    <Shadowed>
      <mesh position={[0, PIPE_Y / 2, 0]} material={MAT.concreteLight}><boxGeometry args={[s * 0.3, PIPE_Y - PIPE_R, s * 0.3]} /></mesh>
      <PipeStub from={-s / 2} to={s / 2} />
      <Flange x={-s * 0.22} />
      <Flange x={s * 0.22} />
      <mesh position={[0, PIPE_Y, 0]} material={MAT.paint}><sphereGeometry args={[PIPE_R * 1.9, 24, 18]} /></mesh>
      {kind === 'gate' && (
        <group position={[0, PIPE_Y, 0]}>
          <mesh position={[0, PIPE_R * 2.6, 0]} material={MAT.paint}><cylinderGeometry args={[PIPE_R * 0.8, PIPE_R * 1.2, PIPE_R * 1.8, 16]} /></mesh>
          <mesh position={[0, PIPE_R * 4.6 + (open ? 0.12 : 0), 0]} material={MAT.steel}><cylinderGeometry args={[0.018, 0.018, PIPE_R * 3, 8]} /></mesh>
          <mesh position={[0, PIPE_R * 6 + (open ? 0.12 : 0), 0]} rotation={[Math.PI / 2, 0, 0]} material={MAT.yellow}><torusGeometry args={[PIPE_R * 1.7, 0.018, 8, 28]} /></mesh>
          <mesh position={[0, PIPE_R * 3.5, 0]} material={lamp(ind, open ? 1.5 : 0.1)}><cylinderGeometry args={[PIPE_R * 0.95, PIPE_R * 0.95, 0.04, 16]} /></mesh>
        </group>
      )}
      {kind === 'control' && (
        <group position={[0, PIPE_Y, 0]}>
          <mesh position={[0, PIPE_R * 3.2, 0]} material={MAT.steel}><cylinderGeometry args={[0.02, 0.02, PIPE_R * 3, 8]} /></mesh>
          {[-1, 1].map(i => <mesh key={i} position={[i * PIPE_R * 1.1, PIPE_R * 3.2, 0]} material={MAT.paintDark}><boxGeometry args={[0.03, PIPE_R * 3, 0.06]} /></mesh>)}
          <mesh position={[0, PIPE_R * 5, 0]} material={MAT.paint}><sphereGeometry args={[PIPE_R * 2.4, 28, 14, 0, Math.PI * 2, 0, Math.PI / 2]} /></mesh>
          <mesh position={[0, PIPE_R * 5, 0]} rotation={[Math.PI / 2, 0, 0]} material={MAT.paintDark}><cylinderGeometry args={[PIPE_R * 2.45, PIPE_R * 2.45, 0.04, 28]} /></mesh>
          {/* Position indicator travels with valve opening */}
          <mesh position={[PIPE_R * 1.3, PIPE_R * 2 + (pos / 100) * PIPE_R * 2.2, 0]} material={lamp('#6aa9e6', 1.2)}><boxGeometry args={[0.02, 0.06, 0.08]} /></mesh>
        </group>
      )}
      {kind === 'check' && (
        <mesh position={[0, PIPE_Y + PIPE_R * 2, 0]} material={MAT.paint}><boxGeometry args={[PIPE_R * 2.4, PIPE_R * 0.9, PIPE_R * 1.6]} /></mesh>
      )}
    </Shadowed>
  )
}

export function FlowMeterModel({ w, d }: ModelProps) {
  const s = Math.min(w, d * 1.4) * 0.8
  return (
    <Shadowed>
      <mesh position={[0, PIPE_Y / 2, 0]} material={MAT.concreteLight}><boxGeometry args={[s * 0.2, PIPE_Y - PIPE_R, s * 0.25]} /></mesh>
      <PipeStub from={-s / 2} to={s / 2} />
      <Flange x={-s * 0.2} r={PIPE_R * 2} />
      <Flange x={s * 0.2} r={PIPE_R * 2} />
      <mesh position={[0, PIPE_Y, 0]} rotation={[0, 0, Math.PI / 2]} material={MAT.motor}><cylinderGeometry args={[PIPE_R * 1.7, PIPE_R * 1.7, s * 0.34, 24]} /></mesh>
      <mesh position={[0, PIPE_Y + PIPE_R * 2.4, 0]} material={MAT.steelDark}><cylinderGeometry args={[0.035, 0.035, PIPE_R * 1.6, 10]} /></mesh>
      <mesh position={[0, PIPE_Y + PIPE_R * 3.6, 0]} rotation={[Math.PI / 2, 0, 0]} material={MAT.paint}><cylinderGeometry args={[PIPE_R * 1.2, PIPE_R * 1.2, PIPE_R * 1.2, 24]} /></mesh>
      <mesh position={[0, PIPE_Y + PIPE_R * 3.6, PIPE_R * 0.61]} material={MAT.lcd}><circleGeometry args={[PIPE_R * 0.8, 24]} /></mesh>
    </Shadowed>
  )
}

export function InstrumentModel({ state, config }: ModelProps) {
  const fn = String(config.tagFunction ?? 'PT').toUpperCase()
  const H = 1.25
  return (
    <Shadowed>
      <mesh position={[0, 0.02, 0]} material={MAT.steelDark}><boxGeometry args={[0.3, 0.04, 0.3]} /></mesh>
      <mesh position={[0, H / 2, 0]} material={MAT.steel}><cylinderGeometry args={[0.03, 0.03, H, 10]} /></mesh>
      <group position={[0, H + 0.12, 0]}>
        <mesh rotation={[Math.PI / 2, 0, 0]} material={fn.startsWith('A') ? MAT.paint : MAT.motor}><cylinderGeometry args={[0.13, 0.13, 0.12, 24]} /></mesh>
        <mesh position={[0, 0, 0.061]} material={MAT.lcd}><circleGeometry args={[0.09, 24]} /></mesh>
        <mesh position={[0, -0.16, 0]} material={MAT.steel}><cylinderGeometry args={[0.025, 0.025, 0.16, 10]} /></mesh>
      </group>
      <Beacon state={state === 'fault' ? 'fault' : 'idle'} position={[0.18, H - 0.3, 0]} />
    </Shadowed>
  )
}

// ── Vessels ──

/** Eases a level towards its target so tanks fill and drain smoothly. */
function useEasedLevel(level: number | null) {
  const v = useRef(level ?? 0)
  const ref = useRef<THREE.Group>(null)
  useFrame((_, dt) => {
    v.current += ((level ?? 0) - v.current) * Math.min(1, dt * 1.5)
    if (ref.current) ref.current.scale.y = Math.max(0.001, v.current)
  })
  return ref
}

function Ladder({ x, z, H }: { x: number; z: number; H: number }) {
  const rungs = Math.floor(H / 0.3)
  return (
    <group position={[x, 0, z]}>
      {[-0.18, 0.18].map(o => <mesh key={o} position={[0, H / 2, o]} material={MAT.yellow}><boxGeometry args={[0.04, H, 0.04]} /></mesh>)}
      {Array.from({ length: rungs }).map((_, i) => (
        <mesh key={i} position={[0, 0.3 + i * 0.3, 0]} material={MAT.yellow}><boxGeometry args={[0.03, 0.03, 0.36]} /></mesh>
      ))}
    </group>
  )
}

export function TankModel({ w, d, h, level, state, runState, kind }: ModelProps & { kind: 'tank' | 'mixer' | 'silo' }) {
  const r = Math.min(w, d) * 0.46
  const base = kind === 'silo' ? Math.max(1.2, h * 0.3) : 0.2
  const H = Math.max(1.6, h - base)
  const levelRef = useEasedLevel(level)
  const bulk = kind === 'silo'
  return (
    <Shadowed>
      {kind === 'silo' ? (
        <>
          {[[-1, -1], [-1, 1], [1, -1], [1, 1]].map(([a, b], i) => (
            <mesh key={i} position={[a * r * 0.7, base / 2, b * r * 0.7]} material={MAT.steelDark}><boxGeometry args={[0.12, base, 0.12]} /></mesh>
          ))}
          <mesh position={[0, base - r * 0.35, 0]} rotation={[Math.PI, 0, 0]} material={MAT.paint}><coneGeometry args={[r, r * 0.7, 32, 1, true]} /></mesh>
        </>
      ) : (
        <mesh position={[0, 0.1, 0]} material={MAT.concreteLight}><cylinderGeometry args={[r * 1.12, r * 1.15, 0.2, 40]} /></mesh>
      )}
      {/* Liquid / bulk contents, scaled to the live level */}
      <group position={[0, base, 0]} ref={levelRef}>
        <mesh position={[0, H / 2, 0]} material={bulk ? MAT.bulk : MAT.waterDeep}><cylinderGeometry args={[r * 0.97, r * 0.97, H, 40]} /></mesh>
      </group>
      <mesh position={[0, base + H / 2, 0]} material={MAT.shell}><cylinderGeometry args={[r, r, H, 40, 1, true]} /></mesh>
      {/* Shallow roof: a sphere cap whose rim meets the shell (radius r / sin 30°, centred below the top). */}
      <mesh position={[0, base + H - 2 * r * Math.cos(Math.PI / 6), 0]} material={MAT.paint}><sphereGeometry args={[2 * r, 48, 12, 0, Math.PI * 2, 0, Math.PI / 6]} /></mesh>
      {[0.02, 0.5, 1].map(f => (
        <mesh key={f} position={[0, base + H * f, 0]} rotation={[Math.PI / 2, 0, 0]} material={MAT.steel}><torusGeometry args={[r * 1.005, 0.03, 8, 48]} /></mesh>
      ))}
      {/* Vertical stiffeners keep the glass shell reading as a steel vessel */}
      {Array.from({ length: 8 }).map((_, i) => {
        const a = (i / 8) * Math.PI * 2
        return <mesh key={i} position={[Math.cos(a) * r, base + H / 2, Math.sin(a) * r]} material={MAT.steel}><boxGeometry args={[0.04, H, 0.04]} /></mesh>
      })}
      {kind !== 'silo' && <Ladder x={r + 0.12} z={0} H={base + H} />}
      <mesh position={[0, base + H + r * 0.16 + 0.5, 0]} rotation={[Math.PI / 2, 0, 0]} material={MAT.yellow}><torusGeometry args={[r * 0.45, 0.025, 6, 32]} /></mesh>
      {kind === 'mixer' && (
        <group position={[0, base + H, 0]}>
          <mesh position={[0, r * 0.35, 0]} material={MAT.motor}><cylinderGeometry args={[r * 0.2, r * 0.2, r * 0.45, 20]} /></mesh>
          <Spin active={runState === 'running'} axis="y" speed={5}>
            <mesh position={[0, -H * 0.42, 0]} material={MAT.steel}><cylinderGeometry args={[0.025, 0.025, H * 0.85, 8]} /></mesh>
            {[0, Math.PI / 2].map(a => (
              <mesh key={a} position={[0, -H * 0.8, 0]} rotation={[0, a, 0.35]} material={MAT.steel}><boxGeometry args={[r * 1.1, 0.03, 0.12]} /></mesh>
            ))}
          </Spin>
        </group>
      )}
      <Beacon state={state} position={[-r * 0.6, base + H + r * 0.1, 0]} />
    </Shadowed>
  )
}

export function HorizontalTankModel({ w, d, level, state }: ModelProps) {
  const r = Math.min(d * 0.45, w * 0.22)
  const L = w * 0.85
  const levelRef = useEasedLevel(level)
  const cy = 0.35 + r
  return (
    <Shadowed>
      {[-1, 1].map(s => <mesh key={s} position={[s * L * 0.3, (cy - r * 0.4) / 2, 0]} material={MAT.concreteLight}><boxGeometry args={[0.3, cy - r * 0.4, r * 1.6]} /></mesh>)}
      <group position={[0, cy - r, 0]} ref={levelRef}>
        <mesh position={[0, r, 0]} rotation={[0, 0, Math.PI / 2]} material={MAT.waterDeep}><boxGeometry args={[r * 1.8, L * 0.98, r * 1.4]} /></mesh>
      </group>
      <mesh position={[0, cy, 0]} rotation={[0, 0, Math.PI / 2]} material={MAT.shell}><cylinderGeometry args={[r, r, L, 40, 1, true]} /></mesh>
      {[-1, 1].map(s => <mesh key={s} position={[s * L / 2, cy, 0]} rotation={[0, 0, (s * Math.PI) / 2]} material={MAT.paint}><sphereGeometry args={[r, 32, 12, 0, Math.PI * 2, 0, Math.PI / 5]} /></mesh>)}
      <Beacon state={state} position={[0, cy + r, 0]} />
    </Shadowed>
  )
}

export function ClarifierModel({ w, d, runState, state }: ModelProps) {
  const R = Math.max(1.5, (w + d) / 4)
  const H = 1.7
  const turning = runState === 'running'
  return (
    <Shadowed>
      <mesh position={[0, H / 2, 0]} material={MAT.concrete}><cylinderGeometry args={[R, R, H, 64, 1, true]} /></mesh>
      <mesh position={[0, H / 2, 0]} material={MAT.concrete}><cylinderGeometry args={[R - 0.18, R - 0.18, H, 64, 1, true]} /></mesh>
      <mesh position={[0, H, 0]} rotation={[-Math.PI / 2, 0, 0]} material={MAT.concreteLight}><ringGeometry args={[R - 0.18, R, 64]} /></mesh>
      <mesh position={[0, H - 0.18, 0]} rotation={[-Math.PI / 2, 0, 0]} material={MAT.water}><circleGeometry args={[R - 0.18, 64]} /></mesh>
      {/* Effluent launder just inside the wall */}
      <mesh position={[0, H - 0.1, 0]} rotation={[Math.PI / 2, 0, 0]} material={MAT.concreteLight}><torusGeometry args={[R - 0.45, 0.07, 6, 64]} /></mesh>
      <mesh position={[0, H / 2 + 0.2, 0]} material={MAT.concreteLight}><cylinderGeometry args={[0.35, 0.4, H + 0.4, 24]} /></mesh>
      <mesh position={[0, H - 0.25, 0]} material={MAT.shell}><cylinderGeometry args={[R * 0.25, R * 0.25, 0.9, 32, 1, true]} /></mesh>
      {/* Rotating bridge with drive head: turns while the rake drive runs */}
      <group position={[0, H + 0.45, 0]}>
        <mesh material={MAT.motor}><cylinderGeometry args={[0.32, 0.32, 0.35, 24]} /></mesh>
        <Spin active={turning} axis="y" speed={0.35}>
          <mesh position={[R / 2, 0.05, 0]} material={MAT.paint}><boxGeometry args={[R, 0.22, 0.7]} /></mesh>
          {[-0.35, 0.35].map(z => <mesh key={z} position={[R / 2, 0.4, z]} material={MAT.yellow}><boxGeometry args={[R, 0.035, 0.035]} /></mesh>)}
          {Array.from({ length: Math.ceil(R / 0.8) }).map((_, i) => [-0.35, 0.35].map(z => (
            <mesh key={`${i}${z}`} position={[0.4 + i * 0.8, 0.22, z]} material={MAT.yellow}><boxGeometry args={[0.03, 0.36, 0.03]} /></mesh>
          )))}
          <mesh position={[R - 0.1, -0.2, 0]} material={MAT.rubber}><boxGeometry args={[0.25, 0.25, 0.35]} /></mesh>
          <mesh position={[R * 0.55, -0.9, 0]} rotation={[0, 0, 0.2]} material={MAT.steelDark}><boxGeometry args={[R * 0.9, 0.05, 0.05]} /></mesh>
        </Spin>
      </group>
      <Beacon state={state} position={[0.3, H + 0.62, 0.3]} />
    </Shadowed>
  )
}

export function GravityFilterModel({ w, d, runState }: ModelProps) {
  const W = w * 0.92
  const D = Math.max(d, w * 0.6) * 0.92
  const H = 1.9
  const t = 0.16
  const washing = runState === 'running'
  const surf = useRef<THREE.Mesh>(null)
  useFrame(({ clock }) => {
    if (!surf.current) return
    const target = washing ? H - 0.2 : H - 0.45
    surf.current.position.y += (target - surf.current.position.y) * 0.05
    surf.current.rotation.z = washing ? Math.sin(clock.elapsedTime * 9) * 0.015 : 0
  })
  return (
    <Shadowed>
      {/* Concrete cell */}
      <mesh position={[0, 0.06, 0]} material={MAT.concrete}><boxGeometry args={[W, 0.12, D]} /></mesh>
      {[[0, -D / 2 + t / 2, W, t], [0, D / 2 - t / 2, W, t]].map(([x, z, a, b], i) => (
        <mesh key={i} position={[x, H / 2, z]} material={MAT.concrete}><boxGeometry args={[a, H, b]} /></mesh>
      ))}
      {[-1, 1].map(s => <mesh key={s} position={[s * (W / 2 - t / 2), H / 2, 0]} material={MAT.concrete}><boxGeometry args={[t, H, D - 2 * t]} /></mesh>)}
      {/* Media: gravel, sand, then water above */}
      <mesh position={[0, 0.12 + 0.2, 0]} material={MAT.gravel}><boxGeometry args={[W - 2 * t, 0.4, D - 2 * t]} /></mesh>
      <mesh position={[0, 0.52 + 0.3, 0]} material={MAT.sand}><boxGeometry args={[W - 2 * t, 0.6, D - 2 * t]} /></mesh>
      <mesh ref={surf} position={[0, H - 0.45, 0]} rotation={[-Math.PI / 2, 0, 0]} material={MAT.water}><planeGeometry args={[W - 2 * t, D - 2 * t]} /></mesh>
      <mesh position={[0, (H - 0.45 + 1.12) / 2, 0]} material={MAT.waterDeep}><boxGeometry args={[W - 2 * t - 0.01, H - 0.45 - 1.12, D - 2 * t - 0.01]} /></mesh>
      {washing && (
        <Sparkles count={70} scale={[W - 0.4, 0.8, D - 0.4]} position={[0, H - 0.5, 0]} size={4} speed={2.5} opacity={0.9} color="#e6f6ff" />
      )}
      {/* Gallery walkway with handrail along one long side */}
      <mesh position={[0, H + 0.02, D / 2 + 0.35]} material={MAT.steelDark}><boxGeometry args={[W, 0.05, 0.7]} /></mesh>
      {[0.5, 1].map(y => <mesh key={y} position={[0, H + y, D / 2 + 0.68]} material={MAT.yellow}><boxGeometry args={[W, 0.035, 0.035]} /></mesh>)}
      {Array.from({ length: Math.max(2, Math.round(W / 1)) + 1 }).map((_, i, a) => (
        <mesh key={i} position={[-W / 2 + (W * i) / (a.length - 1), H + 0.5, D / 2 + 0.68]} material={MAT.yellow}><boxGeometry args={[0.035, 1, 0.035]} /></mesh>
      ))}
      <mesh position={[W / 2 - 0.3, H + 0.3, -D / 2 + 0.05]} material={lamp(washing ? '#6aa9e6' : '#3d444c', washing ? 2 : 0.1)}><boxGeometry args={[0.3, 0.12, 0.08]} /></mesh>
    </Shadowed>
  )
}

export function HeatExchangerModel({ w, d, state }: ModelProps) {
  const L = w * 0.85
  const r = Math.min(d * 0.35, 0.45)
  return (
    <Shadowed>
      {[-1, 1].map(s => <mesh key={s} position={[s * L * 0.3, 0.3, 0]} material={MAT.paintDark}><boxGeometry args={[0.2, 0.6, r * 2]} /></mesh>)}
      <mesh position={[0, 0.6 + r * 0.6, 0]} rotation={[0, 0, Math.PI / 2]} material={MAT.paint}><cylinderGeometry args={[r, r, L, 32]} /></mesh>
      {[-1, 1].map(s => <mesh key={s} position={[s * (L / 2 + 0.05), 0.6 + r * 0.6, 0]} rotation={[0, 0, Math.PI / 2]} material={MAT.steel}><cylinderGeometry args={[r * 1.15, r * 1.15, 0.1, 32]} /></mesh>)}
      <Beacon state={state} position={[0, 0.6 + r * 1.6, 0]} />
    </Shadowed>
  )
}

export function CartridgeFilterModel({ w, d, state }: ModelProps) {
  const r = Math.min(w, d) * 0.3
  return (
    <Shadowed>
      {[0, 1, 2].map(i => {
        const a = (i / 3) * Math.PI * 2
        return <mesh key={i} position={[Math.cos(a) * r * 0.8, 0.3, Math.sin(a) * r * 0.8]} material={MAT.steelDark}><cylinderGeometry args={[0.03, 0.03, 0.6, 8]} /></mesh>
      })}
      <mesh position={[0, 0.6 + r * 1.4, 0]} material={MAT.paint}><cylinderGeometry args={[r, r, r * 2.8, 32]} /></mesh>
      <mesh position={[0, 0.6 + r * 2.8, 0]} material={MAT.steel}><sphereGeometry args={[r, 32, 12, 0, Math.PI * 2, 0, Math.PI / 2]} /></mesh>
      <Beacon state={state} position={[r, 0.6 + r * 2.8, 0]} />
    </Shadowed>
  )
}

// ── Electrical ──

export function MccSectionModel({ w, state }: ModelProps) {
  const W = Math.min(0.9, w * 0.8)
  const closed = state === 'running'
  return (
    <Shadowed>
      <mesh position={[0, 0.05, 0]} material={MAT.concreteLight}><boxGeometry args={[W + 0.1, 0.1, 0.8]} /></mesh>
      <RoundedBox args={[W, 2.1, 0.6]} radius={0.02} position={[0, 1.15, 0]} material={MAT.paint} />
      <mesh position={[0, 1.2, 0.301]} material={MAT.paintDark}><planeGeometry args={[W * 0.85, 1.8]} /></mesh>
      <mesh position={[W * 0.3, 1.2, 0.33]} material={MAT.steel}><boxGeometry args={[0.04, 0.25, 0.05]} /></mesh>
      {/* Breaker handle: up when closed */}
      <mesh position={[0, 1.55, 0.33]} rotation={[0, 0, closed ? 0 : 0.9]} material={MAT.rubber}><boxGeometry args={[0.05, 0.2, 0.05]} /></mesh>
      <mesh position={[-W * 0.25, 1.85, 0.31]} material={lamp(closed ? STATE_COLOR.running : '#3d444c', closed ? 2 : 0.1)}><circleGeometry args={[0.04, 16]} /></mesh>
      <mesh position={[-W * 0.1, 1.85, 0.31]} material={lamp(state === 'fault' ? STATE_COLOR.fault : '#3d444c', state === 'fault' ? 2 : 0.1)}><circleGeometry args={[0.04, 16]} /></mesh>
      <mesh position={[0, 0.7, 0.31]} material={MAT.lcd}><planeGeometry args={[W * 0.4, 0.14]} /></mesh>
    </Shadowed>
  )
}

export function TransformerModel({ w, d, state }: ModelProps) {
  const s = Math.max(1.6, Math.min(w, d) * 1.1)
  const on = state === 'running'
  return (
    <Shadowed>
      <mesh position={[0, 0.1, 0]} material={MAT.concreteLight}><boxGeometry args={[s * 1.3, 0.2, s]} /></mesh>
      <RoundedBox args={[s * 0.8, s * 0.9, s * 0.55]} radius={0.04} position={[0, 0.2 + s * 0.45, 0]} material={MAT.paint} />
      {/* Radiator banks */}
      {[-1, 1].map(side => Array.from({ length: 7 }).map((_, i) => (
        <mesh key={`${side}${i}`} position={[-s * 0.3 + (i * s * 0.6) / 6, 0.2 + s * 0.42, side * s * 0.36]} material={MAT.paint}>
          <boxGeometry args={[0.03, s * 0.7, s * 0.16]} />
        </mesh>
      )))}
      {[-1, 0, 1].map(i => (
        <group key={i} position={[i * s * 0.22, 0.2 + s * 0.9, 0]}>
          {Array.from({ length: 5 }).map((_, k) => (
            <mesh key={k} position={[0, 0.06 + k * 0.07, 0]} material={lamp('#c9c2b4', 0.02)}><cylinderGeometry args={[0.07, 0.07, 0.03, 16]} /></mesh>
          ))}
          <mesh position={[0, 0.2, 0]} material={MAT.copper}><cylinderGeometry args={[0.025, 0.025, 0.42, 8]} /></mesh>
        </group>
      ))}
      <mesh position={[s * 0.5, 0.2 + s * 0.35, 0]} material={MAT.paintDark}><boxGeometry args={[s * 0.25, s * 0.5, s * 0.4]} /></mesh>
      <Beacon state={on ? 'running' : state} position={[-s * 0.45, 0.2 + s * 0.9, s * 0.2]} />
    </Shadowed>
  )
}

export function BusbarTrayModel({ w, d, state }: ModelProps) {
  const L = Math.max(w, d)
  const along = w >= d
  const on = state === 'running'
  const Y = 2.6
  const posts = Math.max(2, Math.round(L / 3) + 1)
  return (
    <Shadowed>
      <group rotation={[0, along ? 0 : Math.PI / 2, 0]}>
        <mesh position={[0, Y, 0]} material={MAT.steelDark}><boxGeometry args={[L, 0.12, 0.5]} /></mesh>
        <mesh position={[0, Y + 0.08, 0]} material={lamp(on ? '#c9a23a' : '#5a5e63', on ? 1.1 : 0.05)}><boxGeometry args={[L, 0.04, 0.3]} /></mesh>
        {Array.from({ length: posts }).map((_, i) => (
          <mesh key={i} position={[-L / 2 + (L * i) / (posts - 1), Y / 2, 0]} material={MAT.steelDark}><boxGeometry args={[0.1, Y, 0.1]} /></mesh>
        ))}
      </group>
    </Shadowed>
  )
}

export function GeneratorModel({ w, d, state }: ModelProps) {
  const s = Math.min(w, d)
  return (
    <Shadowed>
      <Plinth w={s * 1.2} d={s * 0.7} h={0.15} />
      <RoundedBox args={[s * 1.1, s * 0.6, s * 0.6]} radius={0.05} position={[0, 0.15 + s * 0.3, 0]} material={MAT.paint} />
      <mesh position={[s * 0.35, 0.15 + s * 0.8, 0]} material={MAT.steelDark}><cylinderGeometry args={[0.08, 0.08, s * 0.5, 12]} /></mesh>
      <Beacon state={state} position={[-s * 0.45, 0.15 + s * 0.6, 0]} />
    </Shadowed>
  )
}

export function MeterModel({ value }: ModelProps) {
  return (
    <Shadowed>
      <mesh position={[0, 0.7, 0]} material={MAT.steel}><cylinderGeometry args={[0.04, 0.04, 1.4, 10]} /></mesh>
      <RoundedBox args={[0.6, 0.6, 0.2]} radius={0.03} position={[0, 1.55, 0]} material={MAT.paint} />
      <mesh position={[0, 1.6, 0.101]} material={value ? lamp('#6ee7a8', 0.6) : MAT.lcd}><planeGeometry args={[0.44, 0.22]} /></mesh>
    </Shadowed>
  )
}

export function SolarArrayModel({ w, d, state }: ModelProps) {
  const W = Math.max(2.2, w * 0.9)
  const D = Math.max(1.4, d * 0.8)
  return (
    <Shadowed>
      {[-1, 1].map(sx => [-1, 1].map(sz => (
        <mesh key={`${sx}${sz}`} position={[sx * W * 0.38, sz > 0 ? 0.45 : 0.85, sz * D * 0.3]} material={MAT.steel}>
          <boxGeometry args={[0.06, sz > 0 ? 0.9 : 1.7, 0.06]} />
        </mesh>
      )))}
      <group position={[0, 1.15, 0]} rotation={[0.45, 0, 0]}>
        <mesh material={MAT.steel}><boxGeometry args={[W, 0.05, D]} /></mesh>
        {Array.from({ length: 2 }).map((_, r) => Array.from({ length: 4 }).map((_, c) => (
          <mesh key={`${r}${c}`} position={[-W * 0.375 + c * W * 0.25, 0.03, -D * 0.25 + r * D * 0.5]} material={MAT.panelGlass}>
            <boxGeometry args={[W * 0.24, 0.02, D * 0.48]} />
          </mesh>
        )))}
      </group>
      <Beacon state={state} position={[W * 0.45, 0.2, D * 0.3]} />
    </Shadowed>
  )
}

export function CabinetModel({ w, d, state, screen }: ModelProps & { screen?: boolean }) {
  const W = Math.min(1.1, Math.max(0.6, w * 0.6))
  const D = Math.min(0.7, Math.max(0.4, d * 0.4))
  return (
    <Shadowed>
      <mesh position={[0, 0.05, 0]} material={MAT.concreteLight}><boxGeometry args={[W + 0.2, 0.1, D + 0.2]} /></mesh>
      <RoundedBox args={[W, 1.7, D]} radius={0.03} position={[0, 0.95, 0]} material={MAT.paint} />
      {Array.from({ length: 6 }).map((_, i) => (
        <mesh key={i} position={[-W / 2 - 0.02, 0.5 + i * 0.18, 0]} material={MAT.steelDark}><boxGeometry args={[0.04, 0.1, D * 0.8]} /></mesh>
      ))}
      {screen && <mesh position={[0, 1.35, D / 2 + 0.001]} material={MAT.lcd}><planeGeometry args={[W * 0.55, 0.28]} /></mesh>}
      <Beacon state={state} position={[W * 0.3, 1.8, 0]} />
    </Shadowed>
  )
}

export function BatteryModel({ w, d, level, state }: ModelProps) {
  const W = Math.max(2.4, w * 0.9)
  const D = Math.max(1.2, d * 0.8)
  const lv = level ?? 0
  const col = lv < 0.2 ? STATE_COLOR.fault : lv < 0.4 ? '#f5b21b' : STATE_COLOR.running
  return (
    <Shadowed>
      <mesh position={[0, 0.1, 0]} material={MAT.concreteLight}><boxGeometry args={[W + 0.3, 0.2, D + 0.3]} /></mesh>
      <RoundedBox args={[W, 1.6, D]} radius={0.04} position={[0, 1, 0]} material={MAT.paint} />
      {[0.25, 0.5, 0.75].map(f => <mesh key={f} position={[-W / 2 + W * f, 1, D / 2 + 0.005]} material={MAT.paintDark}><planeGeometry args={[0.02, 1.4]} /></mesh>)}
      <mesh position={[0, 1.95, 0]} material={MAT.steelDark}><boxGeometry args={[W * 0.3, 0.3, D * 0.6]} /></mesh>
      {/* State-of-charge bar on the door */}
      <mesh position={[-W * 0.3, 1.5, D / 2 + 0.01]} material={MAT.rubber}><planeGeometry args={[W * 0.3, 0.12]} /></mesh>
      <mesh position={[-W * 0.3 - W * 0.15 * (1 - lv), 1.5, D / 2 + 0.012]} scale={[Math.max(0.001, lv), 1, 1]} material={lamp(col, 1.4)}><planeGeometry args={[W * 0.3, 0.1]} /></mesh>
      <Beacon state={state} position={[W * 0.4, 1.8, 0]} />
    </Shadowed>
  )
}

/** Straight pipe spool (from the 2D pipe symbols). */
export function PipeSpoolModel({ w, d, runState }: ModelProps) {
  const L = Math.max(w, d)
  const along = w >= d
  return (
    <Shadowed>
      <group rotation={[0, along ? 0 : Math.PI / 2, 0]}>
        <PipeStub from={-L / 2} to={L / 2} />
        {runState === 'running' && <mesh position={[0, PIPE_Y, 0]} rotation={[0, 0, Math.PI / 2]} material={lamp('#6aa9e6', 0.5)}><cylinderGeometry args={[PIPE_R * 1.02, PIPE_R * 1.02, L * 0.98, 16, 1, true]} /></mesh>}
      </group>
    </Shadowed>
  )
}

export function GenericModel({ w, d }: ModelProps) {
  return (
    <Shadowed>
      <RoundedBox args={[w * 0.7, 0.8, d * 0.7]} radius={0.05} position={[0, 0.4, 0]} material={MAT.paint} />
    </Shadowed>
  )
}
