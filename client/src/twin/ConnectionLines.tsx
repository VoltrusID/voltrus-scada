import { useRef, useMemo, useEffect, useCallback } from 'react'
import { useFrame } from '@react-three/fiber'
import * as THREE from 'three'
import { TwinEquipment } from '../types'

interface FlowConnection {
  from: string
  to: string
}

interface Props {
  equipment: TwinEquipment[]
  connections: FlowConnection[]
}

interface Connection {
  from: THREE.Vector3
  to: THREE.Vector3
  color: string
  line: THREE.Line
}

const PARTICLES_PER_LINE = 5
const PARTICLE_SPEED = 2.0

export function ConnectionLines({ equipment, connections }: Props) {
  const meshRefs = useRef<(THREE.Mesh | null)[]>([])
  const connectionsRef = useRef<Connection[]>([])

  const eqMap = useMemo(() => new Map(equipment.map(e => [e.id, e])), [equipment])

  const renderedConnections = useMemo(() => {
    if (connections.length === 0) { connectionsRef.current = []; return [] }

    const allConns: Connection[] = []
    for (const conn of connections) {
      const fromEq = eqMap.get(conn.from)
      const toEq = eqMap.get(conn.to)
      if (!fromEq || !toEq) continue

      const from = new THREE.Vector3(fromEq.position[0], 0.5, fromEq.position[2])
      const to = new THREE.Vector3(toEq.position[0], 0.5, toEq.position[2])
      const geom = new THREE.BufferGeometry().setFromPoints([from, to])
      const mat = new THREE.LineBasicMaterial({ color: '#f59e0b', transparent: true, opacity: 0.2 })
      allConns.push({ from, to, color: '#f59e0b', line: new THREE.Line(geom, mat) })
    }

    connectionsRef.current = allConns
    meshRefs.current = new Array(allConns.length * PARTICLES_PER_LINE).fill(null)
    return allConns
  }, [equipment, connections, eqMap])

  useEffect(() => () => {
    connectionsRef.current.forEach(c => {
      c.line.geometry.dispose()
      ;(c.line.material as THREE.Material).dispose()
    })
  }, [])

  useEffect(() => {
    meshRefs.current.forEach((mesh, i) => {
      if (!mesh) return
      const connIdx = Math.floor(i / PARTICLES_PER_LINE)
      const pIdx = i % PARTICLES_PER_LINE
      const conn = connectionsRef.current[connIdx]
      if (!conn) return
      const t = pIdx / PARTICLES_PER_LINE
      mesh.position.copy(conn.from).lerp(conn.to, t)
      mesh.userData.t = t
      mesh.userData.connIdx = connIdx
    })
  }, [renderedConnections])

  const setMeshRef = useCallback((index: number) => (el: THREE.Mesh | null) => {
    meshRefs.current[index] = el
  }, [])

  useFrame((_, delta) => {
    const conns = connectionsRef.current
    for (let i = 0; i < meshRefs.current.length; i++) {
      const mesh = meshRefs.current[i]
      if (!mesh) continue
      const conn = conns[mesh.userData.connIdx]
      if (!conn) continue

      let t = mesh.userData.t + (delta * PARTICLE_SPEED) / conn.from.distanceTo(conn.to)
      if (t > 1) t -= 1
      mesh.userData.t = t
      mesh.position.copy(conn.from).lerp(conn.to, t)

      const mat = mesh.material as THREE.MeshBasicMaterial
      mat.opacity = 0.4 + 0.6 * Math.sin(t * Math.PI)
    }
  })

  if (renderedConnections.length === 0) return null

  return (
    <group>
      {renderedConnections.map((conn, ci) => (
        <group key={ci}>
          <primitive object={conn.line} />
          {Array.from({ length: PARTICLES_PER_LINE }).map((_, pi) => (
            <mesh key={pi} ref={setMeshRef(ci * PARTICLES_PER_LINE + pi)} position={[conn.from.x, conn.from.y, conn.from.z]} userData={{ t: pi / PARTICLES_PER_LINE, connIdx: ci }}>
              <sphereGeometry args={[0.05, 8, 8]} />
              <meshBasicMaterial color={conn.color} transparent opacity={0.8} />
            </mesh>
          ))}
        </group>
      ))}
    </group>
  )
}
