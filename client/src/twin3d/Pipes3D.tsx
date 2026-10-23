import { useEffect, useMemo } from 'react'
import { useFrame } from '@react-three/fiber'
import { Line } from '@react-three/drei'
import * as THREE from 'three'
import { MAT, makeFlowTexture } from './materials'
import { PIPE_R, PIPE_Y } from './models'
import type { PlacedItem } from './placement'
import { toWorld } from './placement'

export type PipeKind = 'flow' | 'power' | 'data'

export interface PipeRun {
  id: string
  kind: PipeKind
  from: PlacedItem
  to: PlacedItem
  /** Explicit route from the 2D editor, stage px. */
  waypoints: [number, number][]
  flowing: boolean
}

const CABLE_Y = 2.45
type P2 = [number, number]

/** Point on the edge of an item's footprint, leaving towards `dir`. */
function edge(it: PlacedItem, dir: 'x+' | 'x-' | 'z+' | 'z-'): P2 {
  const { hx, hz } = it
  switch (dir) {
    case 'x+': return [it.x + hx, it.z]
    case 'x-': return [it.x - hx, it.z]
    case 'z+': return [it.x, it.z + hz]
    case 'z-': return [it.x, it.z - hz]
  }
}

/** Orthogonal plan route between two items, mirroring the 2D editor: straight, or one dog-leg at the midpoint. */
function planRoute(a: PlacedItem, b: PlacedItem, waypoints: [number, number][]): P2[] {
  if (waypoints.length) {
    const wps = waypoints.map(([x, y]) => toWorld(x, y))
    const first = wps[0]
    const last = wps[wps.length - 1]
    const out = (it: PlacedItem, p: P2): P2 =>
      Math.abs(p[0] - it.x) >= Math.abs(p[1] - it.z) ? edge(it, p[0] >= it.x ? 'x+' : 'x-') : edge(it, p[1] >= it.z ? 'z+' : 'z-')
    const s = out(a, first)
    const e = out(b, last)
    // Square off the first and last legs so the pipe leaves each nozzle straight.
    const lead: P2 = s[0] === a.x ? [s[0], first[1]] : [first[0], s[1]]
    const tail: P2 = e[0] === b.x ? [e[0], last[1]] : [last[0], e[1]]
    return dedupe([s, lead, ...wps, tail, e])
  }
  const dx = b.x - a.x
  const dz = b.z - a.z
  if (Math.abs(dx) >= Math.abs(dz)) {
    const s = edge(a, dx >= 0 ? 'x+' : 'x-')
    const e = edge(b, dx >= 0 ? 'x-' : 'x+')
    if (Math.abs(s[1] - e[1]) < 0.05) return [s, e]
    const mx = (s[0] + e[0]) / 2
    return [s, [mx, s[1]], [mx, e[1]], e]
  }
  const s = edge(a, dz >= 0 ? 'z+' : 'z-')
  const e = edge(b, dz >= 0 ? 'z-' : 'z+')
  if (Math.abs(s[0] - e[0]) < 0.05) return [s, e]
  const mz = (s[1] + e[1]) / 2
  return [s, [s[0], mz], [e[0], mz], e]
}

function dedupe(pts: P2[]): P2[] {
  const out: P2[] = []
  for (const p of pts) {
    const q = out[out.length - 1]
    if (!q || Math.hypot(p[0] - q[0], p[1] - q[1]) > 0.02) out.push(p)
  }
  // Drop collinear middles.
  return out.filter((p, i) => {
    if (i === 0 || i === out.length - 1) return true
    const a = out[i - 1], c = out[i + 1]
    return !((Math.abs(a[0] - p[0]) < 0.01 && Math.abs(p[0] - c[0]) < 0.01) || (Math.abs(a[1] - p[1]) < 0.01 && Math.abs(p[1] - c[1]) < 0.01))
  })
}

/** Polyline → curve with rounded corners. */
function roundedPath(pts: THREE.Vector3[], radius = 0.35): THREE.CurvePath<THREE.Vector3> {
  const path = new THREE.CurvePath<THREE.Vector3>()
  let cur = pts[0].clone()
  for (let i = 1; i < pts.length - 1; i++) {
    const p = pts[i]
    const inDir = p.clone().sub(pts[i - 1])
    const outDir = pts[i + 1].clone().sub(p)
    const r = Math.min(radius, inDir.length() / 2, outDir.length() / 2)
    const a = p.clone().sub(inDir.normalize().multiplyScalar(r))
    const b = p.clone().add(outDir.normalize().multiplyScalar(r))
    if (cur.distanceTo(a) > 1e-4) path.add(new THREE.LineCurve3(cur, a))
    path.add(new THREE.QuadraticBezierCurve3(a, p.clone(), b))
    cur = b
  }
  path.add(new THREE.LineCurve3(cur, pts[pts.length - 1].clone()))
  return path
}

const FLOW_PERIOD = 0.9 // metres per dash

