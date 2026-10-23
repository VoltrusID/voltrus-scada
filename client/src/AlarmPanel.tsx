import { useEffect, useRef, useState, useMemo } from 'react'
import { useNavigate, useRouterState } from '@tanstack/react-router'
import { useVirtualizer } from '@tanstack/react-virtual'
import {
  useAlarms,
  useAcknowledgeAlarm,
  useAcknowledgeAllAlarms,
  useAlarmSummary,
  useAlarmAudit,
  useAlarmAnalytics,
  useShelfAlarm,
  useUnshelfAlarm,
  useAlarmJournal,
} from './queries'
import { fmtTs } from './styles'
import type { Alarm } from './types'

const FILTER_TABS = ['all', 'active', 'acknowledged', 'shelved'] as const
const PANEL_TABS = ['alarms', 'analytics', 'audit'] as const
type PanelTab = typeof PANEL_TABS[number]

const PRIORITY_COLORS: Record<string, { bg: string; fg: string }> = {
  critical: { bg: 'rgba(239,68,68,.15)', fg: '#ef4444' },
  warning:  { bg: 'rgba(245,158,11,.15)', fg: '#f59e0b' },
  info:     { bg: 'rgba(59,130,246,.15)', fg: '#3b82f6' },
}

const SHELF_DURATIONS = [
  { label: '15 min', secs: 900 },
  { label: '30 min', secs: 1800 },
  { label: '1 hr',   secs: 3600 },
  { label: '4 hr',   secs: 14400 },
  { label: '8 hr',   secs: 28800 },
]

const useIsMobile = () => {
  const [mobile, setMobile] = useState(window.innerWidth <= 768)
  useEffect(() => {
    const mq = window.matchMedia('(max-width: 768px)')
    const handler = (e: MediaQueryListEvent) => setMobile(e.matches)
    mq.addEventListener('change', handler)
    return () => mq.removeEventListener('change', handler)
  }, [])
  return mobile
}

function formatDuration(secs: number): string {
  if (secs < 60) return `${secs}s`
  if (secs < 3600) return `${Math.floor(secs / 60)}m`
  const h = Math.floor(secs / 3600)
  const m = Math.floor((secs % 3600) / 60)
  return m > 0 ? `${h}h ${m}m` : `${h}h`
}

/* ── Summary header badges ─────────────────────────────────────── */

function SummaryHeader({ isMobile }: { isMobile: boolean }) {
  const { data: summary, isLoading } = useAlarmSummary()

  if (isLoading || !summary) return null

  const badges = [
    { label: 'Critical', value: summary.active_critical, color: PRIORITY_COLORS.critical },
    { label: 'Warning',  value: summary.active_warning,  color: PRIORITY_COLORS.warning },
    { label: 'Info',     value: summary.active_info,     color: PRIORITY_COLORS.info },
    { label: 'Shelved',  value: summary.shelved,         color: { bg: 'rgba(148,163,184,.15)', fg: '#94a3b8' } },
  ]

  return (
    <div style={{
      display: 'flex', gap: isMobile ? 6 : 8, flexWrap: 'wrap',
      padding: '8px 16px', borderBottom: '1px solid var(--border)',
    }}>
      {badges.map(b => (
        <div key={b.label} style={{
          display: 'flex', alignItems: 'center', gap: 4,
          padding: isMobile ? '4px 8px' : '3px 8px',
          borderRadius: 4, background: b.color.bg,
        }}>
          <span style={{ fontSize: isMobile ? 14 : 12, fontWeight: 700, color: b.color.fg }}>{b.value}</span>
          <span style={{ fontSize: isMobile ? 11 : 10, color: b.color.fg, opacity: 0.8 }}>{b.label}</span>
        </div>
      ))}
      {summary.recent_24h > 0 && (
        <div style={{
          padding: isMobile ? '4px 8px' : '3px 8px', borderRadius: 4,
          background: 'rgba(148,163,184,.08)',
          fontSize: isMobile ? 11 : 10, color: 'var(--muted)',
          display: 'flex', alignItems: 'center', gap: 4,
        }}>
          <span style={{ fontWeight: 700 }}>{summary.recent_24h}</span> last 24h
        </div>
      )}
    </div>
  )
}

