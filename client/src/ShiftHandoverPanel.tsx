import { useState, useEffect, useMemo } from 'react'
import { useNavigate, useRouterState } from '@tanstack/react-router'
import { useShiftHandovers, useCreateShiftHandover, useShiftAutoReport } from './queries'
import type { ShiftHandover as ShiftHandoverType } from './queries'
import { btnStyle, inputStyle, labelStyle, labelTextStyle, tabStyle } from './styles'
import { authorLabel } from './authorLabel'

function formatTimestamp(epoch: number): string {
  return new Date(epoch * 1000).toLocaleString('en-US', {
    year: 'numeric', month: 'short', day: '2-digit',
    hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false,
  })
}

function MentionText({ text }: { text: string }) {
  const parts = useMemo(() => {
    const result: { text: string; isMention: boolean }[] = []
    const regex = /(@\w+)/g
    let lastIndex = 0
    let match: RegExpExecArray | null
    while ((match = regex.exec(text)) !== null) {
      if (match.index > lastIndex) {
        result.push({ text: text.slice(lastIndex, match.index), isMention: false })
      }
      result.push({ text: match[1], isMention: true })
      lastIndex = match.index + match[1].length
    }
    if (lastIndex < text.length) {
      result.push({ text: text.slice(lastIndex), isMention: false })
    }
    return result
  }, [text])

  return (
    <span>
      {parts.map((part, i) =>
        part.isMention
          ? <span key={i} style={{ color: '#60a5fa', fontWeight: 600 }}>{part.text}</span>
          : <span key={i}>{part.text}</span>
      )}
    </span>
  )
}

function BriefcaseIcon({ size = 14 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
      <rect x="2" y="7" width="20" height="14" rx="2" ry="2" />
      <path d="M16 21V5a2 2 0 0 0-2-2h-4a2 2 0 0 0-2 2v16" />
    </svg>
  )
}

function PlusIcon({ size = 14 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
      <line x1="12" y1="5" x2="12" y2="19" />
      <line x1="5" y1="12" x2="19" y2="12" />
    </svg>
  )
}

interface HandoverFormState {
  shift_date: string
  shift_period: string
  what_happened: string
  whats_pending: string
  what_to_watch: string
}

const today = () => new Date().toISOString().slice(0, 10)

const emptyForm: HandoverFormState = {
  shift_date: today(),
  shift_period: 'morning',
  what_happened: '',
  whats_pending: '',
  what_to_watch: '',
}

const SHIFT_PERIODS = [
  { value: 'morning', label: 'Morning' },
  { value: 'afternoon', label: 'Afternoon' },
  { value: 'night', label: 'Night' },
]

