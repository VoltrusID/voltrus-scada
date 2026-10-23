import { btnStyle } from './styles'

type AlignOp = 'left' | 'center-h' | 'right' | 'top' | 'center-v' | 'bottom' | 'dist-h' | 'dist-v'

interface Props {
  selectedCount: number
  onAlign: (op: AlignOp) => void
}

const ops: { op: AlignOp; label: string; icon: string }[] = [
  { op: 'left', label: 'Align Left', icon: '⫷' },
  { op: 'center-h', label: 'Center H', icon: '⫿' },
  { op: 'right', label: 'Align Right', icon: '⫸' },
  { op: 'top', label: 'Align Top', icon: '⊤' },
  { op: 'center-v', label: 'Center V', icon: '⊞' },
  { op: 'bottom', label: 'Align Bottom', icon: '⊥' },
  { op: 'dist-h', label: 'Distribute H', icon: '↔' },
  { op: 'dist-v', label: 'Distribute V', icon: '↕' },
]

export function AlignmentToolbar({ selectedCount, onAlign }: Props) {
  if (selectedCount < 2) return null
  return (
    <div style={{
      display: 'flex', gap: 2, padding: '4px 8px',
      background: 'var(--card)', border: '1px solid var(--cyan)',
      borderRadius: 8, marginBottom: 8, alignItems: 'center',
    }}>
      <span style={{ fontSize: 10, color: 'var(--cyan)', fontWeight: 600, marginRight: 4, textTransform: 'uppercase', letterSpacing: '.05em' }}>
        {selectedCount} selected
      </span>
      {ops.map(({ op, label, icon }) => (
        <button
          key={op}
          title={label}
          onClick={() => onAlign(op)}
          style={{
            ...btnStyle, padding: '4px 8px', minWidth: 28, fontSize: 13,
            borderColor: 'var(--border)', borderRadius: 4,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}
        >
          {icon}
        </button>
      ))}
    </div>
  )
}

export type { AlignOp }
