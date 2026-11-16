import { useState } from 'react'
import { getScreenWidget, screenWidgets } from '../edition/widgets'
import type { DashboardWidget } from '../dashboardWidgets'
import type { SensorConfig } from '../types'
import { useDashboardSelector } from '../store/dashboardStore'
import { ScreenWidgetView, useElementSize } from './ScreenWidgetView'

const DRAG_DOTS = (
  <svg width="8" height="12" viewBox="0 0 8 12">
    <circle cx="2" cy="2" r="1.2" fill="var(--fg)"/><circle cx="6" cy="2" r="1.2" fill="var(--fg)"/>
    <circle cx="2" cy="6" r="1.2" fill="var(--fg)"/><circle cx="6" cy="6" r="1.2" fill="var(--fg)"/>
    <circle cx="2" cy="10" r="1.2" fill="var(--fg)"/><circle cx="6" cy="10" r="1.2" fill="var(--fg)"/>
  </svg>
)

const smallBtn: React.CSSProperties = {
  background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 4, color: 'var(--fg)',
  cursor: 'pointer', padding: '2px 6px', fontSize: 10, lineHeight: 1,
}

/** An add-on widget placed on the card dashboard. */
export function DashboardWidgetCard({ widget, sensors, editMode, onChange, onRemove }: {
  widget: DashboardWidget
  sensors: SensorConfig[]
  editMode: boolean
  onChange: (config: Record<string, unknown>) => void
  onRemove: () => void
}) {
  const values = useDashboardSelector(s => s.currentValues)
  const { ref, width, height } = useElementSize<HTMLDivElement>()
  const [settingsOpen, setSettingsOpen] = useState(false)
  const def = getScreenWidget(widget.type, 'dashboard')
  const Editor = def?.configEditor

  return (
    <div style={{
      background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 12, overflow: 'hidden',
      height: '100%', display: 'flex', flexDirection: 'column', position: 'relative',
    }}>
      {editMode && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '4px 8px', borderBottom: '1px solid var(--border)' }}>
          <span className="drag-handle" style={{ cursor: 'grab', display: 'inline-flex', alignItems: 'center', gap: 6, flex: 1, minWidth: 0, opacity: 0.6 }}>
            {DRAG_DOTS}
            <span style={{ fontSize: 10, fontWeight: 600, textTransform: 'uppercase', letterSpacing: '.04em', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {def?.label ?? widget.type}
            </span>
          </span>
          {Editor && <button onClick={() => setSettingsOpen(v => !v)} style={smallBtn} aria-expanded={settingsOpen}>Settings</button>}
          <button onClick={onRemove} style={{ ...smallBtn, borderColor: 'var(--crit)', color: 'var(--crit)' }}>Remove</button>
        </div>
      )}
      <div ref={ref} style={{ flex: 1, minHeight: 0, position: 'relative' }}>
        <ScreenWidgetView
          type={widget.type} config={widget.config} surface="dashboard"
          width={width} height={height} editMode={editMode}
          sensors={sensors} values={values}
        />
      </div>
      {editMode && settingsOpen && Editor && (
        <div style={{ position: 'absolute', inset: '28px 0 0 0', background: 'var(--card)', overflow: 'auto', padding: 10, zIndex: 3 }}>
          <Editor config={widget.config} onChange={onChange} surface="dashboard" sensors={sensors} />
          <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: 8 }}>
            <button onClick={() => setSettingsOpen(false)} style={smallBtn}>Done</button>
          </div>
        </div>
      )}
    </div>
  )
}

/** "Add widget" picker for the dashboard's edit mode; renders nothing when no add-on widget is registered. */
export function AddDashboardWidget({ onAdd }: { onAdd: (type: string) => void }) {
  const defs = screenWidgets('dashboard')
  if (defs.length === 0) return null
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '8px 0' }}>
      <span style={{ fontSize: 12, color: 'var(--muted)' }}>Add widget</span>
      {defs.map(d => (
        <button key={d.type} onClick={() => onAdd(d.type)} style={{ ...smallBtn, fontSize: 12, padding: '5px 10px', display: 'inline-flex', alignItems: 'center', gap: 6 }}>
          {d.icon}{d.label}
        </button>
      ))}
    </div>
  )
}
