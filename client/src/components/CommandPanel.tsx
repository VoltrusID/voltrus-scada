import { useState, useEffect, useCallback } from 'react'
import { useNavigate, useRouterState } from '@tanstack/react-router'
import { fmtTs } from '../styles'
import {
  listCommands,
  cancelCommand,
  getCommandAudit,
} from '../api/commands'
import type { Command, CommandAuditEntry } from '../api/commands'

const PANEL_TABS = ['history', 'active'] as const
type PanelTab = typeof PANEL_TABS[number]

const STATUS_FILTERS = ['all', 'pending', 'queued', 'sent', 'acked', 'failed', 'expired', 'cancelled'] as const

const STATUS_COLORS: Record<string, { bg: string; fg: string }> = {
  pending:  { bg: 'rgba(245,158,11,.15)', fg: '#f59e0b' },
  queued:   { bg: 'rgba(59,130,246,.15)',  fg: '#3b82f6' },
  sent:     { bg: 'rgba(249,115,22,.15)',  fg: '#f97316' },
  acked:    { bg: 'rgba(34,197,94,.15)',   fg: '#22c55e' },
  failed:   { bg: 'rgba(239,68,68,.15)',   fg: '#ef4444' },
  expired:  { bg: 'rgba(148,163,184,.15)', fg: '#94a3b8' },
  cancelled:{ bg: 'rgba(148,163,184,.15)', fg: '#94a3b8' },
}

function StatusBadge({ status }: { status: string }) {
  const colors = STATUS_COLORS[status] || STATUS_COLORS.pending
  return (
    <span style={{
      padding: '2px 8px', borderRadius: 4, fontSize: 10,
      fontWeight: 700, textTransform: 'uppercase',
      background: colors.bg, color: colors.fg,
    }}>{status}</span>
  )
}

/* ── Expanded row showing audit trail ─────────────────────────────── */

function AuditTrail({ commandId }: { commandId: string }) {
  const [entries, setEntries] = useState<CommandAuditEntry[] | null>(null)
  const [err, setErr] = useState('')

  useEffect(() => {
    let cancelled = false
    getCommandAudit(commandId)
      .then(data => { if (!cancelled) setEntries(data) })
      .catch(e => { if (!cancelled) setErr(e.message || 'Failed to load audit') })
    return () => { cancelled = true }
  }, [commandId])

  if (err) return <div style={{ fontSize: 11, color: 'var(--crit)', padding: '4px 0' }}>{err}</div>
  if (entries === null) return <div style={{ fontSize: 11, color: 'var(--muted)', padding: '4px 0' }}>Loading audit...</div>
  if (entries.length === 0) return <div style={{ fontSize: 11, color: 'var(--muted)', padding: '4px 0' }}>No audit entries</div>

  return (
    <div style={{ marginTop: 6, paddingLeft: 8, borderLeft: '2px solid var(--border)' }}>
      {entries.map(entry => (
        <div key={entry.id} style={{ fontSize: 11, marginBottom: 4, color: 'var(--muted)' }}>
          <span style={{ color: 'var(--fg)', fontWeight: 600 }}>{entry.old_status}</span>
          {' -> '}
          <span style={{ color: 'var(--fg)', fontWeight: 600 }}>{entry.new_status}</span>
          {' by '}
          <span>{entry.changed_by || 'system'}</span>
          {' at '}
          <span>{fmtTs(entry.created_at * 1000)}</span>
          {entry.reason && <span style={{ marginLeft: 4 }}>({entry.reason})</span>}
        </div>
      ))}
    </div>
  )
}

/* ── Command row ──────────────────────────────────────────────────── */

function CommandRow({ cmd, onExpand, expanded, onCancel, showCancel }: {
  cmd: Command
  onExpand: () => void
  expanded: boolean
  onCancel: () => void
  showCancel: boolean
}) {
  const isActive = cmd.status === 'pending' || cmd.status === 'queued' || cmd.status === 'sent'

  return (
    <div style={{
      padding: '10px 12px', borderRadius: 6, marginBottom: 4,
      border: `1px solid ${isActive ? STATUS_COLORS[cmd.status]?.fg || 'var(--border)' : 'var(--border)'}`,
      background: 'var(--card)',
      cursor: 'pointer',
    }} onClick={onExpand}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <StatusBadge status={cmd.status} />
          <span style={{ fontSize: 12, fontWeight: 600 }}>Tag #{cmd.data_source_id}</span>
        </div>
        <span style={{ fontSize: 10, color: 'var(--muted)' }}>{fmtTs(cmd.created_at * 1000)}</span>
      </div>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginTop: 6 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <span style={{ fontFamily: "var(--font-mono)", fontSize: 13, fontWeight: 700, color: 'var(--cyan)' }}>
            {cmd.value}
          </span>
          <span style={{ fontSize: 10, padding: '1px 6px', borderRadius: 3, background: 'rgba(139,92,246,.1)', color: '#a78bfa', fontWeight: 600 }}>
            {cmd.protocol}
          </span>
          <span style={{ fontSize: 10, color: 'var(--muted)' }}>
            {cmd.policy}
          </span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <span style={{ fontSize: 10, color: 'var(--muted)' }}>by {cmd.created_by}</span>
          {showCancel && isActive && (
            <button
              onClick={e => { e.stopPropagation(); onCancel() }}
              style={{
                padding: '2px 8px', borderRadius: 4, fontSize: 10, cursor: 'pointer',
                border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)',
              }}
            >
              Cancel
            </button>
          )}
        </div>
      </div>
      {cmd.error_message && (
        <div style={{ fontSize: 11, color: 'var(--crit)', marginTop: 4 }}>{cmd.error_message}</div>
      )}
      {expanded && <AuditTrail commandId={cmd.id} />}
    </div>
  )
}

