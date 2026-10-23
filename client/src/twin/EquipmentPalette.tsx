import { EquipmentType, EQUIPMENT_TYPES, TwinEquipment } from '../types'

interface Props {
  equipment: TwinEquipment[]
  onAdd: (type: EquipmentType) => void
  onRemove: (id: string) => void
}

export function EquipmentPalette({ equipment, onAdd, onRemove }: Props) {
  return (
    <div style={{
      position: 'absolute', top: 8, left: 8, zIndex: 10,
      background: 'rgba(15,15,20,0.9)', border: '1px solid rgba(255,255,255,0.1)',
      borderRadius: 8, padding: 12, width: 200, fontFamily: 'Inter, sans-serif',
    }}>
      <div style={{ fontSize: 11, fontWeight: 600, color: 'rgba(255,255,255,0.6)', textTransform: 'uppercase', letterSpacing: '0.05em', marginBottom: 8 }}>
        Equipment
      </div>
      {equipment.map(eq => (
        <div key={eq.id} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 4 }}>
          <span style={{ fontSize: 12, color: '#e5e7eb' }}>{eq.name}</span>
          <button onClick={() => onRemove(eq.id)} style={{ background: 'none', border: 'none', color: '#ef4444', cursor: 'pointer', fontSize: 14, lineHeight: 1 }}>&times;</button>
        </div>
      ))}
      <div style={{ borderTop: '1px solid rgba(255,255,255,0.1)', marginTop: 8, paddingTop: 8 }}>
        <div style={{ fontSize: 10, color: 'rgba(255,255,255,0.4)', marginBottom: 4 }}>Add equipment:</div>
        {EQUIPMENT_TYPES.map(et => (
          <button key={et.value} onClick={() => onAdd(et.value)} style={{
            display: 'block', width: '100%', padding: '4px 8px', marginBottom: 3,
            background: 'rgba(255,255,255,0.05)', border: '1px solid rgba(255,255,255,0.1)',
            borderRadius: 4, color: '#e5e7eb', fontSize: 11, cursor: 'pointer', textAlign: 'left',
          }}>
            + {et.label}
          </button>
        ))}
      </div>
      <div style={{ fontSize: 9, color: 'rgba(255,255,255,0.3)', marginTop: 8 }}>
        Drag to reposition. Scroll to rotate. Use "Connect Flow" to define electricity paths.
      </div>
    </div>
  )
}