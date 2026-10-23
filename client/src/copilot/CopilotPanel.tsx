import { useState } from 'react'
import { useAiProposals } from '../queries/copilot'
import { AiSettingsForm } from './AiSettingsForm'
import { ProposalReview } from './ProposalReview'
import { AlarmThresholdWizard } from './AlarmThresholdWizard'
import { RegisterMapWizard } from './RegisterMapWizard'
import { TagRenameWizard } from './TagRenameWizard'
import { DashboardLayoutWizard } from './DashboardLayoutWizard'
import { FlowRuleWizard } from './FlowRuleWizard'
import { TemplateWizard } from './TemplateWizard'
import type { AiProposal } from '../types'

const TABS = ['settings', 'generate', 'proposals'] as const
type Tab = typeof TABS[number]

const GENERATE_TABS = [
  { key: 'register_map', label: 'Register Map' },
  { key: 'alarm_thresholds', label: 'Alarm Thresholds' },
  { key: 'tag_rename', label: 'Tag Rename' },
  { key: 'dashboard_layout', label: 'Dashboard Layout' },
  { key: 'flow_rule', label: 'Flow Rule' },
  { key: 'template', label: 'Templates' },
] as const

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

export function CopilotPanel({ onClose }: { onClose: () => void }) {
  const [tab, setTab] = useState<Tab>('generate')
  const [genTab, setGenTab] = useState<string>('alarm_thresholds')
  const [selectedProposal, setSelectedProposal] = useState<AiProposal | null>(null)
  const [statusFilter, setStatusFilter] = useState<string>('')
  const { data: proposals, isLoading } = useAiProposals(undefined, statusFilter || undefined)

  // If a proposal was just generated, switch to proposals tab
  const goToProposals = () => { setTab('proposals'); setSelectedProposal(null) }

  return (
    <div style={{ position: 'fixed', inset: 0, zIndex: 1000, display: 'flex', justifyContent: 'flex-end' }}>
      <div onClick={onClose} style={{ flex: 1, background: 'rgba(0,0,0,.5)' }} />
      <div style={{ width: 420, maxWidth: '100%', background: 'var(--card)', borderLeft: '1px solid var(--border)', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        {/* Header */}
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '12px 16px', borderBottom: '1px solid var(--border)' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span style={{ fontSize: 18 }}>&#x1f916;</span>
            <h2 style={{ margin: 0, fontSize: 16 }}>AI Copilot</h2>
          </div>
          <button onClick={onClose} aria-label="Close" style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', padding: 4, display: 'flex', alignItems: 'center', justifyContent: 'center', borderRadius: 4, transition: 'color 0.15s', marginLeft: 'auto' }}>
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
          </button>
        </div>

        {/* Main tabs */}
        <div style={{ display: 'flex', borderBottom: '1px solid var(--border)' }}>
          {TABS.map(t => (
            <button key={t} onClick={() => { setTab(t); setSelectedProposal(null) }}
              style={{ flex: 1, padding: '10px 0', border: 'none', background: 'none', color: tab === t ? 'var(--primary)' : 'var(--muted)', fontWeight: tab === t ? 600 : 400, cursor: 'pointer', fontSize: 13, textTransform: 'capitalize', borderBottom: tab === t ? '2px solid var(--primary)' : '2px solid transparent' }}>
              {t}
            </button>
          ))}
        </div>

        {/* Content */}
        <div style={{ flex: 1, overflow: 'auto' }}>
          {tab === 'settings' && <AiSettingsForm />}

          {tab === 'generate' && (
            <div>
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, padding: '8px 12px', borderBottom: '1px solid var(--border)' }}>
                {GENERATE_TABS.map(g => (
                  <button key={g.key} onClick={() => setGenTab(g.key)}
                    style={{ padding: '4px 10px', borderRadius: 4, border: 'none', background: genTab === g.key ? 'var(--primary)' : 'var(--hover)', color: genTab === g.key ? '#fff' : 'var(--fg)', fontSize: 12, cursor: 'pointer' }}>
                    {g.label}
                  </button>
                ))}
              </div>
              {genTab === 'register_map' && <RegisterMapWizard onProposal={goToProposals} />}
              {genTab === 'alarm_thresholds' && <AlarmThresholdWizard onProposal={goToProposals} />}
              {genTab === 'tag_rename' && <TagRenameWizard onProposal={goToProposals} />}
              {genTab === 'dashboard_layout' && <DashboardLayoutWizard onProposal={goToProposals} />}
              {genTab === 'flow_rule' && <FlowRuleWizard onProposal={goToProposals} />}
              {genTab === 'template' && <TemplateWizard onProposal={goToProposals} />}
            </div>
          )}

          {tab === 'proposals' && (
            selectedProposal ? (
              <ProposalReview proposal={selectedProposal} onBack={() => setSelectedProposal(null)} />
            ) : (
              <div style={{ padding: 16 }}>
                <div style={{ display: 'flex', gap: 6, marginBottom: 12 }}>
                  {['', 'pending', 'approved', 'applied'].map(s => (
                    <button key={s} onClick={() => setStatusFilter(s)}
                      style={{ padding: '4px 8px', borderRadius: 4, border: 'none', background: statusFilter === s ? 'var(--primary)' : 'var(--hover)', color: statusFilter === s ? '#fff' : 'var(--fg)', fontSize: 12, cursor: 'pointer' }}>
                      {s || 'All'}
                    </button>
                  ))}
                </div>

                {isLoading && <div style={{ color: 'var(--muted)', fontSize: 13 }}>Loading...</div>}
                {proposals && proposals.length === 0 && <div style={{ color: 'var(--muted)', fontSize: 13, textAlign: 'center', padding: 32 }}>No proposals yet</div>}
                {proposals?.map(p => {
                  const sc = STATUS_COLORS[p.status] || STATUS_COLORS.pending
                  return (
                    <div key={p.id} onClick={() => setSelectedProposal(p)}
                      style={{ padding: 12, border: '1px solid var(--border)', borderRadius: 6, marginBottom: 8, cursor: 'pointer', background: 'var(--bg)' }}>
                      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                        <span style={{ fontSize: 13, fontWeight: 500 }}>{p.title}</span>
                        <span style={{ padding: '2px 6px', borderRadius: 3, fontSize: 10, fontWeight: 600, background: sc.bg, color: sc.fg, textTransform: 'uppercase' }}>{p.status}</span>
                      </div>
                      <div style={{ fontSize: 11, color: 'var(--muted)', marginTop: 4 }}>
                        {p.proposal_type} &middot; {fmtTs(p.created_at)}
                      </div>
                    </div>
                  )
                })}
              </div>
            )
          )}
        </div>
      </div>
    </div>
  )
}
