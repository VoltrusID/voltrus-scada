import { useState } from 'react'
import { useMaintenanceLogs, useCreateMaintenanceLog } from '../queries/assets'
import { inputStyle, labelStyle, labelTextStyle, btnStyle } from '../styles'

interface Props {
  assetId: string
}

export function MaintenanceTimeline({ assetId }: Props) {
  const { data: logs, isLoading } = useMaintenanceLogs(assetId)
  const createLog = useCreateMaintenanceLog(assetId)
  const [showForm, setShowForm] = useState(false)
  const [description, setDescription] = useState('')
  const [technician, setTechnician] = useState('')
  const [performedAt, setPerformedAt] = useState('')

  const handleSubmit = () => {
    if (!description.trim() || !technician.trim()) return
    createLog.mutate({
      description: description.trim(),
      technician: technician.trim(),
      performed_at: performedAt || new Date().toISOString().slice(0, 10),
    }, {
      onSuccess: () => {
        setDescription('')
        setTechnician('')
        setPerformedAt('')
        setShowForm(false)
      },
    })
  }

  if (isLoading) {
    return (
      <div style={{ padding: 24, color: 'var(--muted)', fontSize: 13 }}>
        Loading maintenance history...
      </div>
    )
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      {/* Header row */}
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--fg)' }}>
          Maintenance History
          {logs && (
            <span style={{ marginLeft: 8, fontSize: 11, color: 'var(--muted)', fontWeight: 400 }}>
              {logs.length} {logs.length === 1 ? 'entry' : 'entries'}
            </span>
          )}
        </div>
        <button
          style={{ ...btnStyle, fontSize: 12, padding: '5px 12px', borderColor: 'var(--cyan)', color: 'var(--cyan)' }}
          onClick={() => setShowForm(f => !f)}
        >
          {showForm ? 'Cancel' : '+ Add Entry'}
        </button>
      </div>

      {/* Add entry form */}
      {showForm && (
        <div style={{
          background: 'var(--card)',
          border: '1px solid var(--border)',
          borderRadius: 8,
          padding: 16,
          display: 'flex',
          flexDirection: 'column',
          gap: 10,
        }}>
          <label style={labelStyle}>
            <span style={labelTextStyle}>Description</span>
            <textarea
              value={description}
              onChange={e => setDescription(e.target.value)}
              placeholder="What was done..."
              rows={3}
              style={{ ...inputStyle, resize: 'vertical', minHeight: 60 }}
            />
          </label>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
            <label style={labelStyle}>
              <span style={labelTextStyle}>Technician</span>
              <input
                type="text"
                value={technician}
                onChange={e => setTechnician(e.target.value)}
                placeholder="Name"
                style={inputStyle}
              />
            </label>
            <label style={labelStyle}>
              <span style={labelTextStyle}>Date Performed</span>
              <input
                type="date"
                value={performedAt}
                onChange={e => setPerformedAt(e.target.value)}
                style={inputStyle}
              />
            </label>
          </div>

          {createLog.isError && (
            <div style={{ fontSize: 12, color: 'var(--crit)', padding: '6px 10px', background: 'rgba(239,68,68,.1)', borderRadius: 4 }}>
              Failed to add entry. Please try again.
            </div>
          )}

          <button
            onClick={handleSubmit}
            disabled={createLog.isPending || !description.trim() || !technician.trim()}
            style={{
              ...btnStyle,
              alignSelf: 'flex-end',
              background: 'var(--cyan)',
              borderColor: 'var(--cyan)',
              color: '#fff',
              opacity: createLog.isPending || !description.trim() || !technician.trim() ? 0.5 : 1,
              cursor: createLog.isPending || !description.trim() || !technician.trim() ? 'not-allowed' : 'pointer',
            }}
          >
            {createLog.isPending ? 'Saving...' : 'Save Entry'}
          </button>
        </div>
      )}

      {/* Timeline list */}
      {!logs || logs.length === 0 ? (
        <div style={{
          padding: '32px 16px',
          textAlign: 'center',
          color: 'var(--muted)',
          fontSize: 13,
        }}>
          No maintenance entries yet.
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 0 }}>
          {logs.map((log, i) => (
            <div
              key={log.id}
              style={{
                display: 'flex',
                gap: 12,
                padding: '12px 0',
                borderTop: i > 0 ? '1px solid var(--border)' : 'none',
              }}
            >
              {/* Timeline dot */}
              <div style={{
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'center',
                paddingTop: 2,
                width: 16,
                flexShrink: 0,
              }}>
                <div style={{
                  width: 10,
                  height: 10,
                  borderRadius: '50%',
                  background: i === 0 ? 'var(--cyan)' : 'var(--border)',
                  flexShrink: 0,
                }} />
                {i < logs.length - 1 && (
                  <div style={{
                    width: 1,
                    flex: 1,
                    minHeight: 8,
                    background: 'var(--border)',
                    marginTop: 4,
                  }} />
                )}
              </div>

              {/* Content */}
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, color: 'var(--fg)', fontWeight: 500, lineHeight: 1.4 }}>
                  {log.description}
                </div>
                <div style={{ display: 'flex', gap: 16, marginTop: 4, fontSize: 11, color: 'var(--muted)' }}>
                  <span>Technician: {log.technician}</span>
                  <span>{new Date(log.performed_at).toLocaleDateString('en-US', { year: 'numeric', month: 'short', day: 'numeric' })}</span>
                  <span>by {log.created_by}</span>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
