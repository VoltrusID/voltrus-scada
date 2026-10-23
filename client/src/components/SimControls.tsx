import { useState, useEffect, useCallback } from 'react'
import { inputStyle, monoInputStyle, labelStyle, labelTextStyle, btnStyle, parseConfig } from '../styles'
import { toggleSimulation, goLive, listSimProfiles } from '../api/simulation'
import type { SimProfile } from '../api/simulation'

// ── Types ──────────────────────────────────────────────────────────

interface SimControlsProps {
  deviceId: string
  deviceName?: string
  simulationMode: boolean
  simProfileId?: number | null
  simConfig?: string | null
  onUpdate: () => void
}

interface TagOverride {
  tagName: string
  value: number
}

// ── Confirmation dialog (reuse pattern from SimProfileManager) ─────

function ConfirmDialog({
  title,
  message,
  confirmLabel,
  confirmColor,
  onConfirm,
  onCancel,
  loading,
}: {
  title: string
  message: string
  confirmLabel: string
  confirmColor: string
  onConfirm: () => void
  onCancel: () => void
  loading: boolean
}) {
  return (
    <div style={{
      position: 'fixed', inset: 0, zIndex: 1100,
      display: 'flex', alignItems: 'center', justifyContent: 'center',
    }}>
      <div onClick={onCancel} style={{ position: 'absolute', inset: 0, background: 'rgba(0,0,0,.5)' }} />
      <div style={{
        position: 'relative', background: 'var(--card)', border: '1px solid var(--border)',
        borderRadius: 10, padding: 20, maxWidth: 400, width: '90%',
        display: 'flex', flexDirection: 'column', gap: 14,
      }}>
        <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--fg)' }}>{title}</div>
        <div style={{ fontSize: 13, color: 'var(--muted)', lineHeight: 1.5 }}>{message}</div>
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <button
            onClick={onCancel}
            disabled={loading}
            style={{
              ...btnStyle,
              border: '1px solid var(--border)',
              color: 'var(--muted)',
              cursor: loading ? 'not-allowed' : 'pointer',
              opacity: loading ? 0.5 : 1,
            }}
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            disabled={loading}
            style={{
              ...btnStyle,
              border: `1px solid ${confirmColor}`,
              background: confirmColor === 'var(--ok)'
                ? 'rgba(34,197,94,.15)'
                : confirmColor === 'var(--warn)'
                  ? 'rgba(234,179,8,.15)'
                  : 'rgba(34,211,238,.15)',
              color: confirmColor,
              cursor: loading ? 'not-allowed' : 'pointer',
              opacity: loading ? 0.5 : 1,
            }}
          >
            {loading ? 'Please wait...' : confirmLabel}
          </button>
        </div>
      </div>
    </div>
  )
}

// ── SIM badge ──────────────────────────────────────────────────────

function SimBadge() {
  return (
    <span style={{
      fontSize: 10, fontWeight: 700, textTransform: 'uppercase',
      padding: '2px 8px', borderRadius: 4, letterSpacing: '.06em',
      background: 'rgba(234,179,8,.15)', color: '#facc15',
      border: '1px solid rgba(234,179,8,.3)',
    }}>
      SIM
    </span>
  )
}

// ── Toggle switch (pattern from WritePolicyEditor) ─────────────────

function ToggleSwitch({
  checked,
  onChange,
  disabled,
}: {
  checked: boolean
  onChange: (val: boolean) => void
  disabled?: boolean
}) {
  return (
    <button
      onClick={() => !disabled && onChange(!checked)}
      disabled={disabled}
      style={{
        width: 40, height: 22, borderRadius: 11,
        border: 'none', cursor: disabled ? 'not-allowed' : 'pointer',
        position: 'relative',
        background: checked ? 'var(--ok)' : 'var(--border)',
        transition: 'background 0.2s',
        opacity: disabled ? 0.5 : 1,
      }}
    >
      <div style={{
        width: 16, height: 16, borderRadius: '50%', background: '#fff',
        position: 'absolute', top: 3,
        left: checked ? 21 : 3,
        transition: 'left 0.2s',
      }} />
    </button>
  )
}

// ── Main component ─────────────────────────────────────────────────