/* ── Shelf form mini-dialog ────────────────────────────────────── */

function ShelfForm({ alarmId, onClose }: { alarmId: number; onClose: () => void }) {
  const [duration, setDuration] = useState(1800)
  const [reason, setReason] = useState('')
  const shelfMut = useShelfAlarm()

  const handleSubmit = () => {
    shelfMut.mutate(
      { id: alarmId, duration_secs: duration, reason },
      { onSuccess: onClose },
    )
  }

  return (
    <div style={{
      padding: 8, borderRadius: 6, border: '1px solid var(--border)',
      background: 'var(--card)', marginTop: 6,
    }}>
      <div style={{ fontSize: 11, fontWeight: 600, marginBottom: 6, color: 'var(--fg)' }}>Shelf Alarm</div>
      <div style={{ display: 'flex', gap: 6, marginBottom: 6 }}>
        {SHELF_DURATIONS.map(d => (
          <button
            key={d.secs}
            onClick={() => setDuration(d.secs)}
            style={{
              padding: '4px 8px', borderRadius: 4, fontSize: 10, cursor: 'pointer',
              border: `1px solid ${duration === d.secs ? 'var(--cyan)' : 'var(--border)'}`,
              background: duration === d.secs ? 'rgba(6,182,212,.15)' : 'transparent',
              color: duration === d.secs ? 'var(--cyan)' : 'var(--muted)',
            }}
          >{d.label}</button>
        ))}
      </div>
      <input
        type="text"
        placeholder="Reason (optional)"
        value={reason}
        onChange={e => setReason(e.target.value)}
        style={{
          width: '100%', padding: '6px 8px', borderRadius: 4,
          border: '1px solid var(--border)', background: 'var(--bg)',
          color: 'var(--fg)', fontSize: 11, boxSizing: 'border-box',
          marginBottom: 6,
        }}
      />
      <div style={{ display: 'flex', gap: 6, justifyContent: 'flex-end' }}>
        <button onClick={onClose} style={{
          padding: '4px 10px', borderRadius: 4, fontSize: 10, cursor: 'pointer',
          border: '1px solid var(--border)', background: 'transparent', color: 'var(--muted)',
        }}>Cancel</button>
        <button onClick={handleSubmit} disabled={shelfMut.isPending} style={{
          padding: '4px 10px', borderRadius: 4, fontSize: 10, cursor: 'pointer',
          border: '1px solid var(--cyan)', background: 'var(--cyan)', color: '#fff',
          opacity: shelfMut.isPending ? 0.6 : 1,
        }}>Shelf</button>
      </div>
    </div>
  )
}

/* ── Swipeable alarm row (mobile only) ─────────────────────────── */

function SwipeableAlarmRow({ alarm, isMobile, onAck, children }: {
  alarm: { id: number; acknowledged_at?: number | null }
  isMobile: boolean
  onAck: () => void
  children: React.ReactNode
}) {
  const touchStartX = useRef(0)
  const currentX = useRef(0)
  const rowRef = useRef<HTMLDivElement>(null)

  // Skip wrapper on desktop or already-acknowledged alarms
  if (!isMobile || alarm.acknowledged_at) return <>{children}</>

  const onTouchStart = (e: React.TouchEvent) => {
    touchStartX.current = e.touches[0].clientX
    currentX.current = 0
  }

  const onTouchMove = (e: React.TouchEvent) => {
    const deltaX = e.touches[0].clientX - touchStartX.current
    if (deltaX < 0) return // only swipe right
    const clampedX = Math.min(deltaX, 120)
    currentX.current = clampedX
    if (rowRef.current) {
      rowRef.current.style.transform = `translateX(${clampedX}px)`
      rowRef.current.style.transition = 'none'
    }
  }

  const onTouchEnd = () => {
    if (currentX.current > 80) {
      onAck()
    }
    // Animate back
    if (rowRef.current) {
      rowRef.current.style.transition = 'transform 0.2s ease'
      rowRef.current.style.transform = 'translateX(0)'
    }
    currentX.current = 0
  }

  return (
    <div style={{ position: 'relative', overflow: 'hidden', marginBottom: 8, borderRadius: 8 }}>
      {/* Green ack label behind */}
      <div style={{
        position: 'absolute', inset: 0, display: 'flex', alignItems: 'center',
        justifyContent: 'flex-end', paddingRight: 20,
        background: 'rgba(34,197,94,.2)', borderRadius: 8, color: 'var(--ok)',
        fontWeight: 700, fontSize: 14,
      }}>
        Ack &#10003;
      </div>
      {/* Sliding card */}
      <div
        ref={rowRef}
        onTouchStart={onTouchStart}
        onTouchMove={onTouchMove}
        onTouchEnd={onTouchEnd}
        style={{ position: 'relative', zIndex: 1 }}
      >
        {children}
      </div>
    </div>
  )
}