function HandoverLogTab({ showCreate, onCreateToggle }: { showCreate: boolean; onCreateToggle: () => void }) {
  const [dateFilter, setDateFilter] = useState('')
  const [periodFilter, setPeriodFilter] = useState('')
  const [form, setForm] = useState<HandoverFormState>(emptyForm)

  const queryParams = useMemo(() => ({
    shift_date: dateFilter || undefined,
    shift_period: periodFilter || undefined,
  }), [dateFilter, periodFilter])

  const { data: handoversData, isLoading } = useShiftHandovers(queryParams)
  const handovers: ShiftHandoverType[] = Array.isArray(handoversData) ? handoversData : (handoversData as any)?.handovers ?? []

  const createMut = useCreateShiftHandover()

  const handleCreate = () => {
    if (!form.shift_date || !form.shift_period || !form.what_happened) return
    createMut.mutate(form, {
      onSuccess: () => { setForm({ ...emptyForm, shift_date: today(), shift_period: 'morning' }); onCreateToggle() },
    })
  }

  const cardStyle: React.CSSProperties = {
    background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 8,
    padding: 16, marginBottom: 12,
  }

  return (
    <>
      {/* Create form */}
      {showCreate && (
        <div style={{
          padding: '16px 24px', borderBottom: '1px solid var(--border)',
          background: 'rgba(34,197,94,.04)', flexShrink: 0,
        }}>
          <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 10 }}>New Handover Log</div>
          <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', marginBottom: 10 }}>
            <label style={{ ...labelStyle, flex: '1 1 180px' }}>
              <span style={labelTextStyle}>Shift Date</span>
              <input type="date" value={form.shift_date} onChange={e => setForm({ ...form, shift_date: e.target.value })} style={inputStyle} />
            </label>
            <label style={{ ...labelStyle, flex: '1 1 180px' }}>
              <span style={labelTextStyle}>Shift Period</span>
              <select value={form.shift_period} onChange={e => setForm({ ...form, shift_period: e.target.value })} style={inputStyle}>
                {SHIFT_PERIODS.map(p => <option key={p.value} value={p.value}>{p.label}</option>)}
              </select>
            </label>
          </div>
          <label style={{ ...labelStyle, marginBottom: 10 }}>
            <span style={labelTextStyle}>What Happened</span>
            <textarea value={form.what_happened} onChange={e => setForm({ ...form, what_happened: e.target.value })} placeholder="Summary of events during the shift... Use @mention to tag users" rows={2} style={{ ...inputStyle, resize: 'vertical', minHeight: 48 }} />
          </label>
          <label style={{ ...labelStyle, marginBottom: 10 }}>
            <span style={labelTextStyle}>What's Pending</span>
            <textarea value={form.whats_pending} onChange={e => setForm({ ...form, whats_pending: e.target.value })} placeholder="Tasks or issues still pending..." rows={2} style={{ ...inputStyle, resize: 'vertical', minHeight: 48 }} />
          </label>
          <label style={{ ...labelStyle, marginBottom: 10 }}>
            <span style={labelTextStyle}>What to Watch</span>
            <textarea value={form.what_to_watch} onChange={e => setForm({ ...form, what_to_watch: e.target.value })} placeholder="Things the next shift should be aware of..." rows={2} style={{ ...inputStyle, resize: 'vertical', minHeight: 48 }} />
          </label>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <button onClick={handleCreate} disabled={!form.shift_date || !form.shift_period || !form.what_happened || createMut.isPending} style={{
              ...btnStyle, padding: '5px 10px', borderColor: 'var(--cyan)', color: 'var(--cyan)',
              opacity: (!form.shift_date || !form.shift_period || !form.what_happened || createMut.isPending) ? 0.5 : 1,
            }}>
              {createMut.isPending ? 'Creating...' : 'Create'}
            </button>
            <button onClick={() => { onCreateToggle(); setForm({ ...emptyForm, shift_date: today() }) }} style={{
              ...btnStyle, padding: '5px 10px', color: 'var(--muted)',
            }}>Cancel</button>
            {createMut.isError && <span style={{ fontSize: 12, color: 'var(--crit)' }}>{createMut.error?.message || 'Failed'}</span>}
          </div>
        </div>
      )}

      {/* Filter bar */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 12, padding: '14px 24px',
        borderBottom: '1px solid var(--border)', flexShrink: 0, flexWrap: 'wrap',
      }}>
        <label style={{ ...labelStyle, flex: '1 1 180px' }}>
          <span style={labelTextStyle}>Filter by Date</span>
          <input type="date" value={dateFilter} onChange={e => setDateFilter(e.target.value)} style={inputStyle} />
        </label>
        <label style={{ ...labelStyle, flex: '1 1 180px' }}>
          <span style={labelTextStyle}>Filter by Period</span>
          <select value={periodFilter} onChange={e => setPeriodFilter(e.target.value)} style={inputStyle}>
            <option value="">All</option>
            {SHIFT_PERIODS.map(p => <option key={p.value} value={p.value}>{p.label}</option>)}
          </select>
        </label>
      </div>

      {/* List */}
      <div style={{ flex: 1, overflow: 'auto', padding: '16px 24px' }}>
        {isLoading ? (
          <div style={{ padding: 60, textAlign: 'center', color: 'var(--muted)', fontSize: 14 }}>Loading handover logs...</div>
        ) : handovers.length === 0 ? (
          <div style={{ padding: 60, textAlign: 'center' }}>
            <div style={{ color: 'var(--muted)', fontSize: 14, marginBottom: 8 }}>No handover logs found</div>
            <div style={{ color: 'var(--muted)', fontSize: 12, opacity: 0.7 }}>
              {dateFilter || periodFilter ? 'Try adjusting your filters' : 'Click "New" to create your first handover log'}
            </div>
          </div>
        ) : (
          handovers.map(h => (
            <div key={h.id} style={cardStyle}>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 12 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                  <span style={{
                    display: 'inline-block', padding: '2px 8px', borderRadius: 4,
                    fontSize: 11, fontWeight: 600, background: 'rgba(34,197,94,.12)',
                    color: 'var(--ok)', fontFamily: 'var(--font-mono)',
                  }}>{h.shift_date}</span>
                  <span style={{
                    display: 'inline-block', padding: '2px 8px', borderRadius: 4,
                    fontSize: 11, fontWeight: 600, background: 'rgba(96,165,250,.12)',
                    color: '#60a5fa', textTransform: 'capitalize',
                  }}>{h.shift_period}</span>
                  <span style={{ fontSize: 11, color: 'var(--muted)' }}>by {authorLabel(h.author_username, h.author_id)}</span>
                </div>
                <span style={{ fontSize: 11, color: 'var(--muted)', fontFamily: 'var(--font-mono)' }}>
                  {formatTimestamp(h.created_at)}
                </span>
              </div>
              <div style={{ marginBottom: 8 }}>
                <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600, marginBottom: 4 }}>What Happened</div>
                <div style={{ fontSize: 13, lineHeight: 1.5, whiteSpace: 'pre-wrap' }}><MentionText text={h.what_happened} /></div>
              </div>
              {h.whats_pending && (
                <div style={{ marginBottom: 8 }}>
                  <div style={{ fontSize: 11, color: 'var(--warn)', textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600, marginBottom: 4 }}>Pending</div>
                  <div style={{ fontSize: 13, lineHeight: 1.5, whiteSpace: 'pre-wrap' }}><MentionText text={h.whats_pending} /></div>
                </div>
              )}
              {h.what_to_watch && (
                <div>
                  <div style={{ fontSize: 11, color: 'var(--crit)', textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600, marginBottom: 4 }}>Watch Out</div>
                  <div style={{ fontSize: 13, lineHeight: 1.5, whiteSpace: 'pre-wrap' }}><MentionText text={h.what_to_watch} /></div>
                </div>
              )}
            </div>
          ))
        )}
      </div>
    </>
  )
}

