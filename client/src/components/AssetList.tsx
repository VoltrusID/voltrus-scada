import { useState, useMemo } from 'react'
import { useAssets, useDeleteAsset, ASSET_TYPES, CRITICALITY_LEVELS } from '../queries/assets'
import type { Asset } from '../queries/assets'

/* ── Props ────────────────────────────────────────────────────────── */

interface Props {
  onSelect: (id: string) => void
  onScan: () => void
  onImport: () => void
  onExport: () => void
}

/* ── Color helpers ────────────────────────────────────────────────── */

function riskColor(score: number): string {
  if (score <= 25) return '#22c55e'
  if (score <= 50) return '#eab308'
  if (score <= 75) return '#f97316'
  return '#ef4444'
}

const CRITICALITY_COLORS: Record<string, { bg: string; fg: string }> = {
  low:      { bg: 'rgba(34,197,94,.15)',   fg: '#22c55e' },
  medium:   { bg: 'rgba(59,130,246,.15)',  fg: '#3b82f6' },
  high:     { bg: 'rgba(249,115,22,.15)',  fg: '#f97316' },
  critical: { bg: 'rgba(239,68,68,.15)',   fg: '#ef4444' },
}

/* ── Inline styles ────────────────────────────────────────────────── */

const wrapper: React.CSSProperties = {
  display: 'flex', flexDirection: 'column', gap: 12,
}

const toolbar: React.CSSProperties = {
  display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: 8,
}

const selectStyle: React.CSSProperties = {
  background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 6,
  padding: '6px 8px', color: 'var(--fg)', fontSize: 12, fontFamily: 'inherit',
  cursor: 'pointer',
}

const searchStyle: React.CSSProperties = {
  background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 6,
  padding: '6px 10px', color: 'var(--fg)', fontSize: 12, fontFamily: 'inherit',
  flex: '1 1 180px', minWidth: 140,
}

const actionBtn: React.CSSProperties = {
  padding: '6px 12px', borderRadius: 6, fontSize: 12, fontWeight: 600,
  cursor: 'pointer', border: '1px solid var(--border)', background: 'var(--card)',
  color: 'var(--fg)', display: 'flex', alignItems: 'center', gap: 4,
  whiteSpace: 'nowrap',
}

const tableContainer: React.CSSProperties = {
  background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 8,
  overflow: 'auto',
}

const thStyle: React.CSSProperties = {
  textAlign: 'left', padding: '10px 12px', fontSize: 11,
  color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em',
  fontWeight: 600, borderBottom: '1px solid var(--border)',
  whiteSpace: 'nowrap',
}

const tdStyle: React.CSSProperties = {
  padding: '10px 12px', fontSize: 12, color: 'var(--fg)',
  borderBottom: '1px solid var(--border)', whiteSpace: 'nowrap',
}

const rowStyle: React.CSSProperties = {
  cursor: 'pointer', transition: 'background 0.15s',
}

const badge: React.CSSProperties = {
  padding: '2px 8px', borderRadius: 4, fontSize: 10,
  fontWeight: 700, textTransform: 'uppercase', display: 'inline-block',
}

const deleteBtn: React.CSSProperties = {
  padding: '4px 8px', borderRadius: 4, fontSize: 11, cursor: 'pointer',
  border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)',
}

const emptyStyle: React.CSSProperties = {
  padding: 40, textAlign: 'center', color: 'var(--muted)', fontSize: 13,
}

const loadingStyle: React.CSSProperties = {
  padding: 40, textAlign: 'center', color: 'var(--muted)', fontSize: 13,
}

/* ── Component ────────────────────────────────────────────────────── */

