import { Html } from '@react-three/drei'
import { SensorConfig, SensorData } from '../types'
import { stColor } from '../styles'

interface Props {
  name: string
  sensors: SensorConfig[]
  currentValues: Record<string, SensorData>
}

export function EquipmentLabel({ name, sensors, currentValues }: Props) {
  return (
    <Html position={[0, 2.0, 0]} center style={{ pointerEvents: 'none' }}>
      <div style={{
        background: 'rgba(15,15,20,0.92)',
        border: '1px solid rgba(255,255,255,0.1)',
        borderRadius: 8,
        padding: '8px 12px',
        fontFamily: 'Inter, sans-serif',
        minWidth: 140,
        pointerEvents: 'none',
      }}>
        <div style={{ fontSize: 11, fontWeight: 600, color: '#e5e7eb', marginBottom: sensors.length > 0 ? 4 : 0, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
          {name}
        </div>
        {sensors.map(s => {
          const cv = currentValues[s.id]
          const st = cv?.status ?? 'normal'
          return (
            <div key={s.id} style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 11, marginTop: 2 }}>
              <div style={{ width: 6, height: 6, borderRadius: '50%', background: st === 'critical' ? '#ef4444' : st === 'warning' ? '#eab308' : '#22c55e', flexShrink: 0 }} />
              <span style={{ color: 'rgba(255,255,255,0.5)', fontSize: 10 }}>{s.name}:</span>
              <span style={{ fontFamily: 'var(--font-mono)', fontWeight: 600, color: st === 'normal' ? s.color : st === 'critical' ? '#ef4444' : '#eab308' }}>
                {cv && cv.value != null && !Number.isNaN(cv.value) ? cv.value.toFixed(s.decimals) : 'OFFLINE'} {s.unit}
              </span>
            </div>
          )
        })}
      </div>
    </Html>
  )
}