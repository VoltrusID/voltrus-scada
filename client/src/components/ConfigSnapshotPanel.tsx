import { useState, useEffect, useCallback } from 'react'
import { btnStyle, inputStyle, labelStyle, labelTextStyle } from '../styles'
import { listSnapshots, createSnapshot, diffSnapshots, rollback } from '../api/config-snapshots'
import type { ConfigSnapshot, ConfigDiff } from '../api/config-snapshots'

// ── Trigger badge ───────────────────────────────────────────────────

function TriggerBadge({ trigger }: { trigger: string }) {
  const color = trigger === 'manual'
    ? 'var(--ok)'
    : trigger === 'restore'
      ? '#f97316'
      : '#3b82f6'

  const bg = trigger === 'manual'
    ? 'rgba(34,197,94,.12)'
    : trigger === 'restore'
      ? 'rgba(249,115,22,.12)'
      : 'rgba(59,130,246,.12)'

  return (
    <span style={{
      fontSize: 10, fontWeight: 700, textTransform: 'uppercase',
      padding: '2px 8px', borderRadius: 4, letterSpacing: '.06em',
      background: bg, color, border: `1px solid ${color}`,
    }}>
      {trigger}
    </span>
  )
}

// ── Format timestamp ────────────────────────────────────────────────

function formatTimestamp(ts: number): string {
  const d = new Date(ts)
  return d.toLocaleDateString('en-US', {
    month: 'short', day: 'numeric', year: 'numeric',
  }) + ' ' + d.toLocaleTimeString('en-US', {
    hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit',
  })
}

// ── Diff viewer ─────────────────────────────────────────────────────

function DiffViewer({ diff }: { diff: ConfigDiff }) {
  const tableNames = Object.keys(diff)

  if (tableNames.length === 0) {
    return (
      <div style={{ padding: 24, textAlign: 'center', color: 'var(--muted)', fontSize: 13 }}>
        No differences found between these snapshots.
      </div>
    )
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      {tableNames.map(table => {
        const section = diff[table]
        const hasChanges = (section.added.length > 0) || (section.removed.length > 0) || (section.changed.length > 0)
        if (!hasChanges) return null

        return (
          <div key={table} style={{
            background: 'var(--bg)', border: '1px solid var(--border)',
            borderRadius: 6, overflow: 'hidden',
          }}>
            {/* Table header */}
            <div style={{
              padding: '8px 12px', fontSize: 12, fontWeight: 700,
              color: 'var(--fg)', borderBottom: '1px solid var(--border)',
              background: 'rgba(255,255,255,.03)',
              display: 'flex', justifyContent: 'space-between', alignItems: 'center',
            }}>
              <span>{table}</span>
              <span style={{ fontSize: 10, color: 'var(--muted)' }}>
                +{section.added.length} -{section.removed.length} ~{section.changed.length}
              </span>
            </div>

            <div style={{ padding: 8, maxHeight: 300, overflow: 'auto' }}>
              {/* Added rows */}
              {section.added.map((item, i) => (
                <DiffLine key={`a-${i}`} type="added" content={JSON.stringify(item, null, 2)} />
              ))}

              {/* Removed rows */}
              {section.removed.map((item, i) => (
                <DiffLine key={`r-${i}`} type="removed" content={JSON.stringify(item, null, 2)} />
              ))}

              {/* Changed rows */}
              {section.changed.map((item, i) => (
                <div key={`c-${i}`} style={{ marginBottom: 6 }}>
                  <div style={{ fontSize: 10, color: 'var(--muted)', marginBottom: 2, fontFamily: "var(--font-mono)" }}>
                    id: {item.id}
                  </div>
                  <DiffLine type="removed" content={JSON.stringify(item.from, null, 2)} />
                  <DiffLine type="added" content={JSON.stringify(item.to, null, 2)} />
                </div>
              ))}
            </div>
          </div>
        )
      })}
    </div>
  )
}

function DiffLine({ type, content }: { type: 'added' | 'removed' | 'changed'; content: string }) {
  const color = type === 'added'
    ? 'var(--ok)'
    : type === 'removed'
      ? 'var(--crit)'
      : 'var(--warn)'

  const bg = type === 'added'
    ? 'rgba(34,197,94,.06)'
    : type === 'removed'
      ? 'rgba(239,68,68,.06)'
      : 'rgba(234,179,8,.06)'

  return (
    <pre style={{
      margin: 0, padding: '6px 10px', fontSize: 11, lineHeight: 1.5,
      fontFamily: "var(--font-mono)",
      background: bg, color, borderRadius: 4,
      whiteSpace: 'pre-wrap', wordBreak: 'break-all',
      borderLeft: `3px solid ${color}`,
    }}>
      {content}
    </pre>
  )
}

