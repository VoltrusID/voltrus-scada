import { useState, type CSSProperties } from 'react'
import { useAsset, useUpdateAsset, ASSET_TYPES, CRITICALITY_LEVELS } from '../queries/assets'
import { inputStyle, labelStyle, labelTextStyle, tabStyle } from '../styles'
import { MaintenanceTimeline } from './MaintenanceTimeline'

interface Props {
  assetId: string
  onBack: () => void
}

const TABS = ['Overview', 'Maintenance', 'Firmware', 'Risk'] as const
type Tab = (typeof TABS)[number]

const cardStyle: CSSProperties = {
  background: 'var(--card)',
  border: '1px solid var(--border)',
  borderRadius: 8,
  padding: 16,
}

const fieldStyle: CSSProperties = {
  ...inputStyle,
  background: 'transparent',
  border: '1px solid transparent',
  borderRadius: 4,
  padding: '4px 6px',
  transition: 'border-color 0.15s',
}

const fieldFocusStyle: CSSProperties = {
  border: '1px solid var(--border)',
  background: 'var(--bg)',
}

function riskColor(score: number): string {
  if (score >= 75) return 'var(--crit)'
  if (score >= 50) return 'var(--warn)'
  if (score >= 25) return '#f59e0b'
  return 'var(--ok)'
}

function riskLabel(score: number): string {
  if (score >= 75) return 'Critical'
  if (score >= 50) return 'High'
  if (score >= 25) return 'Medium'
  return 'Low'
}

function computeRiskBreakdown(asset: ReturnType<typeof useAsset>['data']): { label: string; points: number; max: number }[] {
  if (!asset) return []
  const factors: { label: string; points: number; max: number }[] = []

  // Firmware outdated (+30) — no firmware version or unknown
  if (!asset.firmware_version) {
    factors.push({ label: 'Firmware Unknown', points: 30, max: 30 })
  } else {
    factors.push({ label: 'Firmware Recorded', points: 0, max: 30 })
  }

  // No maintenance in 12 months (+20)
  if (asset.last_maintenance) {
    const daysSince = (Date.now() - new Date(asset.last_maintenance).getTime()) / 86400000
    if (daysSince > 365) {
      factors.push({ label: 'Maintenance Overdue (>12mo)', points: 20, max: 20 })
    } else {
      factors.push({ label: 'Maintenance Current', points: 0, max: 20 })
    }
  } else {
    factors.push({ label: 'No Maintenance Record', points: 20, max: 20 })
  }

  // Critical asset unmonitored (+25) — critical/high criticality with no device link
  const isCritical = asset.criticality === 'critical' || asset.criticality === 'high'
  if (isCritical && !asset.device_ref) {
    factors.push({ label: 'Critical Unmonitored', points: 25, max: 25 })
  } else {
    factors.push({ label: 'Monitoring Status', points: 0, max: 25 })
  }

  // Default credentials tag (+25)
  if (asset.tags?.includes('default-credentials')) {
    factors.push({ label: 'Default Credentials', points: 25, max: 25 })
  } else {
    factors.push({ label: 'Credential Check', points: 0, max: 25 })
  }

  return factors
}

