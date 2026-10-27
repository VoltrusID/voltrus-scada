import { useState, useEffect, useCallback, useRef } from 'react'
import { listExecutions } from '../api/scripts'
import type { ScriptExecution } from '../api/scripts'
import { fmtUnixSecsClock } from '../unixTime'

// ── Types ──────────────────────────────────────────────────────────

interface ScriptExecutionHistoryProps {
  scriptId: number
}

// ── Status badge ───────────────────────────────────────────────────

function StatusBadge({ status }: { status: string }) {
  const color = status === 'success'
    ? 'var(--ok)'
    : status === 'timeout'
      ? 'var(--warn)'
      : 'var(--crit)'

  const bg = status === 'success'
    ? 'rgba(34,197,94,.12)'
    : status === 'timeout'
      ? 'rgba(234,179,8,.12)'
      : 'rgba(239,68,68,.12)'

  return (
    <span style={{
      fontSize: 10, fontWeight: 700, textTransform: 'uppercase',
      padding: '2px 8px', borderRadius: 4, letterSpacing: '.06em',
      background: bg, color, border: `1px solid ${color}`,
    }}>
      {status}
    </span>
  )
}

// ── Main component ─────────────────────────────────────────────────

export function ScriptExecutionHistory({ scriptId }: ScriptExecutionHistoryProps) {
  const [executions, setExecutions] = useState<ScriptExecution[]>([])
  const [loading, setLoading] = useState(true)
  const [expandedId, setExpandedId] = useState<number | null>(null)
  const pollingRef = useRef<ReturnType<typeof setInterval> | null>(null)

  // Fetch executions
  const fetchExecutions = useCallback(async () => {
    try {
      const data = await listExecutions(scriptId, 20)
      setExecutions(prev => {
        // Detect new executions by comparing lengths (most recent first)
        if (prev.length > 0 && data.length > prev.length) {
          // Auto-expand the newest execution
          if (data[0] && data[0].status !== 'success') {
            setExpandedId(data[0].id)
          }
        }
        return data
      })
    } catch {
      // silently fail — history is non-critical
    } finally {
      setLoading(false)
    }
  }, [scriptId])

  useEffect(() => {
    fetchExecutions()
    // Poll every 10 seconds for new executions
    pollingRef.current = setInterval(fetchExecutions, 10_000)
    return () => {
      if (pollingRef.current) clearInterval(pollingRef.current)
    }
  }, [fetchExecutions])

  const toggleExpand = (id: number) => {
    setExpandedId(prev => prev === id ? null : id)
  }

  // ── Render ──────────────────────────────────────────────────────

  if (loading) {
    return (
      <div style={{ fontSize: 11, color: 'var(--muted)', padding: '8px 0' }}>
        Loading execution history...
      </div>
    )
  }

  if (executions.length === 0) {
    return (
      <div style={{ fontSize: 11, color: 'var(--muted)', padding: '8px 0' }}>
        No executions yet.
      </div>
    )
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 0 }}>
      <div style={{
        fontSize: 10, fontWeight: 600, color: 'var(--muted)',
        textTransform: 'uppercase', letterSpacing: '.04em',
        marginBottom: 6,
      }}>
        Recent Executions
      </div>

      <div style={{
        border: '1px solid var(--border)', borderRadius: 6,
        overflow: 'hidden',
      }}>
        {/* Table header */}
        <div style={{
          display: 'grid',
          gridTemplateColumns: '110px 80px 70px 80px 1fr',
          gap: 0,
          fontSize: 10, fontWeight: 600, color: 'var(--muted)',
          textTransform: 'uppercase', letterSpacing: '.04em',
          padding: '6px 10px',
          background: 'rgba(148,163,184,.06)',
          borderBottom: '1px solid var(--border)',
        }}>
          <span>Time</span>
          <span>Status</span>
          <span>Duration</span>
          <span>Trigger</span>
          <span />
        </div>

        {/* Table rows */}
        {executions.map(exec => {
          const isExpanded = expandedId === exec.id
          return (
            <div key={exec.id}>
              <div
                onClick={() => toggleExpand(exec.id)}
                style={{
                  display: 'grid',
                  gridTemplateColumns: '110px 80px 70px 80px 1fr',
                  gap: 0,
                  fontSize: 12, color: 'var(--fg)',
                  padding: '7px 10px',
                  borderBottom: isExpanded ? 'none' : '1px solid var(--border)',
                  cursor: 'pointer',
                  background: isExpanded ? 'rgba(34,211,238,.04)' : 'transparent',
                  alignItems: 'center',
                }}
              >
                <span style={{ fontFamily: 'var(--font-mono)', fontSize: 11 }}>
                  {fmtUnixSecsClock(exec.created_at)}
                </span>
                <span>
                  <StatusBadge status={exec.status} />
                </span>
                <span style={{ fontFamily: 'var(--font-mono)', fontSize: 11, color: 'var(--muted)' }}>
                  {exec.duration_ms}ms
                </span>
                <span style={{ fontSize: 11, color: 'var(--muted)' }}>
                  {exec.trigger_type || 'manual'}
                </span>
                <span style={{
                  fontSize: 10, color: 'var(--muted)', textAlign: 'right',
                }}>
                  {isExpanded ? '\u25B2' : '\u25BC'}
                </span>
              </div>

              {/* Expanded row content */}
              {isExpanded && (
                <div style={{
                  padding: '8px 10px 10px',
                  background: 'rgba(34,211,238,.04)',
                  borderBottom: '1px solid var(--border)',
                  display: 'flex', flexDirection: 'column', gap: 6,
                }}>
                  {exec.error_message && (
                    <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
                      <div style={{ fontSize: 10, fontWeight: 600, color: 'var(--crit)', textTransform: 'uppercase', letterSpacing: '.04em' }}>
                        Error
                      </div>
                      <pre style={{
                        margin: 0, fontSize: 11, fontFamily: 'var(--font-mono)',
                        color: 'var(--crit)', whiteSpace: 'pre-wrap', wordBreak: 'break-word',
                        maxHeight: 120, overflow: 'auto',
                        padding: '6px 8px', background: 'rgba(239,68,68,.06)',
                        borderRadius: 4,
                      }}>
                        {exec.error_message}
                      </pre>
                    </div>
                  )}
                  {exec.output && (
                    <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
                      <div style={{ fontSize: 10, fontWeight: 600, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.04em' }}>
                        Output
                      </div>
                      <pre style={{
                        margin: 0, fontSize: 11, fontFamily: 'var(--font-mono)',
                        color: 'var(--fg)', whiteSpace: 'pre-wrap', wordBreak: 'break-word',
                        maxHeight: 120, overflow: 'auto',
                        padding: '6px 8px', background: 'rgba(148,163,184,.06)',
                        borderRadius: 4,
                      }}>
                        {exec.output}
                      </pre>
                    </div>
                  )}
                  {!exec.error_message && !exec.output && (
                    <div style={{ fontSize: 11, color: 'var(--muted)' }}>
                      No output or error details available.
                    </div>
                  )}
                </div>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}