// ── Confirm rollback dialog ─────────────────────────────────────────

function ConfirmRollbackDialog({
  snapshot,
  onConfirm,
  onCancel,
  loading,
}: {
  snapshot: ConfigSnapshot
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
        borderRadius: 10, padding: 20, maxWidth: 460, width: '90%',
        display: 'flex', flexDirection: 'column', gap: 14,
      }}>
        <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--fg)' }}>Confirm Rollback</div>
        <div style={{ fontSize: 13, color: 'var(--muted)', lineHeight: 1.6 }}>
          This will restore the configuration to the snapshot taken on{' '}
          <span style={{ color: 'var(--fg)' }}>{formatTimestamp(snapshot.created_at)}</span>.
          {snapshot.label && (
            <> Snapshot: <span style={{ color: 'var(--fg)' }}>"{snapshot.label}"</span></>
          )}
          <br />
          <span style={{ color: 'var(--crit)', fontWeight: 600 }}>
            This will overwrite the current configuration. Make sure you have a backup snapshot.
          </span>
        </div>
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <button
            onClick={onCancel}
            disabled={loading}
            style={{
              ...btnStyle, border: '1px solid var(--border)', color: 'var(--muted)',
              opacity: loading ? 0.5 : 1, cursor: loading ? 'not-allowed' : 'pointer',
            }}
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            disabled={loading}
            style={{
              ...btnStyle,
              border: '1px solid var(--warn)',
              background: 'rgba(234,179,8,.15)',
              color: 'var(--warn)',
              opacity: loading ? 0.5 : 1,
              cursor: loading ? 'not-allowed' : 'pointer',
            }}
          >
            {loading ? 'Rolling back...' : 'Rollback'}
          </button>
        </div>
      </div>
    </div>
  )
}

// ── Snapshot row ────────────────────────────────────────────────────

function SnapshotRow({
  snapshot,
  onRollback,
}: {
  snapshot: ConfigSnapshot
  onRollback: (s: ConfigSnapshot) => void
}) {
  return (
    <div style={{
      padding: '10px 12px', borderRadius: 6, marginBottom: 4,
      border: '1px solid var(--border)', background: 'var(--card)',
      display: 'flex', justifyContent: 'space-between', alignItems: 'center',
    }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
          <span style={{ fontSize: 12, color: 'var(--muted)', fontFamily: "var(--font-mono)" }}>
            {formatTimestamp(snapshot.created_at)}
          </span>
          <TriggerBadge trigger={snapshot.trigger} />
        </div>
        {(snapshot.label || snapshot.description) && (
          <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--fg)', marginTop: 4, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {snapshot.label || snapshot.description}
          </div>
        )}
        {snapshot.created_by && (
          <div style={{ fontSize: 11, color: 'var(--muted)', marginTop: 2 }}>
            by {snapshot.created_by}
          </div>
        )}
      </div>
      <button
        onClick={() => onRollback(snapshot)}
        style={{
          ...btnStyle, fontSize: 11, padding: '4px 10px',
          border: '1px solid var(--warn)', color: 'var(--warn)',
          background: 'transparent', flexShrink: 0,
        }}
      >
        Rollback
      </button>
    </div>
  )
}

// ── Main component ──────────────────────────────────────────────────

