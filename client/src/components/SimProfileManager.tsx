import { useState, useEffect, useRef, useCallback } from 'react'
import { inputStyle, monoInputStyle, labelStyle, labelTextStyle, btnStyle, parseConfig } from '../styles'
import {
  listSimProfiles,
  createSimProfile,
  updateSimProfile,
  deleteSimProfile,
} from '../api/simulation'
import type { SimProfile } from '../api/simulation'

// ── Types ────────────────────────────────────────────────────────────

type DataType = 'sinusoidal' | 'random_walk' | 'step' | 'constant' | 'square_wave'

interface ConfigField {
  key: string
  label: string
  type: 'number' | 'range'
  min?: number
  max?: number
  step?: string | number
  placeholder?: string
}

const DATA_TYPES: { value: DataType; label: string }[] = [
  { value: 'sinusoidal', label: 'Sinusoidal' },
  { value: 'random_walk', label: 'Random Walk' },
  { value: 'step', label: 'Step' },
  { value: 'constant', label: 'Constant' },
  { value: 'square_wave', label: 'Square Wave' },
]

const CONFIG_FIELDS: Record<DataType, ConfigField[]> = {
  sinusoidal: [
    { key: 'amplitude', label: 'Amplitude', type: 'number', step: 'any', placeholder: 'e.g. 100' },
    { key: 'center', label: 'Center', type: 'number', step: 'any', placeholder: 'e.g. 0' },
    { key: 'period_sec', label: 'Period (s)', type: 'number', step: 'any', min: 0.1, placeholder: 'e.g. 60' },
    { key: 'noise_pct', label: 'Noise (%)', type: 'number', step: 'any', min: 0, max: 100, placeholder: 'e.g. 5' },
  ],
  random_walk: [
    { key: 'start_value', label: 'Start Value', type: 'number', step: 'any', placeholder: 'e.g. 50' },
    { key: 'step_size', label: 'Step Size', type: 'number', step: 'any', min: 0, placeholder: 'e.g. 0.5' },
    { key: 'min', label: 'Min', type: 'number', step: 'any', placeholder: 'e.g. 0' },
    { key: 'max', label: 'Max', type: 'number', step: 'any', placeholder: 'e.g. 100' },
  ],
  step: [
    { key: 'low_value', label: 'Low Value', type: 'number', step: 'any', placeholder: 'e.g. 0' },
    { key: 'high_value', label: 'High Value', type: 'number', step: 'any', placeholder: 'e.g. 100' },
    { key: 'step_period_sec', label: 'Step Period (s)', type: 'number', step: 'any', min: 0.1, placeholder: 'e.g. 30' },
  ],
  constant: [
    { key: 'value', label: 'Value', type: 'number', step: 'any', placeholder: 'e.g. 50' },
  ],
  square_wave: [
    { key: 'high_value', label: 'High Value', type: 'number', step: 'any', placeholder: 'e.g. 100' },
    { key: 'low_value', label: 'Low Value', type: 'number', step: 'any', placeholder: 'e.g. 0' },
    { key: 'period_sec', label: 'Period (s)', type: 'number', step: 'any', min: 0.1, placeholder: 'e.g. 60' },
    { key: 'duty_pct', label: 'Duty (%)', type: 'number', step: 'any', min: 0, max: 100, placeholder: 'e.g. 50' },
  ],
}

// ── Waveform preview generation ──────────────────────────────────────