function ReportSection({ title, items, color, type }: { title: string; items: any[]; color: string; type?: 'alarm' | 'annotation' | 'handover' | 'security' }) {
  if (items.length === 0) return null

  const renderItem = (item: any, i: number) => {
    if (typeof item === 'string') {
      return <div key={i} style={{ fontSize: 12, color: 'var(--muted)', padding: '4px 0' }}>{item}</div>
    }

    if (type === 'alarm' || item.severity && item.triggered_at != null) {
      const sevColor = item.severity === 'critical' ? 'var(--crit)' : item.severity === 'warning' ? 'var(--warn)' : 'var(--muted)'
      return (
        <div key={i} style={{ fontSize: 12, color: 'var(--text)', padding: '4px 0', display: 'flex', alignItems: 'baseline', gap: 8 }}>
          <span style={{
            display: 'inline-block', padding: '1px 6px', borderRadius: 4,
            fontSize: 10, fontWeight: 600, background: `${sevColor}20`, color: sevColor,
            textTransform: 'uppercase', flexShrink: 0,
          }}>{item.severity || 'info'}</span>
          <span style={{ flex: 1 }}>{item.message || item.source_name || 'Alarm'}</span>
          <span style={{ fontSize: 11, color: 'var(--muted)', fontFamily: 'var(--font-mono)', flexShrink: 0 }}>
            {formatTimestamp(item.triggered_at)}
          </span>
        </div>
      )
    }

    if (type === 'annotation' || item.annotation_text != null) {
      return (
        <div key={i} style={{ fontSize: 12, color: 'var(--text)', padding: '4px 0', display: 'flex', alignItems: 'baseline', gap: 8 }}>
          <span style={{ fontSize: 11, color: 'var(--cyan)', fontWeight: 600, flexShrink: 0 }}>@{item.author_id}</span>
          <span style={{ flex: 1 }}>{item.annotation_text}</span>
          <span style={{ fontSize: 11, color: 'var(--muted)', fontFamily: 'var(--font-mono)', flexShrink: 0 }}>
            {formatTimestamp(item.created_at)}
          </span>
        </div>
      )
    }

    if (type === 'handover' || item.what_happened != null) {
      return (
        <div key={i} style={{ fontSize: 12, color: 'var(--text)', padding: '4px 0', display: 'flex', alignItems: 'baseline', gap: 8 }}>
          <span style={{ fontSize: 11, color: 'var(--cyan)', fontWeight: 600, flexShrink: 0 }}>@{item.author_username || item.author_id}</span>
          <span style={{ flex: 1 }}>{item.what_happened}</span>
        </div>
      )
    }

    if (type === 'security' || item.event_type != null) {
      const evSevColor = item.severity === 'critical' ? 'var(--crit)' : item.severity === 'warning' ? 'var(--warn)' : 'var(--muted)'
      return (
        <div key={i} style={{ fontSize: 12, color: 'var(--text)', padding: '4px 0', display: 'flex', alignItems: 'baseline', gap: 8 }}>
          <span style={{
            display: 'inline-block', padding: '1px 6px', borderRadius: 4,
            fontSize: 10, fontWeight: 600, background: `${evSevColor}20`, color: evSevColor,
            textTransform: 'uppercase', flexShrink: 0,
          }}>{item.severity}</span>
          <span style={{ flex: 1 }}>{item.details || item.event_type}</span>
          <span style={{ fontSize: 11, color: 'var(--muted)', fontFamily: 'var(--font-mono)', flexShrink: 0 }}>
            {formatTimestamp(item.created_at)}
          </span>
        </div>
      )
    }

    // Fallback for unknown shapes
    return <div key={i} style={{ fontSize: 12, color: 'var(--muted)', padding: '4px 0' }}>{JSON.stringify(item)}</div>
  }

  return (
    <div style={{ marginBottom: 20 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
        <span style={{ fontSize: 13, fontWeight: 600 }}>{title}</span>
        <span style={{
          display: 'inline-block', padding: '1px 6px', borderRadius: 4,
          fontSize: 10, fontWeight: 600, background: `${color}20`, color,
        }}>{items.length}</span>
      </div>
      <div style={{ paddingLeft: 12, borderLeft: `2px solid ${color}40` }}>
        {items.map((item, i) => renderItem(item, i))}
      </div>
    </div>
  )
}

function AutoReportTab() {
  const { data: report, isLoading } = useShiftAutoReport()

  if (isLoading) {
    return <div style={{ padding: 60, textAlign: 'center', color: 'var(--muted)', fontSize: 14 }}>Loading auto report...</div>
  }

  if (!report) {
    return <div style={{ padding: 60, textAlign: 'center', color: 'var(--muted)', fontSize: 14 }}>No report available for the current shift</div>
  }

  const sectionCard: React.CSSProperties = {
    background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 8,
    padding: 16, marginBottom: 12,
  }

  return (
    <div style={{ flex: 1, overflow: 'auto', padding: '16px 24px' }}>
      {/* Report header */}
      <div style={sectionCard}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 8 }}>
          <span style={{
            display: 'inline-block', padding: '2px 8px', borderRadius: 4,
            fontSize: 11, fontWeight: 600, background: 'rgba(34,197,94,.12)',
            color: 'var(--ok)', fontFamily: 'var(--font-mono)',
          }}>{report.shift_date}</span>
          <span style={{
            display: 'inline-block', padding: '2px 8px', borderRadius: 4,
            fontSize: 11, fontWeight: 600, background: 'rgba(96,165,250,.12)',
            color: '#60a5fa', textTransform: 'capitalize',
          }}>{report.shift_period}</span>
          <span style={{ fontSize: 11, color: 'var(--muted)' }}>
            {formatTimestamp(report.period_start)} - {formatTimestamp(report.period_end)}
          </span>
        </div>
      </div>

      {/* Alarms section */}
      <div style={sectionCard}>
        <ReportSection title="Alarms" items={report.alarms || []} color="var(--crit)" type="alarm" />
      </div>

      {/* Annotations section */}
      <div style={sectionCard}>
        <ReportSection title="Annotations" items={report.annotations || []} color="var(--ok)" type="annotation" />
      </div>

      {/* Handover notes section */}
      <div style={sectionCard}>
        <ReportSection title="Handover Notes" items={report.handovers || []} color="#60a5fa" type="handover" />
      </div>

      {/* Security events section */}
      <div style={sectionCard}>
        <ReportSection title="Security Events" items={report.security_events || []} color="var(--warn)" type="security" />
      </div>

      <div style={{ fontSize: 11, color: 'var(--muted)', textAlign: 'center', padding: '8px 0', opacity: 0.6 }}>
        Auto-refreshes every 30 seconds
      </div>
    </div>
  )
}

