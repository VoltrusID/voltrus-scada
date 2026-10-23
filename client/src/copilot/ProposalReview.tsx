import { useState } from 'react'
import { useApproveProposal, useRejectProposal, useApplyProposal } from '../queries/copilot'
import type { AiProposal } from '../types'

const STATUS_COLORS: Record<string, { bg: string; fg: string }> = {
  pending: { bg: 'rgba(245,158,11,.15)', fg: '#f59e0b' },
  approved: { bg: 'rgba(59,130,246,.15)', fg: '#3b82f6' },
  rejected: { bg: 'rgba(239,68,68,.15)', fg: '#ef4444' },
  applied: { bg: 'rgba(34,197,94,.15)', fg: '#22c55e' },
}

function fmtTs(ts: number | null): string {
  if (!ts) return '-'
  return new Date(ts * 1000).toLocaleString()
}

export function ProposalReview({ proposal, onBack }: { proposal: AiProposal; onBack: () => void }) {
  const approve = useApproveProposal()
  const reject = useRejectProposal()
  const apply = useApplyProposal()
  const [editJson, setEditJson] = useState('')

  const isPending = proposal.status === 'pending'
  const isApproved = proposal.status === 'approved'
  const config = proposal.applied_config || proposal.proposed_config
  const sc = STATUS_COLORS[proposal.status] || STATUS_COLORS.pending

  const handleApprove = () => {
    const body: { id: number; applied_config?: any } = { id: proposal.id }
    if (editJson.trim()) {
      try { body.applied_config = JSON.parse(editJson) } catch { /* use original */ }
    }
    approve.mutate(body, { onSuccess: onBack })
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12, padding: 16 }}>
      <button onClick={onBack} style={{ background: 'none', border: 'none', color: 'var(--primary)', cursor: 'pointer', fontSize: 13, padding: 0, textAlign: 'left', marginBottom: 4 }}>
        &larr; Back to proposals
      </button>

      <h3 style={{ margin: 0, fontSize: 16 }}>{proposal.title}</h3>

      <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
        <span style={{ padding: '2px 8px', borderRadius: 4, fontSize: 11, fontWeight: 600, background: sc.bg, color: sc.fg, textTransform: 'uppercase' }}>{proposal.status}</span>
        <span style={{ padding: '2px 8px', borderRadius: 4, fontSize: 11, background: 'var(--hover)', color: 'var(--muted)' }}>{proposal.proposal_type}</span>
        {proposal.model_used && <span style={{ padding: '2px 8px', borderRadius: 4, fontSize: 11, background: 'var(--hover)', color: 'var(--muted)' }}>{proposal.model_used}</span>}
      </div>

      <div style={{ fontSize: 12, color: 'var(--muted)' }}>
        <div>Created: {fmtTs(proposal.created_at)}</div>
        {proposal.reviewed_at && <div>Reviewed: {fmtTs(proposal.reviewed_at)} by {proposal.reviewed_by}</div>}
        {proposal.applied_at && <div>Applied: {fmtTs(proposal.applied_at)}</div>}
        {proposal.tokens_used != null && proposal.tokens_used > 0 && <div>Tokens: {proposal.tokens_used}</div>}
      </div>

      {proposal.error_message && (
        <div style={{ padding: 8, borderRadius: 6, background: 'rgba(239,68,68,.1)', color: '#ef4444', fontSize: 13 }}>{proposal.error_message}</div>
      )}

      {/* Config preview */}
      <div>
        <label style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 4, display: 'block' }}>Proposed Configuration</label>
        <pre style={{ background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 6, padding: 12, fontSize: 12, overflow: 'auto', maxHeight: 300, margin: 0 }}>
          {JSON.stringify(config, null, 2)}
        </pre>
      </div>

      {/* Inline edit for pending proposals */}
      {isPending && (
        <div>
          <label style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 4, display: 'block' }}>Edit config (optional)</label>
          <textarea value={editJson} onChange={e => setEditJson(e.target.value)} placeholder="Paste modified JSON here..."
            rows={6} style={{ width: '100%', padding: 8, borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)', fontSize: 12, fontFamily: 'monospace', boxSizing: 'border-box', resize: 'vertical' }} />
        </div>
      )}

      {/* Action buttons */}
      <div style={{ display: 'flex', gap: 8, marginTop: 4 }}>
        {isPending && (
          <>
            <button onClick={handleApprove} disabled={approve.isPending}
              style={{ flex: 1, padding: '8px 16px', borderRadius: 6, border: 'none', background: '#22c55e', color: '#fff', fontWeight: 600, cursor: 'pointer' }}>
              {approve.isPending ? 'Approving...' : 'Approve'}
            </button>
            <button onClick={() => reject.mutate(proposal.id, { onSuccess: onBack })} disabled={reject.isPending}
              style={{ flex: 1, padding: '8px 16px', borderRadius: 6, border: '1px solid #ef4444', background: 'transparent', color: '#ef4444', cursor: 'pointer' }}>
              {reject.isPending ? 'Rejecting...' : 'Reject'}
            </button>
          </>
        )}
        {isApproved && (
          <button onClick={() => apply.mutate(proposal.id, { onSuccess: onBack })} disabled={apply.isPending}
            style={{ flex: 1, padding: '10px 16px', borderRadius: 6, border: 'none', background: 'var(--primary)', color: '#fff', fontWeight: 600, cursor: 'pointer', fontSize: 14 }}>
            {apply.isPending ? 'Applying...' : 'Apply to Production'}
          </button>
        )}
      </div>
    </div>
  )
}
