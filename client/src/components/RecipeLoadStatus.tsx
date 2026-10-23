import { useState, useEffect, useCallback, useRef } from 'react'
import { getCommand, writeTag } from '../api/commands'
import type { Command } from '../api/commands'

const STATUS_COLORS: Record<string, { bg: string; fg: string }> = {
  pending:  { bg: 'rgba(245,158,11,.15)', fg: '#f59e0b' },
  queued:   { bg: 'rgba(59,130,246,.15)',  fg: '#3b82f6' },
  sent:     { bg: 'rgba(249,115,22,.15)',  fg: '#f97316' },
  acked:    { bg: 'rgba(34,197,94,.15)',   fg: '#22c55e' },
  failed:   { bg: 'rgba(239,68,68,.15)',   fg: '#ef4444' },
  expired:  { bg: 'rgba(148,163,184,.15)', fg: '#94a3b8' },
  cancelled:{ bg: 'rgba(148,163,184,.15)', fg: '#94a3b8' },
}

const TERMINAL_STATES = new Set(['acked', 'failed', 'expired', 'cancelled'])

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

interface Props {
  commandIds: string[]
  onComplete: () => void
}

export function RecipeLoadStatus({ commandIds, onComplete }: Props) {
  const [commands, setCommands] = useState<Map<string, Command>>(new Map())
  const [loading, setLoading] = useState(true)
  const [retrying, setRetrying] = useState<Set<string>>(new Set())
  const [errors, setErrors] = useState<Map<string, string>>(new Map())
  const completeCalledRef = useRef(false)

  const fetchCommands = useCallback(async () => {
    const results = new Map<string, Command>()
    let allTerminal = true

    for (const id of commandIds) {
      try {
        const cmd = await getCommand(id)
        results.set(id, cmd)
        if (!TERMINAL_STATES.has(cmd.status)) allTerminal = false
      } catch (e: any) {
        results.set(id, commands.get(id)!)
        allTerminal = false
      }
    }

    setCommands(results)
    setLoading(false)

    if (allTerminal && !completeCalledRef.current && results.size === commandIds.length) {
      completeCalledRef.current = true
      onComplete()
    }
  }, [commandIds, onComplete, commands])

  // Initial fetch + polling every 2 seconds
  useEffect(() => {
    fetchCommands()
    const interval = setInterval(fetchCommands, 2000)
    return () => clearInterval(interval)
  }, [fetchCommands])

  const handleRetry = async (commandId: string) => {
    const cmd = commands.get(commandId)
    if (!cmd) return

    setRetrying(prev => new Set(prev).add(commandId))
    setErrors(prev => { const next = new Map(prev); next.delete(commandId); return next })

    try {
      await writeTag(cmd.data_source_id, parseFloat(cmd.value), cmd.policy)
      // Re-fetch after retry
      await fetchCommands()
    } catch (e: any) {
      setErrors(prev => new Map(prev).set(commandId, e.message || 'Retry failed'))
    } finally {
      setRetrying(prev => { const next = new Set(prev); next.delete(commandId); return next })
    }
  }

  const commandList = commandIds.map(id => commands.get(id)).filter(Boolean) as Command[]
  const successCount = commandList.filter(c => c.status === 'acked').length
  const failedCount = commandList.filter(c => c.status === 'failed').length
  const total = commandIds.length
  const allComplete = commandList.length === total && commandList.every(c => TERMINAL_STATES.has(c.status))

  if (loading) {
    return (
      <div style={{
        background: 'var(--card)', border: '1px solid var(--border)',
        borderRadius: 8, padding: 20, textAlign: 'center',
      }}>
        <div style={{ fontSize: 13, color: 'var(--muted)' }}>Loading command status...</div>
      </div>
    )
  }

  return (
    <div style={{
      background: 'var(--card)', border: '1px solid var(--border)',
      borderRadius: 8, overflow: 'hidden',
    }}>
      {/* Header */}
      <div style={{
        padding: '12px 16px', borderBottom: '1px solid var(--border)',
        display: 'flex', justifyContent: 'space-between', alignItems: 'center',
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <span style={{ fontSize: 13, fontWeight: 600 }}>Recipe Load Status</span>
          {!allComplete && (
            <span style={{
              width: 8, height: 8, borderRadius: '50%',
              background: 'var(--cyan)', animation: 'pulse 1.5s infinite',
            }} />
          )}
        </div>
        <button
          onClick={onComplete}
          disabled={!allComplete}
          style={{
            padding: '4px 12px', borderRadius: 4, fontSize: 11, cursor: allComplete ? 'pointer' : 'not-allowed',
            border: '1px solid var(--border)', background: 'transparent',
            color: allComplete ? 'var(--fg)' : 'var(--muted)', opacity: allComplete ? 1 : 0.5,
          }}
        >
          {allComplete ? 'Close' : 'Running...'}
        </button>
      </div>

      {/* Summary */}
      <div style={{
        padding: '10px 16px', borderBottom: '1px solid var(--border)',
        display: 'flex', gap: 12, alignItems: 'center',
        background: successCount === total
          ? 'rgba(34,197,94,.05)'
          : failedCount > 0
            ? 'rgba(239,68,68,.05)'
            : 'transparent',
      }}>
        <span style={{ fontSize: 12, fontWeight: 600 }}>
          {successCount}/{total} parameters written successfully
        </span>
        {failedCount > 0 && (
          <span style={{ fontSize: 11, color: 'var(--crit)' }}>
            {failedCount} failed
          </span>
        )}
      </div>

      {/* Command table */}
      <div style={{ maxHeight: 300, overflow: 'auto' }}>
        {commandIds.map(id => {
          const cmd = commands.get(id)
          if (!cmd) return (
            <div key={id} style={{
              padding: '8px 16px', borderBottom: '1px solid var(--border)',
              display: 'flex', justifyContent: 'space-between', alignItems: 'center',
            }}>
              <span style={{ fontSize: 11, color: 'var(--muted)', fontFamily: "var(--font-mono)" }}>{id.slice(0, 8)}...</span>
              <span style={{ fontSize: 11, color: 'var(--muted)' }}>Loading...</span>
            </div>
          )

          const isFailed = cmd.status === 'failed'
          const isRetrying = retrying.has(id)
          const retryError = errors.get(id)

          return (
            <div key={id} style={{
              padding: '8px 16px', borderBottom: '1px solid var(--border)',
              display: 'flex', justifyContent: 'space-between', alignItems: 'center',
            }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <StatusBadge status={cmd.status} />
                <span style={{ fontSize: 12, fontWeight: 500 }}>
                  Tag #{cmd.data_source_id}
                </span>
                <span style={{
                  fontFamily: "var(--font-mono)", fontSize: 12,
                  color: 'var(--cyan)', fontWeight: 600,
                }}>
                  {cmd.value}
                </span>
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                {(cmd.error_message || retryError) && (
                  <span style={{ fontSize: 10, color: 'var(--crit)', maxWidth: 200, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {retryError || cmd.error_message}
                  </span>
                )}
                {isFailed && !isRetrying && (
                  <button
                    onClick={() => handleRetry(id)}
                    style={{
                      padding: '2px 8px', borderRadius: 4, fontSize: 10, cursor: 'pointer',
                      border: '1px solid var(--warn)', background: 'transparent',
                      color: 'var(--warn)', fontWeight: 600,
                    }}
                  >
                    Retry
                  </button>
                )}
                {isRetrying && (
                  <span style={{ fontSize: 10, color: 'var(--muted)' }}>Retrying...</span>
                )}
              </div>
            </div>
          )
        })}
      </div>
    </div>
  )
}