function generateWaveform(dataType: DataType, config: Record<string, number>, points: number = 200): number[] {
  const values: number[] = []
  for (let i = 0; i < points; i++) {
    const t = i / points // normalized 0..1
    let v = 0
    switch (dataType) {
      case 'sinusoidal': {
        const amp = config.amplitude ?? 50
        const center = config.center ?? 0
        const noise = (config.noise_pct ?? 0) / 100
        v = center + amp * Math.sin(t * Math.PI * 2)
        if (noise > 0) v += (Math.random() - 0.5) * 2 * amp * noise
        break
      }
      case 'random_walk': {
        const start = config.start_value ?? 50
        const step = config.step_size ?? 0.5
        const min = config.min ?? 0
        const max = config.max ?? 100
        // deterministic walk using index as seed
        v = start
        let seed = 42
        for (let j = 0; j < i; j++) {
          seed = (seed * 1103515245 + 12345) & 0x7fffffff
          v += (seed % 2 === 0 ? 1 : -1) * step
          v = Math.max(min, Math.min(max, v))
        }
        break
      }
      case 'step': {
        const low = config.low_value ?? 0
        const high = config.high_value ?? 100
        const period = config.step_period_sec ?? 30
        const steps = Math.floor(t * 4) // show ~4 steps
        v = steps % 2 === 0 ? low : high
        break
      }
      case 'constant': {
        v = config.value ?? 50
        break
      }
      case 'square_wave': {
        const hi = config.high_value ?? 100
        const lo = config.low_value ?? 0
        const duty = (config.duty_pct ?? 50) / 100
        v = (t % 1) < duty ? hi : lo
        break
      }
    }
    values.push(v)
  }
  return values
}

// ── Waveform canvas ──────────────────────────────────────────────────

function WaveformPreview({ dataType, config }: { dataType: DataType; config: Record<string, number> }) {
  const canvasRef = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const ctx = canvas.getContext('2d')
    if (!ctx) return

    const dpr = window.devicePixelRatio || 1
    const w = canvas.clientWidth
    const h = canvas.clientHeight
    canvas.width = w * dpr
    canvas.height = h * dpr
    ctx.scale(dpr, dpr)

    // Background
    ctx.fillStyle = 'var(--bg, #0a0e17)'
    ctx.fillRect(0, 0, w, h)

    // Grid lines
    ctx.strokeStyle = 'rgba(255,255,255,0.06)'
    ctx.lineWidth = 1
    for (let i = 1; i < 4; i++) {
      const y = (h / 4) * i
      ctx.beginPath()
      ctx.moveTo(0, y)
      ctx.lineTo(w, y)
      ctx.stroke()
    }
    for (let i = 1; i < 8; i++) {
      const x = (w / 8) * i
      ctx.beginPath()
      ctx.moveTo(x, 0)
      ctx.lineTo(x, h)
      ctx.stroke()
    }

    // Generate waveform
    const values = generateWaveform(dataType, config)
    if (values.length === 0) return

    const min = Math.min(...values)
    const max = Math.max(...values)
    const range = max - min || 1
    const pad = 8

    // Draw waveform
    ctx.beginPath()
    ctx.strokeStyle = '#22d3ee'
    ctx.lineWidth = 1.5
    ctx.lineJoin = 'round'
    for (let i = 0; i < values.length; i++) {
      const x = pad + (i / (values.length - 1)) * (w - pad * 2)
      const y = pad + (1 - (values[i] - min) / range) * (h - pad * 2)
      if (i === 0) ctx.moveTo(x, y)
      else ctx.lineTo(x, y)
    }
    ctx.stroke()

    // Fill under curve
    const lastX = pad + (w - pad * 2)
    const baseline = pad + (h - pad * 2)
    ctx.lineTo(lastX, baseline)
    ctx.lineTo(pad, baseline)
    ctx.closePath()
    ctx.fillStyle = 'rgba(34,211,238,0.08)'
    ctx.fill()

    // Min/max labels
    ctx.fillStyle = 'rgba(255,255,255,0.4)'
    ctx.font = '10px JetBrains Mono, monospace'
    ctx.textAlign = 'left'
    ctx.fillText(max.toFixed(1), pad, pad + 10)
    ctx.fillText(min.toFixed(1), pad, baseline - 2)
  }, [dataType, config])

  return (
    <canvas
      ref={canvasRef}
      style={{
        width: '100%',
        height: 80,
        borderRadius: 6,
        border: '1px solid var(--border)',
        display: 'block',
      }}
    />
  )
}

