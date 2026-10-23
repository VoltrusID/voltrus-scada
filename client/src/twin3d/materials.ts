import * as THREE from 'three'
import type { EquipState } from '../scada2/hmi/symbols'

// Shared PBR materials. Module-level so forty pumps share one material each instead of forty.
const std = (color: string, metalness: number, roughness: number, extra: Partial<THREE.MeshStandardMaterialParameters> = {}) =>
  new THREE.MeshStandardMaterial({ color, metalness, roughness, ...extra })

export const MAT = {
  steel: std('#9aa3ad', 0.75, 0.32),
  steelDark: std('#4b535c', 0.7, 0.45),
  paint: std('#7d8792', 0.25, 0.55),          // ISA-101 grey equipment body
  paintDark: std('#3d444c', 0.3, 0.6),
  motor: std('#5d6f80', 0.35, 0.45),          // motor frames, a touch of blue-grey
  concrete: std('#5a5e63', 0.0, 0.92),
  concreteLight: std('#70757b', 0.0, 0.9),
  rubber: std('#16191c', 0.0, 0.85),
  yellow: std('#d6a01e', 0.2, 0.5),           // safety rails, hand wheels
  copper: std('#b87333', 0.85, 0.35),
  cable: std('#c9a23a', 0.1, 0.6),
  sand: std('#b9a175', 0.0, 0.95),
  gravel: std('#6e675d', 0.0, 0.95),
  bulk: std('#cdb68a', 0.0, 0.9),
  panelGlass: std('#1b3566', 0.6, 0.18),
  lcd: std('#0b1610', 0.1, 0.4, { emissive: new THREE.Color('#6ee7a8'), emissiveIntensity: 0.25 }),
  water: new THREE.MeshPhysicalMaterial({
    color: '#2f78c4', metalness: 0, roughness: 0.08, transmission: 0, transparent: true, opacity: 0.82,
    clearcoat: 1, clearcoatRoughness: 0.1,
  }),
  waterDeep: new THREE.MeshStandardMaterial({ color: '#1f5fa8', metalness: 0, roughness: 0.2, transparent: true, opacity: 0.9 }),
  // Tank shells are see-through so the liquid level reads from any angle.
  shell: new THREE.MeshPhysicalMaterial({
    color: '#c8d2dc', metalness: 0.2, roughness: 0.15, transparent: true, opacity: 0.22, depthWrite: false,
    side: THREE.DoubleSide, clearcoat: 1,
  }),
  ground: std('#23282e', 0, 0.95),
}

export const STATE_COLOR: Record<EquipState, string> = {
  running: '#34c47a', fault: '#f04646', stale: '#d65ce8', stopped: '#7d8792', idle: '#7d8792',
}

const lampCache = new Map<string, THREE.MeshStandardMaterial>()
/** Emissive lamp material per colour (cached). */
export function lamp(color: string, intensity = 1.6): THREE.MeshStandardMaterial {
  const k = `${color}:${intensity}`
  let m = lampCache.get(k)
  if (!m) {
    m = new THREE.MeshStandardMaterial({ color, emissive: new THREE.Color(color), emissiveIntensity: intensity, roughness: 0.4 })
    lampCache.set(k, m)
  }
  return m
}

/** Repeating dash texture for animated pipe flow; each pipe clones it to set its own repeat. */
export function makeFlowTexture(): THREE.CanvasTexture {
  const c = document.createElement('canvas')
  c.width = 64
  c.height = 8
  const g = c.getContext('2d')!
  g.fillStyle = '#000'
  g.fillRect(0, 0, 64, 8)
  const grad = g.createLinearGradient(0, 0, 64, 0)
  grad.addColorStop(0, 'rgba(0,0,0,1)')
  grad.addColorStop(0.55, 'rgba(255,255,255,1)')
  grad.addColorStop(0.75, 'rgba(255,255,255,1)')
  grad.addColorStop(1, 'rgba(0,0,0,1)')
  g.fillStyle = grad
  g.fillRect(0, 0, 64, 8)
  const t = new THREE.CanvasTexture(c)
  t.wrapS = THREE.RepeatWrapping
  t.wrapT = THREE.RepeatWrapping
  return t
}