/* ── Priority badge component ──────────────────────────────────── */

function PriorityBadge({ priority, isMobile }: { priority: string; isMobile: boolean }) {
  const colors = PRIORITY_COLORS[priority] || PRIORITY_COLORS.info
  return (
    <span style={{
      padding: isMobile ? '3px 10px' : '2px 8px',
      borderRadius: 4, fontSize: isMobile ? 12 : 10,
      fontWeight: 700, textTransform: 'uppercase',
      background: colors.bg, color: colors.fg,
    }}>{priority}</span>
  )
}

/* ── Analytics tab ─────────────────────────────────────────────── */

function AnalyticsTab({ isMobile }: { isMobile: boolean }) {
  const now = useMemo(() => Date.now(), [])
  const { data: analytics = [], isLoading } = useAlarmAnalytics({
    from: Math.floor((now - 24 * 3600 * 1000) / 1000),
    to: Math.floor(now / 1000),
  })
  const { data: summary } = useAlarmSummary()

  const top10 = analytics.slice(0, 10)
  const maxCount = top10.length > 0 ? Math.max(...top10.map(a => a.trigger_count), 1) : 1

  // Alarm flood detection: >10 alarms in 5 min
  const fiveMinAgo = Math.floor(now / 1000) - 300
  const recentAlarms = top10.filter(a => a.last_triggered_at > fiveMinAgo)
  const recentTotal = recentAlarms.reduce((s, a) => s + a.trigger_count, 0)
  const floodDetected = recentTotal > 10

  if (isLoading) {
    return <div style={{ padding: 20, color: 'var(--muted)', textAlign: 'center', fontSize: 13 }}>Loading analytics...</div>
  }

  return (
    <div style={{ padding: isMobile ? 12 : 8, overflow: 'auto', flex: 1 }}>
      {/* Flood warning */}
      {floodDetected && (
        <div style={{
          padding: '10px 12px', borderRadius: 6, marginBottom: 12,
          background: 'rgba(239,68,68,.12)', border: '1px solid #ef4444',
          color: '#ef4444', fontSize: 12, fontWeight: 600,
          display: 'flex', alignItems: 'center', gap: 8,
        }}>
          <span style={{ fontSize: 16 }}>&#9888;</span>
          Alarm flood detected — {recentTotal} triggers in the last 5 minutes
        </div>
      )}

      {/* Avg ack time */}
      {summary && summary.avg_ack_time_secs > 0 && (
        <div style={{
          padding: '8px 12px', borderRadius: 6, marginBottom: 12,
          background: 'var(--card)', border: '1px solid var(--border)',
          fontSize: 12, color: 'var(--fg)',
        }}>
          Avg time to acknowledge: <strong>{formatDuration(Math.round(summary.avg_ack_time_secs))}</strong>
        </div>
      )}

      {/* Top 10 table */}
      <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--muted)', marginBottom: 8, textTransform: 'uppercase', letterSpacing: '.04em' }}>
        Top 10 Most Frequent (24h)
      </div>
      {top10.length === 0 && (
        <div style={{ color: 'var(--muted)', fontSize: 13, textAlign: 'center', padding: 20 }}>No alarm data</div>
      )}
      {top10.map((entry, i) => (
        <div key={entry.source_key} style={{
          display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6,
          padding: '6px 8px', borderRadius: 4, background: 'var(--card)',
        }}>
          <span style={{ fontSize: 10, color: 'var(--muted)', width: 16, textAlign: 'right' }}>{i + 1}.</span>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 11, fontWeight: 600, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{entry.source_name}</div>
            <div style={{ height: 4, borderRadius: 2, background: 'var(--border)', marginTop: 3 }}>
              <div style={{
                height: '100%', borderRadius: 2,
                width: `${(entry.trigger_count / maxCount) * 100}%`,
                background: 'var(--cyan)',
              }} />
            </div>
          </div>
          <span style={{ fontSize: 11, fontWeight: 700, color: 'var(--cyan)', minWidth: 28, textAlign: 'right' }}>{entry.trigger_count}</span>
        </div>
      ))}
    </div>
  )
}