/* ── Command Panel ────────────────────────────────────────────────── */

interface Props {
  onClose: () => void
  isAdmin: boolean
}

export function CommandPanel({ onClose, isAdmin }: Props) {
  const navigate = useNavigate()
  const routerSearch = useRouterState({ select: s => s.location.search }) as { tab?: string }
  const panelTab: PanelTab = (PANEL_TABS as readonly string[]).includes(routerSearch.tab as string) ? routerSearch.tab as PanelTab : 'history'
  const setPanelTab = (t: PanelTab) => navigate({ to: '/commands', search: prev => ({ ...prev, tab: t }) })
  const [statusFilter, setStatusFilter] = useState<string>('all')
  const [history, setHistory] = useState<Command[]>([])
  const [active, setActive] = useState<Command[]>([])
  const [loading, setLoading] = useState(false)
  const [err, setErr] = useState('')
  const [expandedId, setExpandedId] = useState<string | null>(null)
  const [page, setPage] = useState(0)
  const pageSize = 20

  // Fetch history commands
  const fetchHistory = useCallback(async () => {
    try {
      const data = await listCommands({
        status: statusFilter === 'all' ? undefined : statusFilter,
        limit: pageSize,
        offset: page * pageSize,
      })
      setHistory(data)
    } catch (e: any) {
      setErr(e.message || 'Failed to load commands')
    }
  }, [statusFilter, page])

  // Fetch active commands
  const fetchActive = useCallback(async () => {
    try {
      const data = await listCommands({})
      setActive(data.filter(c =>
        c.status === 'pending' || c.status === 'queued' || c.status === 'sent'
      ))
    } catch {
      // silent refresh
    }
  }, [])

  // Initial load and tab-dependent refresh
  useEffect(() => {
    setLoading(true)
    setErr('')
    const load = panelTab === 'history' ? fetchHistory() : fetchActive()
    load.finally(() => setLoading(false))
  }, [panelTab, fetchHistory, fetchActive])

  // Re-fetch history when filter/page changes
  useEffect(() => {
    if (panelTab === 'history') fetchHistory()
  }, [panelTab, statusFilter, page, fetchHistory])

  // Auto-refresh active tab every 2 seconds
  useEffect(() => {
    if (panelTab !== 'active') return
    const interval = setInterval(fetchActive, 2000)
    return () => clearInterval(interval)
  }, [panelTab, fetchActive])

  // Escape key
  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [onClose])

  const handleCancel = async (commandId: string) => {
    try {
      await cancelCommand(commandId)
      setActive(prev => prev.filter(c => c.id !== commandId))
      if (panelTab === 'history') fetchHistory()
    } catch {
      // error handled by refresh
    }
  }

  const commands = panelTab === 'history' ? history : active
  const thStyle: React.CSSProperties = { textAlign: 'left', padding: '8px 12px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600, borderBottom: '1px solid var(--border)' }

  return (
    <div role="dialog" aria-label="Command Panel" style={{
      position: 'fixed', inset: 0, zIndex: 1000,
      display: 'flex', justifyContent: 'flex-end',
    }}>
      <div onClick={onClose} style={{ flex: 1, background: 'rgba(0,0,0,.5)' }} />
      <div style={{
        width: 420, height: '100%',
        background: 'var(--bg)',
        borderLeft: '1px solid var(--border)',
        display: 'flex', flexDirection: 'column', overflow: 'hidden',
      }}>
        {/* Header */}
        <div style={{ padding: '16px 16px 12px', borderBottom: '1px solid var(--border)', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
          <span style={{ fontSize: 15, fontWeight: 700 }}>Commands</span>
          <button onClick={onClose} aria-label="Close command panel" style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', fontSize: 18, padding: '0 4px' }}>&times;</button>
        </div>

        {/* Panel tabs */}
        <div style={{ display: 'flex', borderBottom: '1px solid var(--border)' }}>
          {PANEL_TABS.map(tab => (
            <button key={tab} onClick={() => { setPanelTab(tab); setExpandedId(null) }} aria-label={`Tab: ${tab}`} aria-selected={panelTab === tab} style={{
              flex: 1, padding: '8px 0', fontSize: 11, fontWeight: 600, textTransform: 'uppercase',
              letterSpacing: '.04em', cursor: 'pointer', border: 'none', background: 'transparent',
              borderBottom: panelTab === tab ? '2px solid var(--cyan)' : '2px solid transparent',
              color: panelTab === tab ? 'var(--cyan)' : 'var(--muted)',
            }}>{tab}</button>
          ))}
        </div>

        {/* History tab - status filter */}
        {panelTab === 'history' && (
          <div style={{ display: 'flex', borderBottom: '1px solid var(--border)', overflowX: 'auto' }}>
            {STATUS_FILTERS.map(f => (
              <button key={f} onClick={() => { setStatusFilter(f); setPage(0) }} style={{
                padding: '6px 10px', fontSize: 10, fontWeight: 600, cursor: 'pointer',
                border: 'none', background: 'transparent', whiteSpace: 'nowrap',
                borderBottom: statusFilter === f ? '2px solid var(--cyan)' : '2px solid transparent',
                color: statusFilter === f ? 'var(--cyan)' : 'var(--muted)',
                textTransform: 'uppercase', letterSpacing: '.03em',
              }}>{f}</button>
            ))}
          </div>
        )}

        {/* Active tab summary */}
        {panelTab === 'active' && (
          <div style={{ padding: '8px 16px', borderBottom: '1px solid var(--border)', display: 'flex', gap: 8 }}>
            {['pending', 'queued', 'sent'].map(status => {
              const count = active.filter(c => c.status === status).length
              if (count === 0) return null
              const colors = STATUS_COLORS[status]
              return (
                <div key={status} style={{
                  display: 'flex', alignItems: 'center', gap: 4,
                  padding: '3px 8px', borderRadius: 4, background: colors.bg,
                }}>
                  <span style={{ fontSize: 12, fontWeight: 700, color: colors.fg }}>{count}</span>
                  <span style={{ fontSize: 10, color: colors.fg, opacity: 0.8 }}>{status}</span>
                </div>
              )
            })}
            {active.length === 0 && (
              <span style={{ fontSize: 11, color: 'var(--muted)' }}>No active commands</span>
            )}
          </div>
        )}

        {/* Command list */}
        <div style={{ flex: 1, overflow: 'auto', padding: 8 }}>
          {loading && <div style={{ padding: 20, color: 'var(--muted)', textAlign: 'center', fontSize: 13 }}>Loading...</div>}
          {err && <div style={{ padding: 12, color: 'var(--crit)', textAlign: 'center', fontSize: 12 }}>{err}</div>}
          {!loading && !err && commands.length === 0 && (
            <div style={{ padding: 20, color: 'var(--muted)', textAlign: 'center', fontSize: 13 }}>No commands</div>
          )}
          {!loading && commands.map(cmd => (
            <CommandRow
              key={cmd.id}
              cmd={cmd}
              expanded={expandedId === cmd.id}
              onExpand={() => setExpandedId(expandedId === cmd.id ? null : cmd.id)}
              onCancel={() => handleCancel(cmd.id)}
              showCancel={isAdmin}
            />
          ))}
        </div>

        {/* Pagination for history tab */}
        {panelTab === 'history' && commands.length > 0 && (
          <div style={{
            padding: '8px 16px', borderTop: '1px solid var(--border)',
            display: 'flex', justifyContent: 'space-between', alignItems: 'center',
          }}>
            <button
              onClick={() => setPage(p => Math.max(0, p - 1))}
              disabled={page === 0}
              style={{
                padding: '4px 10px', borderRadius: 4, fontSize: 11, cursor: page === 0 ? 'not-allowed' : 'pointer',
                border: '1px solid var(--border)', background: 'transparent', color: page === 0 ? 'var(--muted)' : 'var(--fg)',
                opacity: page === 0 ? 0.5 : 1,
              }}
            >Previous</button>
            <span style={{ fontSize: 11, color: 'var(--muted)' }}>Page {page + 1}</span>
            <button
              onClick={() => setPage(p => p + 1)}
              disabled={commands.length < pageSize}
              style={{
                padding: '4px 10px', borderRadius: 4, fontSize: 11,
                cursor: commands.length < pageSize ? 'not-allowed' : 'pointer',
                border: '1px solid var(--border)', background: 'transparent',
                color: commands.length < pageSize ? 'var(--muted)' : 'var(--fg)',
                opacity: commands.length < pageSize ? 0.5 : 1,
              }}
            >Next</button>
          </div>
        )}
      </div>
    </div>
  )
}