export function SimControls({
  deviceId,
  deviceName,
  simulationMode,
  simProfileId,
  simConfig,
  onUpdate,
}: SimControlsProps) {
  const [profiles, setProfiles] = useState<SimProfile[]>([])
  const [profilesLoading, setProfilesLoading] = useState(true)
  const [profilesError, setProfilesError] = useState('')

  // Local form state
  const [selectedProfileId, setSelectedProfileId] = useState<number | null>(simProfileId ?? null)
  const [tagOverrides, setTagOverrides] = useState<TagOverride[]>([])
  const [simEnabled, setSimEnabled] = useState(simulationMode)

  // Operation states
  const [toggling, setToggling] = useState(false)
  const [error, setError] = useState('')

  // Confirmation dialogs
  const [showGoLiveConfirm, setShowGoLiveConfirm] = useState(false)
  const [showEnableConfirm, setShowEnableConfirm] = useState(false)

  // Sync props to local state
  useEffect(() => {
    setSimEnabled(simulationMode)
    setSelectedProfileId(simProfileId ?? null)
  }, [simulationMode, simProfileId])

  // Parse tag overrides from simConfig when constant mode
  useEffect(() => {
    if (simConfig) {
      try {
        const parsed = parseConfig(simConfig)
        if (parsed && typeof parsed === 'object') {
          // If it's a flat tag-value map, convert to array
          const entries = Object.entries(parsed)
          if (entries.length > 0 && typeof entries[0][1] === 'number') {
            setTagOverrides(
              entries.map(([tagName, value]) => ({ tagName, value: value as number })),
            )
          }
        }
      } catch {
        // ignore parse errors
      }
    }
  }, [simConfig])

  // Fetch profiles on mount
  const fetchProfiles = useCallback(async () => {
    try {
      const data = await listSimProfiles()
      setProfiles(data)
      setProfilesError('')
    } catch (e: any) {
      setProfilesError(e.message || 'Failed to load profiles')
    } finally {
      setProfilesLoading(false)
    }
  }, [])

  useEffect(() => {
    fetchProfiles()
  }, [fetchProfiles])

  // Determine selected profile's data type
  const selectedProfile = profiles.find(p => p.id === selectedProfileId)
  const isConstantMode = selectedProfile?.data_type === 'constant'

  // Handle tag override change
  const handleTagOverrideChange = (index: number, value: number) => {
    setTagOverrides(prev => prev.map((t, i) => i === index ? { ...t, value } : t))
  }

  // Handle add tag override
  const handleAddTagOverride = () => {
    setTagOverrides(prev => [...prev, { tagName: '', value: 0 }])
  }

  // Handle remove tag override
  const handleRemoveTagOverride = (index: number) => {
    setTagOverrides(prev => prev.filter((_, i) => i !== index))
  }

  // Build the sim config JSON
  const buildSimConfig = useCallback((): string | undefined => {
    if (!isConstantMode || tagOverrides.length === 0) return undefined
    const config: Record<string, number> = {}
    for (const t of tagOverrides) {
      if (t.tagName.trim()) {
        config[t.tagName.trim()] = t.value
      }
    }
    return Object.keys(config).length > 0 ? JSON.stringify(config) : undefined
  }, [isConstantMode, tagOverrides])

  // Enable simulation
  const handleEnableSim = async () => {
    setShowEnableConfirm(false)
    setToggling(true)
    setError('')
    try {
      await toggleSimulation(deviceId, {
        simulation_mode: true,
        sim_profile_id: selectedProfileId ?? undefined,
        sim_config: buildSimConfig(),
      })
      setSimEnabled(true)
      onUpdate()
    } catch (e: any) {
      setError(e.message || 'Failed to enable simulation')
    } finally {
      setToggling(false)
    }
  }

  // Go live
  const handleGoLive = async () => {
    setShowGoLiveConfirm(false)
    setToggling(true)
    setError('')
    try {
      await goLive(deviceId)
      setSimEnabled(false)
      onUpdate()
    } catch (e: any) {
      setError(e.message || 'Failed to switch to live mode')
    } finally {
      setToggling(false)
    }
  }

  // Toggle handler
  const handleToggle = (checked: boolean) => {
    if (checked) {
      setShowEnableConfirm(true)
    } else {
      setShowGoLiveConfirm(true)
    }
  }

  // Go-live button handler (separate from toggle for explicit action)
  const handleGoLiveBtn = () => {
    setShowGoLiveConfirm(true)
  }

  return (
    <div style={{
      background: 'var(--card)',
      border: '1px solid var(--border)',
      borderRadius: 8,
      padding: 16,
      display: 'flex',
      flexDirection: 'column',
      gap: 12,
    }}>
      {/* Header with toggle and badge */}
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <div style={{ fontSize: 13, fontWeight: 600 }}>Simulation</div>
          {simEnabled && <SimBadge />}
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <span style={{ fontSize: 11, color: simEnabled ? 'var(--ok)' : 'var(--muted)' }}>
            {simEnabled ? 'Active' : 'Inactive'}
          </span>
          <ToggleSwitch
            checked={simEnabled}
            onChange={handleToggle}
            disabled={toggling}
          />
        </div>
      </div>

      {/* Device name label */}
      {deviceName && (
        <div style={{ fontSize: 11, color: 'var(--muted)' }}>
          Device: <span style={{ color: 'var(--fg)', fontWeight: 500 }}>{deviceName}</span>
        </div>
      )}

      {/* Simulation settings (shown when sim is enabled or being configured) */}
      {(simEnabled || showEnableConfirm) && (
        <div style={{
          display: 'flex', flexDirection: 'column', gap: 10,
          paddingLeft: 4, borderLeft: '2px solid var(--ok)',
        }}>
          {/* Profile dropdown */}
          <label style={labelStyle}>
            <span style={labelTextStyle}>Sim Profile</span>
            {profilesLoading ? (
              <div style={{ fontSize: 12, color: 'var(--muted)', padding: '6px 8px' }}>
                Loading profiles...
              </div>
            ) : profilesError ? (
              <div style={{ fontSize: 12, color: 'var(--crit)', padding: '6px 8px' }}>
                {profilesError}
              </div>
            ) : (
              <select
                value={selectedProfileId ?? ''}
                onChange={e => setSelectedProfileId(e.target.value ? parseInt(e.target.value) : null)}
                style={{
                  ...inputStyle,
                  appearance: 'none',
                  background: 'var(--bg) url("data:image/svg+xml,%3Csvg xmlns=%27http://www.w3.org/2000/svg%27 width=%2712%27 height=%2712%27 viewBox=%270 0 24 24%27 fill=%27none%27 stroke=%27%2394a3b8%27 stroke-width=%272%27%3E%3Cpolyline points=%276 9 12 15 18 9%27/%3E%3C/svg%3E") no-repeat right 8px center',
                  paddingRight: 28,
                }}
              >
                <option value="">-- Select profile --</option>
                {profiles.map(p => (
                  <option key={p.id} value={p.id}>{p.name} ({p.data_type})</option>
                ))}
              </select>
            )}
          </label>

          {/* Per-tag value overrides for constant mode */}
          {isConstantMode && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              <div style={{
                fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase',
                letterSpacing: '.04em',
              }}>
                Tag Value Overrides
              </div>

              {tagOverrides.map((tag, index) => (
                <div key={index} style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                  <input
                    type="text"
                    value={tag.tagName}
                    onChange={e => setTagOverrides(prev =>
                      prev.map((t, i) => i === index ? { ...t, tagName: e.target.value } : t),
                    )}
                    placeholder="Tag name"
                    style={{ ...inputStyle, flex: 1, fontSize: 12 }}
                  />
                  <input
                    type="number"
                    step="any"
                    value={tag.value}
                    onChange={e => {
                      const val = e.target.value === '' ? 0 : parseFloat(e.target.value)
                      if (!isNaN(val)) handleTagOverrideChange(index, val)
                    }}
                    placeholder="Value"
                    style={{ ...monoInputStyle, width: 90, fontSize: 12 }}
                  />
                  <button
                    onClick={() => handleRemoveTagOverride(index)}
                    style={{
                      padding: '4px 8px', borderRadius: 4, fontSize: 10, cursor: 'pointer',
                      border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)',
                    }}
                  >
                    Remove
                  </button>
                </div>
              ))}

              <button
                onClick={handleAddTagOverride}
                style={{
                  padding: '4px 10px', borderRadius: 4, fontSize: 11, cursor: 'pointer',
                  border: '1px dashed var(--border)', background: 'transparent', color: 'var(--muted)',
                }}
              >
                + Add tag override
              </button>
            </div>
          )}

          {/* Go-live button when sim is active */}
          {simEnabled && (
            <div style={{
              padding: '10px 12px', borderRadius: 6,
              background: 'rgba(234,179,8,.08)', border: '1px solid rgba(234,179,8,.2)',
              display: 'flex', justifyContent: 'space-between', alignItems: 'center',
            }}>
              <div>
                <div style={{ fontSize: 12, fontWeight: 600, color: '#facc15' }}>
                  Simulation Active
                </div>
                <div style={{ fontSize: 11, color: 'var(--muted)', marginTop: 2 }}>
                  Device is receiving simulated data, not live PLC values.
                </div>
              </div>
              <button
                onClick={handleGoLiveBtn}
                disabled={toggling}
                style={{
                  ...btnStyle,
                  border: '1px solid var(--ok)',
                  background: 'rgba(34,197,94,.15)',
                  color: 'var(--ok)',
                  cursor: toggling ? 'not-allowed' : 'pointer',
                  opacity: toggling ? 0.5 : 1,
                  fontSize: 12, padding: '5px 12px', fontWeight: 600,
                  whiteSpace: 'nowrap',
                }}
              >
                Go Live
              </button>
            </div>
          )}
        </div>
      )}

      {/* Error */}
      {error && (
        <div style={{
          fontSize: 12, color: 'var(--crit)',
          padding: '6px 10px', background: 'rgba(239,68,68,.1)', borderRadius: 4,
        }}>
          {error}
        </div>
      )}

      {/* Enable simulation confirmation dialog */}
      {showEnableConfirm && (
        <ConfirmDialog
          title="Enable Simulation"
          message={`Enable simulation mode for${deviceName ? ` "${deviceName}"` : ' this device'}? The device will receive simulated data instead of live PLC values.`}
          confirmLabel="Enable Simulation"
          confirmColor="var(--warn)"
          onConfirm={handleEnableSim}
          onCancel={() => setShowEnableConfirm(false)}
          loading={toggling}
        />
      )}

      {/* Go-live confirmation dialog */}
      {showGoLiveConfirm && (
        <ConfirmDialog
          title="Switch to Live Mode"
          message="Switching to LIVE mode. Device will connect to real PLC."
          confirmLabel="Go Live"
          confirmColor="var(--ok)"
          onConfirm={handleGoLive}
          onCancel={() => setShowGoLiveConfirm(false)}
          loading={toggling}
        />
      )}
    </div>
  )
}
