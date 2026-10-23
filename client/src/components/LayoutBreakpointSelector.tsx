interface Props {
  layoutBreakpoint: string | null
  onChange: (bp: string | null) => void
}

export function LayoutBreakpointSelector({ layoutBreakpoint, onChange }: Props) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 12 }}>
      <span style={{ fontSize: 11, color: 'var(--muted)', fontWeight: 500, textTransform: 'uppercase', letterSpacing: '.05em' }}>Edit layout:</span>
      {(['lg', 'md', 'sm', 'xs'] as const).map((bp) => {
        const labels: Record<string, string> = { lg: 'Desktop (24col)', md: 'Tablet (12col)', sm: 'Mobile (4col)', xs: 'Small (2col)' }
        return (
          <button key={bp} onClick={() => onChange(layoutBreakpoint === bp ? null : bp)} style={{
            padding: '4px 10px', borderRadius: 6, fontSize: 11, fontWeight: 600, cursor: 'pointer',
            border: `1px solid ${layoutBreakpoint === bp ? 'var(--cyan)' : 'var(--border)'}`,
            background: layoutBreakpoint === bp ? 'rgba(34,197,94,.1)' : 'var(--card)',
            color: layoutBreakpoint === bp ? 'var(--cyan)' : 'var(--muted)',
          }}>{labels[bp]}</button>
        )
      })}
      {layoutBreakpoint && <span style={{ fontSize: 10, color: 'var(--cyan)', marginLeft: 4 }}>Editing {layoutBreakpoint.toUpperCase()} layout — drag/resize to customize</span>}
    </div>
  )
}