// ── Config form fields ───────────────────────────────────────────────

function ConfigFields({
  dataType,
  config,
  onChange,
}: {
  dataType: DataType
  config: Record<string, number>
  onChange: (key: string, value: number) => void
}) {
  const fields = CONFIG_FIELDS[dataType] ?? []
  return (
    <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
      {fields.map(field => (
        <label key={field.key} style={labelStyle}>
          <span style={labelTextStyle}>{field.label}</span>
          <input
            type="number"
            step={field.step ?? 'any'}
            min={field.min}
            max={field.max}
            value={config[field.key] ?? ''}
            onChange={e => {
              const val = e.target.value === '' ? 0 : parseFloat(e.target.value)
              if (!isNaN(val)) onChange(field.key, val)
            }}
            placeholder={field.placeholder}
            style={monoInputStyle}
          />
        </label>
      ))}
    </div>
  )
}

// ── Delete confirmation dialog ───────────────────────────────────────

function ConfirmDialog({
  message,
  onConfirm,
  onCancel,
}: {
  message: string
  onConfirm: () => void
  onCancel: () => void
}) {
  return (
    <div style={{
      position: 'fixed', inset: 0, zIndex: 1100,
      display: 'flex', alignItems: 'center', justifyContent: 'center',
    }}>
      <div onClick={onCancel} style={{ position: 'absolute', inset: 0, background: 'rgba(0,0,0,.5)' }} />
      <div style={{
        position: 'relative', background: 'var(--card)', border: '1px solid var(--border)',
        borderRadius: 10, padding: 20, maxWidth: 360, width: '90%',
        display: 'flex', flexDirection: 'column', gap: 14,
      }}>
        <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--fg)' }}>Confirm Delete</div>
        <div style={{ fontSize: 13, color: 'var(--muted)' }}>{message}</div>
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <button
            onClick={onCancel}
            style={{ ...btnStyle, border: '1px solid var(--border)', color: 'var(--muted)' }}
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            style={{
              ...btnStyle,
              border: '1px solid var(--crit)',
              background: 'rgba(239,68,68,.15)',
              color: 'var(--crit)',
            }}
          >
            Delete
          </button>
        </div>
      </div>
    </div>
  )
}

// ── Profile row ──────────────────────────────────────────────────────

function ProfileRow({
  profile,
  selected,
  onSelect,
  onEdit,
  onDelete,
}: {
  profile: SimProfile
  selected: boolean
  onSelect: () => void
  onEdit: () => void
  onDelete: () => void
}) {
  const typeLabel = DATA_TYPES.find(d => d.value === profile.data_type)?.label ?? profile.data_type

  return (
    <div
      onClick={onSelect}
      style={{
        padding: '10px 12px', borderRadius: 6, marginBottom: 4,
        border: `1px solid ${selected ? 'var(--cyan)' : 'var(--border)'}`,
        background: selected ? 'rgba(34,211,238,.06)' : 'var(--card)',
        cursor: 'pointer',
        display: 'flex', justifyContent: 'space-between', alignItems: 'center',
      }}
    >
      <div>
        <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--fg)' }}>{profile.name}</div>
        <div style={{
          fontSize: 10, padding: '1px 6px', borderRadius: 3, marginTop: 4, display: 'inline-block',
          background: 'rgba(139,92,246,.1)', color: '#a78bfa', fontWeight: 600,
        }}>
          {typeLabel}
        </div>
      </div>
      <div style={{ display: 'flex', gap: 6 }}>
        <button
          onClick={e => { e.stopPropagation(); onEdit() }}
          style={{
            padding: '3px 8px', borderRadius: 4, fontSize: 10, cursor: 'pointer',
            border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)',
          }}
        >
          Edit
        </button>
        <button
          onClick={e => { e.stopPropagation(); onDelete() }}
          style={{
            padding: '3px 8px', borderRadius: 4, fontSize: 10, cursor: 'pointer',
            border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)',
          }}
        >
          Delete
        </button>
      </div>
    </div>
  )
}