export function AssetList({ onSelect, onScan, onImport, onExport }: Props) {
  const [typeFilter, setTypeFilter] = useState('')
  const [critFilter, setCritFilter] = useState('')
  const [search, setSearch] = useState('')
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null)

  const filters = useMemo(() => ({
    ...(typeFilter ? { asset_type: typeFilter } : {}),
    ...(critFilter ? { criticality: critFilter } : {}),
    ...(search ? { search } : {}),
  }), [typeFilter, critFilter, search])

  const { data: assets, isLoading, error } = useAssets(filters)
  const deleteAsset = useDeleteAsset()

  const handleDelete = (e: React.MouseEvent, id: string) => {
    e.stopPropagation()
    if (confirmDeleteId === id) {
      deleteAsset.mutate(id)
      setConfirmDeleteId(null)
    } else {
      setConfirmDeleteId(id)
    }
  }

  const handleRowClick = (asset: Asset) => {
    setConfirmDeleteId(null)
    onSelect(asset.id)
  }

  return (
    <div style={wrapper}>
      {/* ── Filter toolbar ─────────────────────────────────────────── */}
      <div style={toolbar}>
        <select
          value={typeFilter}
          onChange={e => setTypeFilter(e.target.value)}
          style={selectStyle}
          aria-label="Filter by type"
        >
          <option value="">All Types</option>
          {ASSET_TYPES.map(t => (
            <option key={t} value={t}>{t.toUpperCase()}</option>
          ))}
        </select>

        <select
          value={critFilter}
          onChange={e => setCritFilter(e.target.value)}
          style={selectStyle}
          aria-label="Filter by criticality"
        >
          <option value="">All Criticality</option>
          {CRITICALITY_LEVELS.map(c => (
            <option key={c} value={c}>{c.charAt(0).toUpperCase() + c.slice(1)}</option>
          ))}
        </select>

        <input
          type="text"
          placeholder="Search assets..."
          value={search}
          onChange={e => setSearch(e.target.value)}
          style={searchStyle}
          aria-label="Search assets"
        />

        <div style={{ marginLeft: 'auto', display: 'flex', gap: 6 }}>
          <button onClick={onScan} style={{ ...actionBtn, borderColor: 'var(--cyan)', color: 'var(--cyan)' }}>
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>
            Scan Network
          </button>
          <button onClick={onImport} style={actionBtn}>
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/></svg>
            Import CSV
          </button>
          <button onClick={onExport} style={actionBtn}>
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
            NIS2 Export
          </button>
        </div>
      </div>

      {/* ── Table ──────────────────────────────────────────────────── */}
      <div style={tableContainer}>
        <table style={{ width: '100%', borderCollapse: 'collapse' }}>
          <thead>
            <tr>
              <th style={thStyle}>Name</th>
              <th style={thStyle}>Type</th>
              <th style={thStyle}>Vendor</th>
              <th style={thStyle}>IP</th>
              <th style={thStyle}>Criticality</th>
              <th style={thStyle}>Risk Score</th>
              <th style={thStyle}>Firmware</th>
              <th style={thStyle}>Last Maintenance</th>
              <th style={thStyle}></th>
            </tr>
          </thead>
          <tbody>
            {isLoading && (
              <tr><td colSpan={9} style={loadingStyle}>Loading assets...</td></tr>
            )}
            {error && (
              <tr><td colSpan={9} style={{ ...emptyStyle, color: 'var(--crit)' }}>Failed to load assets</td></tr>
            )}
            {!isLoading && !error && assets && assets.length === 0 && (
              <tr><td colSpan={9} style={emptyStyle}>No assets found. Add one manually or scan your network.</td></tr>
            )}
            {!isLoading && assets?.map(asset => {
              const critColors = CRITICALITY_COLORS[asset.criticality] || CRITICALITY_COLORS.low
              const rc = riskColor(asset.risk_score)
              const isConfirming = confirmDeleteId === asset.id

              return (
                <tr
                  key={asset.id}
                  onClick={() => handleRowClick(asset)}
                  onMouseEnter={e => { (e.currentTarget as HTMLElement).style.background = 'var(--bg)' }}
                  onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = 'transparent' }}
                  style={rowStyle}
                >
                  <td style={{ ...tdStyle, fontWeight: 600 }}>{asset.name}</td>
                  <td style={{ ...tdStyle, textTransform: 'uppercase', fontSize: 10, color: 'var(--muted)' }}>
                    {asset.asset_type}
                  </td>
                  <td style={tdStyle}>{asset.vendor ?? '-'}</td>
                  <td style={{ ...tdStyle, fontFamily: "var(--font-mono)", fontSize: 11 }}>
                    {asset.ip_address ?? '-'}
                  </td>
                  <td style={tdStyle}>
                    <span style={{ ...badge, background: critColors.bg, color: critColors.fg }}>
                      {asset.criticality}
                    </span>
                  </td>
                  <td style={tdStyle}>
                    <span style={{
                      display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
                      width: 28, height: 28, borderRadius: '50%',
                      background: `${rc}22`, color: rc,
                      fontSize: 11, fontWeight: 700, fontFamily: "var(--font-mono)",
                    }}>
                      {asset.risk_score}
                    </span>
                  </td>
                  <td style={{ ...tdStyle, fontFamily: "var(--font-mono)", fontSize: 11, color: 'var(--muted)' }}>
                    {asset.firmware_version ?? '-'}
                  </td>
                  <td style={{ ...tdStyle, fontSize: 11, color: 'var(--muted)' }}>
                    {asset.last_maintenance
                      ? new Date(asset.last_maintenance).toLocaleDateString()
                      : '-'
                    }
                  </td>
                  <td style={tdStyle}>
                    <button
                      onClick={e => handleDelete(e, asset.id)}
                      style={{
                        ...deleteBtn,
                        ...(isConfirming ? { background: 'var(--crit)', color: '#fff' } : {}),
                      }}
                    >
                      {isConfirming ? 'Confirm?' : 'Delete'}
                    </button>
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
    </div>
  )
}