/* ── Audit tab ─────────────────────────────────────────────────── */

function AuditTab({ isMobile }: { isMobile: boolean }) {
  const { data: entries = [], isLoading } = useAlarmAudit()

  if (isLoading) {
    return <div style={{ padding: 20, color: 'var(--muted)', textAlign: 'center', fontSize: 13 }}>Loading audit trail...</div>
  }

  if (entries.length === 0) {
    return <div style={{ padding: 20, color: 'var(--muted)', textAlign: 'center', fontSize: 13 }}>No audit entries</div>
  }

  return (
    <div style={{ padding: isMobile ? 12 : 8, overflow: 'auto', flex: 1 }}>
      {entries.map(entry => (
        <div key={entry.id} style={{
          padding: '8px 10px', borderRadius: 6, marginBottom: 6,
          border: '1px solid var(--border)', background: 'var(--card)',
        }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 4 }}>
            <span style={{
              fontSize: 10, fontWeight: 700, textTransform: 'uppercase',
              padding: '2px 6px', borderRadius: 3,
              background: 'rgba(6,182,212,.12)', color: 'var(--cyan)',
            }}>{entry.action}</span>
            <span style={{ fontSize: 10, color: 'var(--muted)' }}>{fmtTs(entry.created_at * 1000)}</span>
          </div>
          <div style={{ fontSize: 11, color: 'var(--fg)', marginBottom: 2 }}>
            Alarm #{entry.alarm_id}
          </div>
          {entry.details && (
            <div style={{ fontSize: 11, color: 'var(--muted)', marginBottom: 2 }}>{entry.details}</div>
          )}
          <div style={{ fontSize: 10, color: 'var(--muted)' }}>
            by {entry.performed_by || 'system'}
          </div>
        </div>
      ))}
    </div>
  )
}

/* ── Per-alarm ISA-18.2 journal (History) ──────────────────────── */

function AlarmHistory({ alarmId }: { alarmId: number }) {
  const { data: entries = [], isLoading } = useAlarmJournal(alarmId)

  if (isLoading) {
    return <div style={{ marginTop: 8, fontSize: 10, color: 'var(--muted)' }}>Loading history...</div>
  }
  if (entries.length === 0) {
    return <div style={{ marginTop: 8, fontSize: 10, color: 'var(--muted)' }}>No history</div>
  }

  return (
    <div style={{ marginTop: 8, paddingTop: 6, borderTop: '1px solid var(--border)' }}>
      <div style={{
        fontSize: 9, fontWeight: 700, textTransform: 'uppercase',
        letterSpacing: '.04em', color: 'var(--muted)', marginBottom: 4,
      }}>History</div>
      {entries.map(entry => (
        <div key={entry.id} style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 10, marginBottom: 2 }}>
          <span style={{ color: 'var(--muted)', minWidth: 52 }}>{fmtTs(entry.at * 1000)}</span>
          <span style={{ fontWeight: 600, color: 'var(--fg)' }}>{entry.event}</span>
          {entry.actor && <span style={{ color: 'var(--muted)' }}>by {entry.actor}</span>}
        </div>
      ))}
    </div>
  )
}

/* ── Alarm panel ──────────────────────────────────────────────── */

interface Props {
  onClose: () => void
}

