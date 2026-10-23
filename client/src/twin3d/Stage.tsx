import { useEffect, useRef } from 'react'
import { useFrame, useThree } from '@react-three/fiber'
import { Environment, Lightformer } from '@react-three/drei'
import * as THREE from 'three'
import { MAT } from './materials'

/** Lighting, sky, ground and fog shared by the plant twin and the free-layout twin. */
export function TwinEnvironment({ size = 60, center = [0, 0] as [number, number] }: { size?: number; center?: [number, number] }) {
  const sun = useRef<THREE.DirectionalLight>(null)
  const half = size * 0.75
  useEffect(() => {
    const l = sun.current
    if (!l) return
    l.target.position.set(center[0], 0, center[1])
    l.target.updateMatrixWorld()
  }, [center])
  return (
    <>
      <color attach="background" args={['#10151b']} />
      <fog attach="fog" args={['#10151b', size * 1.2, size * 3.2]} />
      <hemisphereLight args={['#c9d6e6', '#20262d', 0.55]} />
      <directionalLight
        ref={sun}
        position={[center[0] + size * 0.5, size * 0.9, center[1] + size * 0.35]}
        intensity={2.4}
        color="#fff4e2"
        castShadow
        shadow-mapSize={[4096, 4096]}
        shadow-bias={-0.0004}
        shadow-normalBias={0.03}
        shadow-camera-left={-half}
        shadow-camera-right={half}
        shadow-camera-top={half}
        shadow-camera-bottom={-half}
        shadow-camera-near={1}
        shadow-camera-far={size * 3}
      />
      {/* Studio-style reflections for the metals, generated locally (no HDR download). */}
      <Environment resolution={256} frames={1}>
        <Lightformer intensity={1.6} position={[0, 12, 0]} rotation-x={Math.PI / 2} scale={[40, 40, 1]} color="#dfe8f2" />
        <Lightformer intensity={0.8} position={[-20, 4, 0]} rotation-y={Math.PI / 2} scale={[30, 6, 1]} color="#9fb3c8" />
        <Lightformer intensity={0.6} position={[20, 4, 10]} rotation-y={-Math.PI / 2} scale={[30, 6, 1]} color="#ffe1b8" />
      </Environment>
      <mesh rotation={[-Math.PI / 2, 0, 0]} position={[center[0], -0.005, center[1]]} receiveShadow material={MAT.ground}>
        <planeGeometry args={[size * 6, size * 6]} />
      </mesh>
    </>
  )
}

export interface CameraGoal { pos: [number, number, number]; target: [number, number, number] }

/** Eases the camera and orbit target towards a goal; any user drag cancels the flight. */
export function CameraRig({ goal, onArrive }: { goal: CameraGoal | null; onArrive: () => void }) {
  const controls = useThree(s => s.controls) as any
  const camera = useThree(s => s.camera)
  const active = useRef<CameraGoal | null>(null)
  useEffect(() => { active.current = goal }, [goal])
  useEffect(() => {
    if (!controls) return
    const cancel = () => { if (active.current) { active.current = null; onArrive() } }
    controls.addEventListener('start', cancel)
    return () => controls.removeEventListener('start', cancel)
  }, [controls, onArrive])
  const p = useRef(new THREE.Vector3())
  const t = useRef(new THREE.Vector3())
  useFrame((_, dt) => {
    const g = active.current
    if (!g || !controls) return
    const k = 1 - Math.pow(0.02, dt)
    p.current.set(...g.pos)
    t.current.set(...g.target)
    camera.position.lerp(p.current, k)
    controls.target.lerp(t.current, k)
    controls.update()
    if (camera.position.distanceTo(p.current) < 0.05 && controls.target.distanceTo(t.current) < 0.05) {
      active.current = null
      onArrive()
    }
  })
  return null
}