type TabId = 'log' | 'report'

interface Props {
  onClose: () => void
  embedded?: boolean
}

export function ShiftHandoverPanel({ onClose, embedded }: Props) {
  useEffect(() => {
    if (embedded) return
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, embedded])

  const navigate = useNavigate()
  const routerSearch = useRouterState({ select: s => s.location.search }) as { tab?: string }
  const activeTab: TabId = routerSearch.tab === 'log' || routerSearch.tab === 'report' ? routerSearch.tab : 'log'
  const setActiveTab = (t: TabId) => navigate({ to: '/shift', search: prev => ({ ...prev, tab: t }) })
  const [showCreate, setShowCreate] = useState(false)

  const content = (
    <>
      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        padding: '18px 24px', borderBottom: '1px solid var(--border)', flexShrink: 0,
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <BriefcaseIcon size={20} />
          <span style={{ fontSize: 17, fontWeight: 700 }}>Shift Handover</span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          {activeTab === 'log' && (
            <button onClick={() => setShowCreate(v => !v)} style={{
              ...btnStyle, padding: '5px 10px', borderColor: 'var(--cyan)', color: 'var(--cyan)',
            }} aria-label="Create new handover log">
              <PlusIcon /> New
            </button>
          )}
          {!embedded && (
            <button onClick={onClose} aria-label="Close" style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', padding: 4, display: 'flex', alignItems: 'center', justifyContent: 'center', borderRadius: 4, transition: 'color 0.15s', marginLeft: 'auto' }}>
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
            </button>
          )}
        </div>
      </div>

      {/* Tabs */}
      <div style={{ display: 'flex', borderBottom: '1px solid var(--border)', flexShrink: 0 }}>
        <button style={tabStyle(activeTab === 'log')} onClick={() => setActiveTab('log')}>
          Handover Log
        </button>
        <button style={tabStyle(activeTab === 'report')} onClick={() => setActiveTab('report')}>
          Auto Report
        </button>
      </div>

      {/* Tab content */}
      {activeTab === 'log' && <HandoverLogTab showCreate={showCreate} onCreateToggle={() => setShowCreate(v => !v)} />}
      {activeTab === 'report' && <AutoReportTab />}
    </>
  )

  if (embedded) {
    return <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>{content}</div>
  }

  return (
    <div role="dialog" aria-label="Shift Handover Panel" style={{
      position: 'fixed', inset: 0, zIndex: 1000, background: 'rgba(0,0,0,.6)',
      display: 'flex', alignItems: 'center', justifyContent: 'center',
    }} onClick={onClose}>
      <div style={{
        position: 'relative', width: '92%', maxWidth: 1000, maxHeight: '88vh',
        background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 12,
        display: 'flex', flexDirection: 'column', overflow: 'hidden',
      }} onClick={e => e.stopPropagation()}>
        {content}
      </div>
    </div>
  )
}