export function ConfigSnapshotPanel() {
  const [snapshots, setSnapshots] = useState<ConfigSnapshot[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')

  // Create snapshot
  const [showCreate, setShowCreate] = useState(false)
  const [newLabel, setNewLabel] = useState('')
  const [creating, setCreating] = useState(false)

  // Diff viewer
  const [diffFromId, setDiffFromId] = useState<number | ''>('')
  const [diffToId, setDiffToId] = useState<number | ''>('')
  const [diff, setDiff] = useState<ConfigDiff | null>(null)
  const [diffLoading, setDiffLoading] = useState(false)
  const [diffError, setDiffError] = useState('')

  // Rollback
  const [rollbackTarget, setRollbackTarget] = useState<ConfigSnapshot | null>(null)
  const [rollbackLoading, setRollbackLoading] = useState(false)

  // Fetch snapshots
  const fetchSnapshots = useCallback(async () => {
    try {
      const data = await listSnapshots()
      setSnapshots(data)
      setError('')
    } catch (e: any) {
      setError(e.message || 'Failed to load snapshots')
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    fetchSnapshots()
  }, [fetchSnapshots])

  // Handle create snapshot
  const handleCreate = async () => {
    setCreating(true)
    setError('')
    try {
      await createSnapshot(newLabel.trim() || undefined)
      setShowCreate(false)
      setNewLabel('')
      await fetchSnapshots()
    } catch (e: any) {
      setError(e.message || 'Failed to create snapshot')
    } finally {
      setCreating(false)
    }
  }

  // Handle diff
  const handleDiff = async () => {
    if (diffFromId === '' || diffToId === '') return
    setDiffLoading(true)
    setDiffError('')
    setDiff(null)
    try {
      const result = await diffSnapshots(Number(diffFromId), Number(diffToId))
      setDiff(result)
    } catch (e: any) {
      setDiffError(e.message || 'Failed to compute diff')
    } finally {
      setDiffLoading(false)
    }
  }

  // Handle rollback
  const handleRollback = async () => {
    if (!rollbackTarget) return
    setRollbackLoading(true)
    setError('')
    try {
      await rollback(rollbackTarget.id)
      setRollbackTarget(null)
      await fetchSnapshots()
    } catch (e: any) {
      setError(e.message || 'Failed to rollback')
    } finally {
      setRollbackLoading(false)
    }
  }

  // Snapshot options for dropdowns (newest first)
  const snapshotOptions = snapshots.slice().reverse()

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
        <span style={{ fontSize: 15, fontWeight: 700 }}>Config Snapshots</span>
        <button
          onClick={() => { setShowCreate(true); setNewLabel('') }}
          disabled={showCreate}
          style={{
            ...btnStyle,
            border: '1px solid var(--cyan)',
            background: showCreate ? 'transparent' : 'var(--cyan)',
            color: showCreate ? 'var(--muted)' : '#fff',
            cursor: showCreate ? 'not-allowed' : 'pointer',
            opacity: showCreate ? 0.5 : 1,
            fontSize: 12, padding: '5px 12px',
          }}
        >
          + Create Snapshot
        </button>
      </div>

      {/* Content */}
      <div style={{ flex: 1, overflow: 'auto', display: 'flex', flexDirection: 'column' }}>
        {/* Error banner */}
        {error && (
          <div style={{
            padding: '8px 12px', fontSize: 12, color: 'var(--crit)',
            background: 'rgba(239,68,68,.1)', borderBottom: '1px solid rgba(239,68,68,.2)',
          }}>
            {error}
          </div>
        )}

        {/* Create snapshot form */}
        {showCreate && (
          <div style={{
            padding: 12, borderBottom: '1px solid var(--border)',
            display: 'flex', gap: 8, alignItems: 'flex-end',
            background: 'rgba(0,0,0,.2)',
          }}>
            <label style={{ ...labelStyle, flex: 1 }}>
              <span style={labelTextStyle}>Label (optional)</span>
              <input
                type="text"
                value={newLabel}
                onChange={e => setNewLabel(e.target.value)}
                placeholder="e.g. Before flow refactor"
                style={inputStyle}
                autoFocus
                onKeyDown={e => { if (e.key === 'Enter') handleCreate() }}
              />
            </label>
            <button
              onClick={handleCreate}
              disabled={creating}
              style={{
                ...btnStyle,
                border: '1px solid var(--cyan)',
                background: creating ? 'transparent' : 'var(--cyan)',
                color: creating ? 'var(--muted)' : '#fff',
                cursor: creating ? 'not-allowed' : 'pointer',
                opacity: creating ? 0.5 : 1,
              }}
            >
              {creating ? 'Creating...' : 'Save'}
            </button>
            <button
              onClick={() => setShowCreate(false)}
              disabled={creating}
              style={{
                ...btnStyle,
                border: '1px solid var(--border)', color: 'var(--muted)',
                cursor: creating ? 'not-allowed' : 'pointer',
                opacity: creating ? 0.5 : 1,
              }}
            >
              Cancel
            </button>
          </div>
        )}

        {/* Diff viewer controls */}
        <div style={{
          padding: '10px 12px', borderBottom: '1px solid var(--border)',
          display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap',
          background: 'rgba(0,0,0,.15)',
        }}>
          <span style={{ fontSize: 11, color: 'var(--muted)', fontWeight: 600, textTransform: 'uppercase', letterSpacing: '.04em' }}>
            Diff
          </span>
          <select
            value={diffFromId}
            onChange={e => setDiffFromId(e.target.value === '' ? '' : Number(e.target.value))}
            style={{
              ...inputStyle, width: 'auto', minWidth: 140, fontSize: 11,
              appearance: 'none',
              background: 'var(--bg) url("data:image/svg+xml,%3Csvg xmlns=%27http://www.w3.org/2000/svg%27 width=%2712%27 height=%2712%27 viewBox=%270 0 24 24%27 fill=%27none%27 stroke=%27%2394a3b8%27 stroke-width=%272%27%3E%3Cpolyline points=%276 9 12 15 18 9%27/%3E%3C/svg%3E") no-repeat right 6px center',
              paddingRight: 24,
            }}
          >
            <option value="">From snapshot...</option>
            {snapshotOptions.map(s => (
              <option key={s.id} value={s.id}>
                #{s.id} — {formatTimestamp(s.created_at)}{s.label ? ` (${s.label})` : ''}
              </option>
            ))}
          </select>
          <span style={{ fontSize: 11, color: 'var(--muted)' }}>→</span>
          <select
            value={diffToId}
            onChange={e => setDiffToId(e.target.value === '' ? '' : Number(e.target.value))}
            style={{
              ...inputStyle, width: 'auto', minWidth: 140, fontSize: 11,
              appearance: 'none',
              background: 'var(--bg) url("data:image/svg+xml,%3Csvg xmlns=%27http://www.w3.org/2000/svg%27 width=%2712%27 height=%2712%27 viewBox=%270 0 24 24%27 fill=%27none%27 stroke=%27%2394a3b8%27 stroke-width=%272%27%3E%3Cpolyline points=%276 9 12 15 18 9%27/%3E%3C/svg%3E") no-repeat right 6px center',
              paddingRight: 24,
            }}
          >
            <option value="">To snapshot...</option>
            {snapshotOptions.map(s => (
              <option key={s.id} value={s.id}>
                #{s.id} — {formatTimestamp(s.created_at)}{s.label ? ` (${s.label})` : ''}
              </option>
            ))}
          </select>
          <button
            onClick={handleDiff}
            disabled={diffFromId === '' || diffToId === '' || diffLoading}
            style={{
              ...btnStyle, fontSize: 11, padding: '5px 12px',
              border: '1px solid var(--cyan)', color: 'var(--cyan)',
              background: 'transparent',
              cursor: (diffFromId === '' || diffToId === '' || diffLoading) ? 'not-allowed' : 'pointer',
              opacity: (diffFromId === '' || diffToId === '' || diffLoading) ? 0.5 : 1,
            }}
          >
            {diffLoading ? 'Comparing...' : 'Compare'}
          </button>
          {diff && (
            <button
              onClick={() => { setDiff(null); setDiffError('') }}
              style={{
                ...btnStyle, fontSize: 11, padding: '5px 10px',
                border: '1px solid var(--border)', color: 'var(--muted)',
                background: 'transparent',
              }}
            >
              Clear
            </button>
          )}
        </div>

        {/* Diff result */}
        {diffError && (
          <div style={{
            padding: '8px 12px', fontSize: 12, color: 'var(--crit)',
            background: 'rgba(239,68,68,.08)',
          }}>
            {diffError}
          </div>
        )}
        {diff && (
          <div style={{ padding: 8, borderBottom: '1px solid var(--border)' }}>
            <DiffViewer diff={diff} />
          </div>
        )}

        {/* Snapshot list */}
        <div style={{ flex: 1, overflow: 'auto', padding: 8 }}>
          {loading && (
            <div style={{ padding: 20, color: 'var(--muted)', textAlign: 'center', fontSize: 13 }}>
              Loading snapshots...
            </div>
          )}
          {!loading && snapshots.length === 0 && !error && (
            <div style={{ padding: 20, color: 'var(--muted)', textAlign: 'center', fontSize: 13 }}>
              No snapshots yet. Click "Create Snapshot" to save the current configuration.
            </div>
          )}
          {!loading && snapshots.map(snapshot => (
            <SnapshotRow
              key={snapshot.id}
              snapshot={snapshot}
              onRollback={setRollbackTarget}
            />
          ))}
        </div>
      </div>

      {/* Rollback confirmation dialog */}
      {rollbackTarget && (
        <ConfirmRollbackDialog
          snapshot={rollbackTarget}
          onConfirm={handleRollback}
          onCancel={() => setRollbackTarget(null)}
          loading={rollbackLoading}
        />
      )}
    </div>
  )
}
