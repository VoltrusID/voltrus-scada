import React from 'react'
import { useQuery } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import type { Alarm } from '../types'
import { useAcknowledgeAlarm, fetchJson } from '../queries'

export const ALARM_BANNER_ROWS = 3
const ROW_H = 26
export const ALARM_BANNER_HEIGHT = 30 + ROW_H * ALARM_BANNER_ROWS

const rank = (a: Alarm) => (a.acknowledged_at ? 0 : 2) + (a.severity === 'critical' ? 1 : 0)

const clock = (secs: number) => new Date(secs * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })

/** ISA-101 alarm banner under the runtime screen: newest unacknowledged alarms first, acknowledge in place. */
export const AlarmBanner: React.FC = () => {
  // 'all' rather than 'active': acknowledged alarms stay in the banner until they clear. Under ['alarms'] so acks refresh it.
  const { data: alarms = [] } = useQuery({
    queryKey: ['alarms', 'banner'],
    queryFn: () => fetchJson<Alarm[]>('/api/v1/alarms?status=all&limit=50'),
    refetchInterval: 3_000,
  })
  const ack = useAcknowledgeAlarm()
  const navigate = useNavigate()

  const active = alarms.filter(a => a.resolved_at == null && a.shelved_until == null)
  const sorted = [...active].sort((a, b) => rank(b) - rank(a) || b.triggered_at - a.triggered_at)
  const crit = active.filter(a => a.severity === 'critical').length
  const warn = active.length - crit
  const unack = active.filter(a => !a.acknowledged_at).length

  return (
    <div style={{
      height: ALARM_BANNER_HEIGHT, flexShrink: 0, display: 'flex', flexDirection: 'column',
      background: 'var(--card)', borderTop: '1px solid var(--border)', fontSize: 12,
    }}>
      <div style={{ height: 30, display: 'flex', alignItems: 'center', gap: 14, padding: '0 12px', borderBottom: '1px solid var(--border)' }}>
        <span style={{ fontSize: 10, fontWeight: 700, letterSpacing: '.1em', color: 'var(--text-muted)' }}>ALARMS</span>
        <Count color="var(--hmi-fault)" n={crit} label="critical" />
        <Count color="var(--hmi-warn)" n={warn} label="warning" />
        <span style={{ color: unack ? 'var(--fg)' : 'var(--text-muted)' }}><b style={{ fontFamily: 'var(--font-mono)' }}>{unack}</b> unacknowledged</span>
        <span style={{ flex: 1 }} />
        {active.length > ALARM_BANNER_ROWS && <span style={{ color: 'var(--text-muted)' }}>+{active.length - ALARM_BANNER_ROWS} more</span>}
        <button onClick={() => navigate({ to: '/alarms' })} style={{ border: 'none', background: 'none', color: 'var(--cyan)', cursor: 'pointer', fontSize: 12, fontWeight: 600, padding: 0 }}>
          Alarm summary →
        </button>
      </div>
      {sorted.length === 0 ? (
        <div style={{ flex: 1, display: 'grid', placeItems: 'center', color: 'var(--text-muted)' }}>No active alarms</div>
      ) : sorted.slice(0, ALARM_BANNER_ROWS).map(a => {
        const color = a.severity === 'critical' ? 'var(--hmi-fault)' : 'var(--hmi-warn)'
        const unacked = !a.acknowledged_at
        return (
          <div key={a.id} style={{
            height: ROW_H, display: 'grid', gridTemplateColumns: '18px 76px minmax(160px, 1.1fr) 2fr 110px 92px', alignItems: 'center', gap: 10,
            padding: '0 12px', borderBottom: '1px solid color-mix(in srgb, var(--border) 60%, transparent)',
            background: unacked ? `color-mix(in srgb, ${color} 7%, transparent)` : undefined,
          }}>
            <span className={unacked && a.severity === 'critical' ? 'hmi-blink' : undefined}
              style={{ width: 9, height: 9, background: unacked ? color : 'transparent', border: `1.5px solid ${color}`, transform: 'rotate(45deg)', justifySelf: 'center' }} />
            <span style={{ fontFamily: 'var(--font-mono)', color: 'var(--text-muted)' }}>{clock(a.triggered_at)}</span>
            <span style={{ fontWeight: 600, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{a.source_name || a.source_key}</span>
            <span style={{ color: 'var(--text-dim)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
              {a.alarm_type === 'low' ? 'Low' : 'High'} {a.severity === 'critical' ? 'critical' : 'warning'}: {fmt(a.value)} (limit {fmt(a.threshold)})
            </span>
            <span style={{ fontSize: 10, fontWeight: 700, letterSpacing: '.08em', color: unacked ? color : 'var(--text-muted)' }}>
              {unacked ? 'UNACKED' : `ACK · ${a.acknowledged_by ?? ''}`}
            </span>
            {unacked ? (
              <button onClick={() => ack.mutate(a.id)} disabled={ack.isPending} style={{
                height: 20, padding: '0 10px', borderRadius: 4, border: '1px solid var(--border)', background: 'var(--input-bg)',
                color: 'var(--fg)', cursor: 'pointer', fontSize: 11, fontWeight: 600,
              }}>Acknowledge</button>
            ) : <span />}
          </div>
        )
      })}
    </div>
  )
}

function Count({ color, n, label }: { color: string; n: number; label: string }) {
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6, color: n ? 'var(--fg)' : 'var(--text-muted)' }}>
      <span style={{ width: 8, height: 8, background: n ? color : 'transparent', border: `1.5px solid ${color}`, transform: 'rotate(45deg)' }} />
      <b style={{ fontFamily: 'var(--font-mono)' }}>{n}</b> {label}
    </span>
  )
}

const fmt = (v: number) => (Math.abs(v) >= 100 ? v.toFixed(0) : Math.abs(v) >= 10 ? v.toFixed(1) : v.toFixed(2))