export function AlarmPanel({ onClose }: Props) {
  // Inject pulse animation once
  if (typeof document !== 'undefined' && !document.getElementById('alarm-pulse-style')) {
    const style = document.createElement('style')
    style.id = 'alarm-pulse-style'
    style.textContent = '@keyframes pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.4; } }'
    document.head.appendChild(style)
  }

  const isMobile = useIsMobile()
  const [filter, setFilter] = useState<string>('active')
  const navigate = useNavigate()
  const routerSearch = useRouterState({ select: s => s.location.search }) as { tab?: string }
  const panelTab: PanelTab = (PANEL_TABS as readonly string[]).includes(routerSearch.tab as string) ? routerSearch.tab as PanelTab : 'alarms'
  const setPanelTab = (t: PanelTab) => navigate({ to: '/alarms', search: prev => ({ ...prev, tab: t }) })
  const [shelfTarget, setShelfTarget] = useState<number | null>(null)
  const [historyTarget, setHistoryTarget] = useState<number | null>(null)
  const { data: alarms = [], isLoading } = useAlarms(filter)
  const ackMut = useAcknowledgeAlarm()
  const ackAllMut = useAcknowledgeAllAlarms()
  const unshelfMut = useUnshelfAlarm()

  const parentRef = useRef<HTMLDivElement>(null)
  const virtualizer = useVirtualizer({
    count: alarms.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => isMobile ? 120 : 100,
    overscan: 5,
    measureElement: (el) => el?.getBoundingClientRect().height ?? (isMobile ? 120 : 100),
  })

  // Escape key closes the panel
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [onClose])

  // Mobile-friendly card sizing
  const cardPad = isMobile ? 16 : 12
  const sevBadgeSize = isMobile ? 12 : 10
  const sourceSize = isMobile ? 15 : 13
  const msgSize = isMobile ? 13 : 12
  const timeSize = isMobile ? 13 : 11

  return (
    <div role="dialog" aria-label="Alarm Panel" style={{
      position: 'fixed', inset: 0, zIndex: 1000,
      display: 'flex',
      justifyContent: isMobile ? 'center' : 'flex-end',
    }}>
      {/* Backdrop – hidden on mobile (panel is full-width, close via X button) */}
      {!isMobile && <div onClick={onClose} style={{ flex: 1, background: 'rgba(0,0,0,.5)' }} />}
      {/* Panel */}
      <div style={{
        width: isMobile ? '100%' : 360,
        height: '100%',
        background: 'var(--bg)',
        borderLeft: isMobile ? 'none' : '1px solid var(--border)',
        display: 'flex', flexDirection: 'column', overflow: 'hidden',
      }}>
        {/* Header */}
        <div style={{ padding: '12px 16px 10px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center' }}>
          <span style={{ fontSize: isMobile ? 17 : 15, fontWeight: 700 }}>Alarms</span>
          <button onClick={() => ackAllMut.mutate()} aria-label="Acknowledge all alarms" style={{ marginLeft: 10, padding: '5px 10px', borderRadius: 6, border: '1px solid var(--warn)', background: 'transparent', color: 'var(--warn)', cursor: 'pointer', fontSize: 11, fontWeight: 600 }}>Ack All</button>
          <button onClick={onClose} aria-label="Close" style={{ marginLeft: 'auto', background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', padding: 4, display: 'flex', alignItems: 'center', justifyContent: 'center', borderRadius: 4, transition: 'color 0.15s' }}>
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
          </button>
        </div>

        {/* Summary header badges */}
        <SummaryHeader isMobile={isMobile} />

        {/* Panel tabs: Alarms | Analytics | Audit */}
        <div style={{ display: 'flex', borderBottom: '1px solid var(--border)' }}>
          {PANEL_TABS.map(tab => (
            <button key={tab} onClick={() => setPanelTab(tab)} aria-label={`Tab: ${tab}`} aria-selected={panelTab === tab} style={{
              flex: 1, padding: isMobile ? '10px 0' : '8px 0', fontSize: 11, fontWeight: 600, textTransform: 'uppercase',
              letterSpacing: '.04em', cursor: 'pointer', border: 'none', background: 'transparent',
              borderBottom: panelTab === tab ? '2px solid var(--cyan)' : '2px solid transparent',
              color: panelTab === tab ? 'var(--cyan)' : 'var(--muted)',
            }}>{tab}</button>
          ))}
        </div>

        {/* Alarms tab content */}
        {panelTab === 'alarms' && (
          <>
            {/* Filter tabs */}
            <div style={{ display: 'flex', borderBottom: '1px solid var(--border)' }}>
              {FILTER_TABS.map(tab => (
                <button key={tab} onClick={() => setFilter(tab)} aria-label={`Filter: ${tab}`} aria-selected={filter === tab} style={{
                  flex: 1, padding: isMobile ? '10px 0' : '8px 0', fontSize: 11, fontWeight: 600, textTransform: 'uppercase',
                  letterSpacing: '.04em', cursor: 'pointer', border: 'none', background: 'transparent',
                  borderBottom: filter === tab ? '2px solid var(--cyan)' : '2px solid transparent',
                  color: filter === tab ? 'var(--cyan)' : 'var(--muted)',
                }}>{tab}</button>
              ))}
            </div>

            {/* Alarm list */}
            <div ref={parentRef} role="list" aria-live="polite" style={{ flex: 1, overflow: 'auto', padding: isMobile ? 12 : 8 }}>
              {isLoading && <div style={{ padding: 20, color: 'var(--muted)', textAlign: 'center', fontSize: 13 }}>Loading...</div>}
              {!isLoading && alarms.length === 0 && <div style={{ padding: 20, color: 'var(--muted)', textAlign: 'center', fontSize: 13 }}>No alarms</div>}
              {!isLoading && alarms.length > 0 && (
                <div style={{ height: `${virtualizer.getTotalSize()}px`, width: '100%', position: 'relative' }}>
                  {virtualizer.getVirtualItems().map(virtualItem => {
                    const alarm = alarms[virtualItem.index]
                    const isCritical = alarm.severity === 'critical' || alarm.priority === 'critical'
                    const isActive = !alarm.acknowledged_at
                    const isResolved = !!alarm.resolved_at
                    const isShelved = !!alarm.shelved_until && alarm.shelved_until * 1000 > Date.now()
                    const priority = alarm.priority || alarm.severity
                    const pColors = PRIORITY_COLORS[priority] || PRIORITY_COLORS.info
                    const sevColor = isCritical ? 'var(--crit)' : pColors.fg
                    const sevBg = isCritical ? 'rgba(239,68,68,.15)' : pColors.bg

                    // Calculate remaining shelf time
                    const shelfRemaining = isShelved && alarm.shelved_until
                      ? Math.max(0, alarm.shelved_until - Math.floor(Date.now() / 1000))
                      : 0

                    const cardContent = (
                      <div role="listitem" style={{
                        padding: cardPad, borderRadius: 8,
                        border: `1px solid ${isActive && !isShelved ? sevColor : 'var(--border)'}`,
                        background: isShelved ? 'rgba(148,163,184,.08)' : (isActive ? sevBg : 'var(--card)'),
                        opacity: isResolved ? 0.6 : (isShelved ? 0.7 : 1),
                      }}>
                        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 6 }}>
                          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                            {isActive && !isShelved && <div style={{ width: isMobile ? 10 : 8, height: isMobile ? 10 : 8, borderRadius: '50%', background: sevColor, animation: 'pulse 1.5s infinite' }} />}
                            <PriorityBadge priority={priority} isMobile={isMobile} />
                            {alarm.alarm_type && (
                              <span style={{
                                padding: '1px 6px', borderRadius: 3, fontSize: 9, fontWeight: 600,
                                background: alarm.alarm_type === 'low' ? 'rgba(168,139,250,.15)' : 'rgba(34,197,94,.15)',
                                color: alarm.alarm_type === 'low' ? '#a78bfa' : 'var(--ok)',
                              }}>{alarm.alarm_type === 'low' ? '↓ LO' : '↑ HI'}</span>
                            )}
                            {isShelved && (
                              <span style={{
                                padding: '1px 6px', borderRadius: 3, fontSize: 9, fontWeight: 600,
                                background: 'rgba(148,163,184,.15)', color: '#94a3b8',
                              }}>
                                SHELVED {shelfRemaining > 0 ? formatDuration(shelfRemaining) : ''}
                              </span>
                            )}
                          </div>
                          <span style={{ fontSize: timeSize, color: 'var(--muted)' }}>{fmtTs(alarm.triggered_at * 1000)}</span>
                        </div>
                        <div style={{ fontSize: sourceSize, fontWeight: 600, marginBottom: 4 }}>{alarm.source_name}</div>
                        <div style={{ fontSize: msgSize, color: 'var(--muted)', marginBottom: 8 }}>{alarm.message}</div>

                        {/* Action buttons */}
                        <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                          {isActive && !isShelved && (
                            <button onClick={() => ackMut.mutate(alarm.id)} aria-label={`Acknowledge alarm: ${alarm.source_name}`} style={{
                              padding: '4px 8px', borderRadius: 6, fontSize: 11, fontWeight: 600, cursor: 'pointer',
                              border: `1px solid ${sevColor}`, background: 'transparent', color: sevColor,
                            }}>Acknowledge</button>
                          )}

                          {/* Shelf / Unshelf controls */}
                          {isActive && !isShelved && (
                            <button onClick={() => setShelfTarget(shelfTarget === alarm.id ? null : alarm.id)} style={{
                              padding: '4px 8px', borderRadius: 6, fontSize: 11, fontWeight: 600, cursor: 'pointer',
                              border: '1px solid var(--border)', background: 'transparent', color: 'var(--muted)',
                            }}>Shelf</button>
                          )}
                          {isShelved && (
                            <button onClick={() => unshelfMut.mutate(alarm.id)} disabled={unshelfMut.isPending} style={{
                              padding: '4px 8px', borderRadius: 6, fontSize: 11, fontWeight: 600, cursor: 'pointer',
                              border: '1px solid #94a3b8', background: 'transparent', color: '#94a3b8',
                              opacity: unshelfMut.isPending ? 0.6 : 1,
                            }}>Unshelf</button>
                          )}
                          <button onClick={() => setHistoryTarget(historyTarget === alarm.id ? null : alarm.id)} aria-label={`Toggle history: ${alarm.source_name}`} aria-expanded={historyTarget === alarm.id} style={{
                            padding: '4px 8px', borderRadius: 6, fontSize: 11, fontWeight: 600, cursor: 'pointer',
                            border: '1px solid var(--border)', background: 'transparent',
                            color: historyTarget === alarm.id ? 'var(--cyan)' : 'var(--muted)',
                          }}>History</button>
                        </div>

                        {/* Shelf form */}
                        {shelfTarget === alarm.id && (
                          <ShelfForm alarmId={alarm.id} onClose={() => setShelfTarget(null)} />
                        )}

                        {/* ISA-18.2 journal */}
                        {historyTarget === alarm.id && <AlarmHistory alarmId={alarm.id} />}

                        {alarm.acknowledged_at && (
                          <span style={{ fontSize: 11, color: 'var(--ok)' }}>&#10003; Acknowledged{alarm.acknowledged_by ? ` by ${alarm.acknowledged_by}` : ''}</span>
                        )}
                      </div>
                    )

                    return (
                      <div
                        key={alarm.id}
                        ref={virtualizer.measureElement}
                        data-index={virtualItem.index}
                        style={{
                          position: 'absolute', top: 0, left: 0, width: '100%',
                          transform: `translateY(${virtualItem.start}px)`,
                        }}
                      >
                        <SwipeableAlarmRow
                          alarm={alarm}
                          isMobile={isMobile}
                          onAck={() => ackMut.mutate(alarm.id)}
                        >
                          {cardContent}
                        </SwipeableAlarmRow>
                      </div>
                    )
                  })}
                </div>
              )}
            </div>
          </>
        )}

        {/* Analytics tab content */}
        {panelTab === 'analytics' && <AnalyticsTab isMobile={isMobile} />}

        {/* Audit tab content */}
        {panelTab === 'audit' && <AuditTab isMobile={isMobile} />}
      </div>
    </div>
  )
}