function FlowPipe({ pts, flowing }: { pts: THREE.Vector3[]; flowing: boolean }) {
  const { geom, mat, length, sleepers } = useMemo(() => {
    const path = roundedPath(pts)
    const length = path.getLength()
    const geom = new THREE.TubeGeometry(path as unknown as THREE.Curve<THREE.Vector3>, Math.max(8, Math.ceil(length * 8)), PIPE_R, 12, false)
    const tex = makeFlowTexture()
    tex.repeat.set(length / FLOW_PERIOD, 1)
    const mat = MAT.steel.clone()
    mat.emissive = new THREE.Color('#4aa3ff')
    mat.emissiveMap = tex
    // Pipe sleepers about every 3 m along straight runs.
    const sleepers: THREE.Vector3[] = []
    for (let i = 1; i < pts.length; i++) {
      const a = pts[i - 1], b = pts[i]
      const n = Math.floor(a.distanceTo(b) / 3)
      for (let k = 1; k <= n; k++) sleepers.push(a.clone().lerp(b, k / (n + 1)))
    }
    return { geom, mat, length, sleepers }
  }, [pts])
  useEffect(() => () => { geom.dispose(); mat.emissiveMap?.dispose(); mat.dispose() }, [geom, mat])
  useFrame((_, dt) => {
    const target = flowing ? 1.6 : 0
    mat.emissiveIntensity += (target - mat.emissiveIntensity) * Math.min(1, dt * 3)
    if (flowing && mat.emissiveMap) mat.emissiveMap.offset.x -= (dt * 1.4) / FLOW_PERIOD
  })
  void length
  return (
    <group>
      <mesh geometry={geom} material={mat} castShadow receiveShadow />
      {sleepers.map((p, i) => (
        <mesh key={i} position={[p.x, (p.y - PIPE_R) / 2, p.z]} material={MAT.concreteLight} castShadow receiveShadow>
          <boxGeometry args={[0.25, Math.max(0.05, p.y - PIPE_R), 0.25]} />
        </mesh>
      ))}
    </group>
  )
}

function CableRun({ pts, live }: { pts: THREE.Vector3[]; live: boolean }) {
  const geom = useMemo(() => new THREE.TubeGeometry(roundedPath(pts, 0.25) as unknown as THREE.Curve<THREE.Vector3>, Math.max(8, pts.length * 12), 0.035, 6, false), [pts])
  useEffect(() => () => geom.dispose(), [geom])
  return <mesh geometry={geom} material={live ? MAT.cable : MAT.rubber} castShadow />
}

export function Pipes3D({ runs }: { runs: PipeRun[] }) {
  const built = useMemo(() => runs.map(run => {
    const plan = planRoute(run.from, run.to, run.waypoints)
    if (run.kind === 'flow') {
      return { run, pts: plan.map(([x, z]) => new THREE.Vector3(x, PIPE_Y, z)) }
    }
    if (run.kind === 'power') {
      // Cables rise to an overhead run, then drop to each end; busbar trays are tapped at their own height.
      const top = (it: PlacedItem) => (it.key === 'busbar' ? 2.6 : Math.min(it.top, CABLE_Y - 0.1))
      const [s, e] = [plan[0], plan[plan.length - 1]]
      const sx = run.from.key === 'busbar' ? [e[0], run.from.z] as P2 : [run.from.x, run.from.z] as P2
      const ex = run.to.key === 'busbar' ? [s[0], run.to.z] as P2 : [run.to.x, run.to.z] as P2
      const pts: THREE.Vector3[] = [new THREE.Vector3(sx[0], top(run.from), sx[1])]
      if (run.from.key !== 'busbar') pts.push(new THREE.Vector3(sx[0], CABLE_Y, sx[1]))
      if (Math.abs(sx[0] - ex[0]) > 0.05 && Math.abs(sx[1] - ex[1]) > 0.05) pts.push(new THREE.Vector3(ex[0], CABLE_Y, sx[1]))
      if (run.to.key !== 'busbar') pts.push(new THREE.Vector3(ex[0], CABLE_Y, ex[1]))
      pts.push(new THREE.Vector3(ex[0], top(run.to), ex[1]))
      return { run, pts: pts.filter((p, i) => i === 0 || p.distanceTo(pts[i - 1]) > 0.02) }
    }
    return { run, pts: [new THREE.Vector3(run.from.x, run.from.top * 0.8, run.from.z), new THREE.Vector3(run.to.x, run.to.top * 0.8, run.to.z)] }
  }), [runs])

  return (
    <group>
      {built.map(({ run, pts }) =>
        pts.length < 2 ? null
          : run.kind === 'flow' ? <FlowPipe key={run.id} pts={pts} flowing={run.flowing} />
          : run.kind === 'power' ? <CableRun key={run.id} pts={pts} live={run.flowing} />
          : <Line key={run.id} points={pts} color="#5b8fd1" lineWidth={1.5} dashed dashSize={0.25} gapSize={0.15} />,
      )}
    </group>
  )
}