// ── Main component ───────────────────────────────────────────────────

export function SimProfileManager() {
  const [profiles, setProfiles] = useState<SimProfile[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const [selectedId, setSelectedId] = useState<number | null>(null)

  // Form state
  const [showForm, setShowForm] = useState(false)
  const [editingId, setEditingId] = useState<number | null>(null)
  const [formName, setFormName] = useState('')
  const [formDataType, setFormDataType] = useState<DataType>('sinusoidal')
  const [formConfig, setFormConfig] = useState<Record<string, number>>({})
  const [saving, setSaving] = useState(false)
  const [formError, setFormError] = useState('')

  // Delete confirmation
  const [deleteTarget, setDeleteTarget] = useState<SimProfile | null>(null)
  const [deleting, setDeleting] = useState(false)

  // Fetch profiles
  const fetchProfiles = useCallback(async () => {
    try {
      const data = await listSimProfiles()
      setProfiles(data)
      setError('')
    } catch (e: any) {
      setError(e.message || 'Failed to load profiles')
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    fetchProfiles()
  }, [fetchProfiles])

  // Reset form to defaults for a data type
  const defaultConfig = useCallback((dataType: DataType): Record<string, number> => {
    const cfg: Record<string, number> = {}
    for (const field of CONFIG_FIELDS[dataType]) {
      cfg[field.key] = 0
    }
    return cfg
  }, [])

  // Start creating
  const handleCreate = () => {
    setEditingId(null)
    setFormName('')
    setFormDataType('sinusoidal')
    setFormConfig(defaultConfig('sinusoidal'))
    setFormError('')
    setShowForm(true)
  }

  // Start editing
  const handleEdit = (profile: SimProfile) => {
    setEditingId(profile.id)
    setFormName(profile.name)
    setFormDataType(profile.data_type as DataType)
    const parsed = parseConfig(profile.config)
    setFormConfig(typeof parsed === 'object' && parsed !== null ? parsed as Record<string, number> : {})
    setFormError('')
    setShowForm(true)
  }

  // Save (create or update)
  const handleSave = async () => {
    if (!formName.trim()) {
      setFormError('Name is required')
      return
    }
    setSaving(true)
    setFormError('')
    try {
      const payload = {
        name: formName.trim(),
        data_type: formDataType,
        config: JSON.stringify(formConfig),
      }
      if (editingId !== null) {
        await updateSimProfile(editingId, payload)
      } else {
        await createSimProfile(payload)
      }
      setShowForm(false)
      await fetchProfiles()
    } catch (e: any) {
      setFormError(e.message || 'Failed to save profile')
    } finally {
      setSaving(false)
    }
  }

  // Delete
  const handleDelete = async () => {
    if (!deleteTarget) return
    setDeleting(true)
    try {
      await deleteSimProfile(deleteTarget.id)
      if (selectedId === deleteTarget.id) setSelectedId(null)
      await fetchProfiles()
    } catch (e: any) {
      setError(e.message || 'Failed to delete profile')
    } finally {
      setDeleting(false)
      setDeleteTarget(null)
    }
  }

  // Handle data type change in form
  const handleDataTypeChange = (dt: DataType) => {
    setFormDataType(dt)
    setFormConfig(defaultConfig(dt))
  }

  // Handle config field change
  const handleConfigChange = (key: string, value: number) => {
    setFormConfig(prev => ({ ...prev, [key]: value }))
  }

  // Selected profile for preview
  const selectedProfile = profiles.find(p => p.id === selectedId)
  const previewDataType = showForm ? formDataType : (selectedProfile?.data_type as DataType ?? 'sinusoidal')
  const previewConfig = showForm
    ? formConfig
    : (selectedProfile ? (parseConfig(selectedProfile.config) as Record<string, number>) : {})

  return (
    <div style={{
      background: 'var(--card)',
      border: '1px solid var(--border)',
      borderRadius: 8,
      display: 'flex', flexDirection: 'column',
      height: '100%', overflow: 'hidden',
    }}>
      {/* Header */}
      <div style={{
        padding: '12px 16px', borderBottom: '1px solid var(--border)',
        display: 'flex', justifyContent: 'space-between', alignItems: 'center',
      }}>
        <span style={{ fontSize: 15, fontWeight: 700 }}>Simulation Profiles</span>
        <button
          onClick={handleCreate}
          disabled={showForm}
          style={{
            ...btnStyle,
            border: '1px solid var(--cyan)',
            background: showForm ? 'transparent' : 'var(--cyan)',
            color: showForm ? 'var(--muted)' : '#fff',
            cursor: showForm ? 'not-allowed' : 'pointer',
            opacity: showForm ? 0.5 : 1,
            fontSize: 12, padding: '5px 12px',
          }}
        >
          + New Profile
        </button>
      </div>

      {/* Content area */}
      <div style={{ flex: 1, overflow: 'auto', display: 'flex', flexDirection: 'column' }}>
        {/* Profile list */}
        <div style={{ flex: 1, overflow: 'auto', padding: 8 }}>
          {loading && (
            <div style={{ padding: 20, color: 'var(--muted)', textAlign: 'center', fontSize: 13 }}>
              Loading...
            </div>
          )}
          {error && !showForm && (
            <div style={{ padding: 12, color: 'var(--crit)', textAlign: 'center', fontSize: 12 }}>
              {error}
            </div>
          )}
          {!loading && profiles.length === 0 && !error && (
            <div style={{ padding: 20, color: 'var(--muted)', textAlign: 'center', fontSize: 13 }}>
              No simulation profiles yet. Create one to get started.
            </div>
          )}
          {!loading && profiles.map(profile => (
            <ProfileRow
              key={profile.id}
              profile={profile}
              selected={selectedId === profile.id}
              onSelect={() => setSelectedId(selectedId === profile.id ? null : profile.id)}
              onEdit={() => handleEdit(profile)}
              onDelete={() => setDeleteTarget(profile)}
            />
          ))}
        </div>

        {/* Form / Preview panel */}
        {showForm ? (
          <div style={{
            borderTop: '1px solid var(--border)',
            padding: 16, display: 'flex', flexDirection: 'column', gap: 12,
            background: 'rgba(0,0,0,.2)',
          }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--fg)' }}>
              {editingId !== null ? 'Edit Profile' : 'New Profile'}
            </div>

            {/* Name */}
            <label style={labelStyle}>
              <span style={labelTextStyle}>Name</span>
              <input
                type="text"
                value={formName}
                onChange={e => setFormName(e.target.value)}
                placeholder="Profile name"
                style={inputStyle}
                autoFocus
              />
            </label>

            {/* Data type */}
            <label style={labelStyle}>
              <span style={labelTextStyle}>Data Type</span>
              <select
                value={formDataType}
                onChange={e => handleDataTypeChange(e.target.value as DataType)}
                style={{
                  ...inputStyle,
                  appearance: 'none',
                  background: 'var(--bg) url("data:image/svg+xml,%3Csvg xmlns=%27http://www.w3.org/2000/svg%27 width=%2712%27 height=%2712%27 viewBox=%270 0 24 24%27 fill=%27none%27 stroke=%27%2394a3b8%27 stroke-width=%272%27%3E%3Cpolyline points=%276 9 12 15 18 9%27/%3E%3C/svg%3E") no-repeat right 8px center',
                  paddingRight: 28,
                }}
              >
                {DATA_TYPES.map(dt => (
                  <option key={dt.value} value={dt.value}>{dt.label}</option>
                ))}
              </select>
            </label>

            {/* Config fields */}
            <ConfigFields
              dataType={formDataType}
              config={formConfig}
              onChange={handleConfigChange}
            />

            {/* Waveform preview */}
            <div style={{ marginTop: 4 }}>
              <div style={{ fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase', marginBottom: 4, letterSpacing: '.04em' }}>
                Waveform Preview
              </div>
              <WaveformPreview dataType={formDataType} config={formConfig} />
            </div>

            {/* Form error */}
            {formError && (
              <div style={{ fontSize: 12, color: 'var(--crit)', padding: '6px 10px', background: 'rgba(239,68,68,.1)', borderRadius: 4 }}>
                {formError}
              </div>
            )}

            {/* Form actions */}
            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
              <button
                onClick={() => setShowForm(false)}
                disabled={saving}
                style={{
                  ...btnStyle,
                  border: '1px solid var(--border)',
                  color: 'var(--muted)',
                  cursor: saving ? 'not-allowed' : 'pointer',
                  opacity: saving ? 0.5 : 1,
                }}
              >
                Cancel
              </button>
              <button
                onClick={handleSave}
                disabled={saving}
                style={{
                  ...btnStyle,
                  border: '1px solid var(--cyan)',
                  background: saving ? 'transparent' : 'var(--cyan)',
                  color: saving ? 'var(--muted)' : '#fff',
                  cursor: saving ? 'not-allowed' : 'pointer',
                  opacity: saving ? 0.5 : 1,
                }}
              >
                {saving ? 'Saving...' : editingId !== null ? 'Update' : 'Create'}
              </button>
            </div>
          </div>
        ) : selectedProfile ? (
          /* Preview for selected profile */
          <div style={{
            borderTop: '1px solid var(--border)',
            padding: 16, display: 'flex', flexDirection: 'column', gap: 10,
            background: 'rgba(0,0,0,.2)',
          }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
              <div>
                <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--fg)' }}>{selectedProfile.name}</span>
                <span style={{
                  fontSize: 10, padding: '1px 6px', borderRadius: 3, marginLeft: 8,
                  background: 'rgba(139,92,246,.1)', color: '#a78bfa', fontWeight: 600,
                }}>
                  {DATA_TYPES.find(d => d.value === selectedProfile.data_type)?.label ?? selectedProfile.data_type}
                </span>
              </div>
              <button
                onClick={() => handleEdit(selectedProfile)}
                style={{ ...btnStyle, fontSize: 11, padding: '4px 10px' }}
              >
                Edit
              </button>
            </div>

            {/* Config values */}
            {(() => {
              const cfg = parseConfig(selectedProfile.config)
              if (typeof cfg !== 'object' || cfg === null || Object.keys(cfg).length === 0) return null
              return (
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
                  {Object.entries(cfg).map(([key, val]) => (
                    <div key={key} style={{
                      fontSize: 10, padding: '3px 8px', borderRadius: 4,
                      background: 'rgba(255,255,255,.04)', color: 'var(--muted)',
                      fontFamily: "var(--font-mono)",
                    }}>
                      {key}: {String(val)}
                    </div>
                  ))}
                </div>
              )
            })()}

            {/* Waveform preview */}
            <div>
              <div style={{ fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase', marginBottom: 4, letterSpacing: '.04em' }}>
                Waveform Preview
              </div>
              <WaveformPreview
                dataType={selectedProfile.data_type as DataType}
                config={parseConfig(selectedProfile.config) as Record<string, number>}
              />
            </div>
          </div>
        ) : null}
      </div>

      {/* Delete confirmation dialog */}
      {deleteTarget && (
        <ConfirmDialog
          message={`Are you sure you want to delete "${deleteTarget.name}"? This action cannot be undone.`}
          onConfirm={handleDelete}
          onCancel={() => setDeleteTarget(null)}
        />
      )}
    </div>
  )
}