export function AssetDetail({ assetId, onBack }: Props) {
  const { data: asset, isLoading } = useAsset(assetId)
  const updateAsset = useUpdateAsset()
  const [activeTab, setActiveTab] = useState<Tab>('Overview')

  if (isLoading) {
    return (
      <div style={{ padding: 24, color: 'var(--muted)', fontSize: 13 }}>
        Loading asset...
      </div>
    )
  }

  if (!asset) {
    return (
      <div style={{ padding: 24, color: 'var(--muted)', fontSize: 13 }}>
        Asset not found.
      </div>
    )
  }

  const handleBlur = (field: string, value: string) => {
    const current = asset as unknown as Record<string, unknown>
    if (current[field] === value) return
    updateAsset.mutate({ id: assetId, [field]: value })
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
        <button
          onClick={onBack}
          style={{
            background: 'none',
            border: '1px solid var(--border)',
            borderRadius: 6,
            color: 'var(--fg)',
            cursor: 'pointer',
            padding: '6px 10px',
            fontSize: 13,
            display: 'flex',
            alignItems: 'center',
            gap: 4,
          }}
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M19 12H5M12 19l-7-7 7-7" />
          </svg>
          Back
        </button>
        <div style={{ fontSize: 20, fontWeight: 700, color: 'var(--fg)' }}>{asset.name}</div>
        {/* Risk score badge */}
        <span style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 4,
          fontSize: 11,
          fontWeight: 600,
          color: riskColor(asset.risk_score),
          background: `${riskColor(asset.risk_score)}18`,
          border: `1px solid ${riskColor(asset.risk_score)}40`,
          borderRadius: 4,
          padding: '2px 8px',
        }}>
          Risk: {asset.risk_score}
        </span>
        {/* Device link badge */}
        {asset.device_ref ? (
          <span style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 4,
            fontSize: 11,
            fontWeight: 500,
            color: 'var(--ok)',
            background: 'rgba(34,197,94,.1)',
            border: '1px solid rgba(34,197,94,.25)',
            borderRadius: 4,
            padding: '2px 8px',
          }}>
            Linked: {asset.device_ref}
          </span>
        ) : (
          <span style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 4,
            fontSize: 11,
            fontWeight: 500,
            color: 'var(--muted)',
            background: 'rgba(255,255,255,.04)',
            border: '1px solid var(--border)',
            borderRadius: 4,
            padding: '2px 8px',
          }}>
            No device link
          </span>
        )}
      </div>

      {/* Tabs */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 0, borderBottom: '1px solid var(--border)' }}>
        {TABS.map(tab => (
          <button key={tab} style={tabStyle(activeTab === tab)} onClick={() => setActiveTab(tab)}>
            {tab}
          </button>
        ))}
      </div>

      {/* Tab content */}
      {activeTab === 'Overview' && (
        <div style={{ ...cardStyle, display: 'flex', flexDirection: 'column', gap: 12 }}>
          <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', marginBottom: 4 }}>
            Asset Information
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(260px, 1fr))', gap: 10 }}>
            <EditableField label="Name" value={asset.name} field="name" onBlur={handleBlur} />
            <div style={labelStyle}>
              <span style={labelTextStyle}>Type</span>
              <select
                defaultValue={asset.asset_type}
                onBlur={e => handleBlur('asset_type', e.target.value)}
                style={{ ...fieldStyle, cursor: 'pointer' }}
              >
                {ASSET_TYPES.map(t => (
                  <option key={t} value={t}>{t.toUpperCase()}</option>
                ))}
              </select>
            </div>
            <EditableField label="Vendor" value={asset.vendor ?? ''} field="vendor" onBlur={handleBlur} />
            <EditableField label="Model" value={asset.model ?? ''} field="model" onBlur={handleBlur} />
            <EditableField label="Serial Number" value={asset.serial_number ?? ''} field="serial_number" onBlur={handleBlur} />
            <EditableField label="IP Address" value={asset.ip_address ?? ''} field="ip_address" onBlur={handleBlur} />
            <EditableField label="MAC Address" value={asset.mac_address ?? ''} field="mac_address" onBlur={handleBlur} />
            <EditableField label="Location" value={asset.location ?? ''} field="location" onBlur={handleBlur} />
            <div style={labelStyle}>
              <span style={labelTextStyle}>Criticality</span>
              <select
                defaultValue={asset.criticality}
                onBlur={e => handleBlur('criticality', e.target.value)}
                style={{ ...fieldStyle, cursor: 'pointer' }}
              >
                {CRITICALITY_LEVELS.map(l => (
                  <option key={l} value={l}>{l.charAt(0).toUpperCase() + l.slice(1)}</option>
                ))}
              </select>
            </div>
          </div>
          <div style={{ ...labelStyle, marginTop: 4 }}>
            <span style={labelTextStyle}>Notes</span>
            <textarea
              key={asset.notes ?? '__empty__'}
              defaultValue={asset.notes ?? ''}
              onBlur={e => handleBlur('notes', e.target.value)}
              rows={3}
              style={{ ...fieldStyle, resize: 'vertical', minHeight: 60, width: '100%', boxSizing: 'border-box' }}
              placeholder="Add notes about this asset..."
            />
          </div>
          {updateAsset.isError && (
            <div style={{ fontSize: 12, color: 'var(--crit)', padding: '6px 10px', background: 'rgba(239,68,68,.1)', borderRadius: 4 }}>
              Failed to save changes.
            </div>
          )}
        </div>
      )}

      {activeTab === 'Maintenance' && (
        <div style={cardStyle}>
          <MaintenanceTimeline assetId={assetId} />
        </div>
      )}

      {activeTab === 'Firmware' && (
        <div style={{ ...cardStyle, display: 'flex', flexDirection: 'column', gap: 16 }}>
          <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', marginBottom: 4 }}>
            Firmware Information
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(200px, 1fr))', gap: 12 }}>
            <InfoBlock label="Current Version" value={asset.firmware_version ?? 'Unknown'} />
            <InfoBlock label="Vendor" value={asset.vendor ?? 'Unknown'} />
            <InfoBlock label="Model" value={asset.model ?? 'Unknown'} />
          </div>
        </div>
      )}

      {activeTab === 'Risk' && (
        <div style={{ ...cardStyle, display: 'flex', flexDirection: 'column', gap: 20 }}>
          {/* Risk score circle */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 24 }}>
            <div style={{
              width: 120,
              height: 120,
              borderRadius: '50%',
              border: `4px solid ${riskColor(asset.risk_score)}`,
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              justifyContent: 'center',
              background: `${riskColor(asset.risk_score)}12`,
              flexShrink: 0,
            }}>
              <div style={{ fontSize: 36, fontWeight: 700, color: riskColor(asset.risk_score), lineHeight: 1 }}>
                {asset.risk_score}
              </div>
              <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', marginTop: 2 }}>
                {riskLabel(asset.risk_score)}
              </div>
            </div>
            <div style={{ fontSize: 13, color: 'var(--muted)', lineHeight: 1.5 }}>
              Composite risk score based on asset criticality, firmware status,
              maintenance recency, device linkage, and documentation completeness.
            </div>
          </div>

          {/* Breakdown factors */}
          <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em' }}>
            Contributing Factors
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            {computeRiskBreakdown(asset).map(factor => {
              const pct = Math.round((factor.points / factor.max) * 100)
              const barColor = pct >= 80 ? 'var(--crit)' : pct >= 50 ? 'var(--warn)' : pct >= 25 ? '#f59e0b' : 'var(--ok)'
              return (
                <div key={factor.label} style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <span style={{ fontSize: 12, color: 'var(--fg)' }}>{factor.label}</span>
                    <span style={{ fontSize: 11, color: 'var(--muted)' }}>
                      {factor.points} / {factor.max} pts
                    </span>
                  </div>
                  <div style={{ height: 6, borderRadius: 3, background: 'var(--border)' }}>
                    <div style={{
                      height: '100%',
                      borderRadius: 3,
                      background: barColor,
                      width: `${pct}%`,
                      transition: 'width 0.3s',
                    }} />
                  </div>
                </div>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}

/* ── Helper components ─────────────────────────────────────────── */

function EditableField({ label, value, field, onBlur }: {
  label: string
  value: string
  field: string
  onBlur: (field: string, value: string) => void
}) {
  return (
    <label style={labelStyle}>
      <span style={labelTextStyle}>{label}</span>
      <input
        key={value}
        type="text"
        defaultValue={value}
        onBlur={e => onBlur(field, e.target.value)}
        onFocus={e => {
          e.target.style.borderColor = 'var(--border)'
          e.target.style.background = 'var(--bg)'
        }}
        style={fieldStyle}
      />
    </label>
  )
}

function InfoBlock({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', marginBottom: 4 }}>
        {label}
      </div>
      <div style={{ fontSize: 15, fontWeight: 600, color: 'var(--fg)' }}>
        {value}
      </div>
    </div>
  )
}
