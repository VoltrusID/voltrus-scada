import { useState } from 'react'
import {
  useOeeMetrics,
  useDowntimeEvents,
  useShifts,
  useCreateShift,
  useDeleteShift,
  useCloseDowntimeEvent,
} from './queries'

const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']

function OeeGauge({ value, label, color }: { value: number; label: string; color: string }) {
  const pct = Math.round(value * 100)
  const radius = 40
  const circumference = 2 * Math.PI * radius
  const offset = circumference * (1 - Math.min(value, 1))

  return (
    <div style={{ textAlign: 'center', minWidth: 100 }}>
      <svg width="100" height="100" viewBox="0 0 100 100">
        <circle cx="50" cy="50" r={radius} fill="none" stroke="var(--border)" strokeWidth="8" />
        <circle
          cx="50" cy="50" r={radius} fill="none"
          stroke={color} strokeWidth="8"
          strokeDasharray={circumference}
          strokeDashoffset={offset}
          strokeLinecap="round"
          transform="rotate(-90 50 50)"
          style={{ transition: 'stroke-dashoffset 0.5s ease' }}
        />
        <text x="50" y="46" textAnchor="middle" fill="var(--fg)" fontSize="18" fontWeight="700">
          {pct}%
        </text>
        <text x="50" y="62" textAnchor="middle" fill="var(--muted)" fontSize="9">
          {label}
        </text>
      </svg>
    </div>
  )
}

function BreakdownBar({ label, value, color }: { label: string; value: number; color: string }) {
  const pct = Math.round(value * 100)
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
      <span style={{ width: 90, fontSize: 13, color: 'var(--muted)' }}>{label}</span>
      <div style={{ flex: 1, height: 8, background: 'var(--border)', borderRadius: 4, overflow: 'hidden' }}>
        <div style={{ width: `${pct}%`, height: '100%', background: color, borderRadius: 4, transition: 'width 0.5s ease' }} />
      </div>
      <span style={{ width: 36, fontSize: 13, fontWeight: 600, textAlign: 'right' }}>{pct}%</span>
    </div>
  )
}

function formatDuration(secs: number | null): string {
  if (secs == null) return '--'
  const h = Math.floor(secs / 3600)
  const m = Math.floor((secs % 3600) / 60)
  if (h > 0) return `${h}h ${m}m`
  return `${m}m`
}

function formatTime(ts: number): string {
  return new Date(ts * 1000).toLocaleString()
}

function daysMaskToString(mask: number): string {
  return DAYS.filter((_, i) => mask & (1 << i)).join(', ')
}

export function OeePanel() {
  const [deviceId, setDeviceId] = useState('')
  const [timeRange, setTimeRange] = useState<'8h' | '24h' | '7d'>('8h')

  const now = Math.floor(Date.now() / 1000)
  const rangeMap = { '8h': 8 * 3600, '24h': 24 * 3600, '7d': 7 * 86400 }
  const from = now - rangeMap[timeRange]

  const { data: metrics } = useOeeMetrics(deviceId || undefined, from, now)
  const { data: events } = useDowntimeEvents(deviceId || undefined, from, now)
  const { data: shifts } = useShifts()

  return (
    <div style={{ padding: 24 }}>
      <h2 style={{ marginBottom: 16 }}>OEE / Downtime Tracking</h2>

      {/* Filters */}
      <div style={{ display: 'flex', gap: 12, marginBottom: 20, alignItems: 'center', flexWrap: 'wrap' }}>
        <input
          placeholder="Device ID"
          value={deviceId}
          onChange={e => setDeviceId(e.target.value)}
          style={{
            padding: '6px 12px', background: 'var(--bg2)', border: '1px solid var(--border)',
            borderRadius: 6, color: 'var(--fg)', fontSize: 13, width: 200,
          }}
        />
        <div style={{ display: 'flex', gap: 0, border: '1px solid var(--border)', borderRadius: 6, overflow: 'hidden' }}>
          {(['8h', '24h', '7d'] as const).map(r => (
            <button
              key={r}
              onClick={() => setTimeRange(r)}
              style={{
                padding: '6px 14px', fontSize: 13, cursor: 'pointer', border: 'none',
                background: timeRange === r ? 'var(--cyan)' : 'var(--bg2)',
                color: timeRange === r ? '#000' : 'var(--fg)',
                fontWeight: timeRange === r ? 600 : 400,
              }}
            >
              {r}
            </button>
          ))}
        </div>
      </div>

      {deviceId && metrics ? (
        <>
          {/* OEE Gauges */}
          <div style={{ display: 'flex', gap: 16, justifyContent: 'center', marginBottom: 24, flexWrap: 'wrap' }}>
            <OeeGauge value={metrics.oee} label="OEE" color={metrics.oee >= 0.85 ? '#22c55e' : metrics.oee >= 0.65 ? '#f59e0b' : '#ef4444'} />
            <OeeGauge value={metrics.availability} label="Availability" color="#3b82f6" />
            <OeeGauge value={metrics.performance} label="Performance" color="#8b5cf6" />
            <OeeGauge value={metrics.quality} label="Quality" color="#06b6d4" />
          </div>

          {/* Breakdown bars */}
          <div style={{ maxWidth: 400, margin: '0 auto 24px' }}>
            <BreakdownBar label="Availability" value={metrics.availability} color="#3b82f6" />
            <BreakdownBar label="Performance" value={metrics.performance} color="#8b5cf6" />
            <BreakdownBar label="Quality" value={metrics.quality} color="#06b6d4" />
          </div>

          {/* Summary stats */}
          <div style={{ display: 'flex', gap: 24, justifyContent: 'center', marginBottom: 24, flexWrap: 'wrap' }}>
            <StatBox label="Running Time" value={formatDuration(metrics.total_running_secs)} />
            <StatBox label="Planned Time" value={formatDuration(metrics.total_planned_secs)} />
            <StatBox label="Downtime Events" value={String(metrics.downtime_count)} />
          </div>
        </>
      ) : (
        <div style={{ textAlign: 'center', padding: '40px 0', color: 'var(--muted)' }}>
          {deviceId ? 'Loading...' : 'Enter a Device ID to view OEE metrics'}
        </div>
      )}

      {/* Downtime Events Table */}
      <h3 style={{ marginBottom: 8 }}>Downtime Events</h3>
      {events && events.length > 0 ? (
        <div style={{ overflowX: 'auto', marginBottom: 24 }}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13 }}>
            <thead>
              <tr style={{ borderBottom: '2px solid var(--border)', textAlign: 'left' }}>
                <th style={{ padding: '8px 12px' }}>State</th>
                <th style={{ padding: '8px 12px' }}>Device</th>
                <th style={{ padding: '8px 12px' }}>Started</th>
                <th style={{ padding: '8px 12px' }}>Ended</th>
                <th style={{ padding: '8px 12px' }}>Duration</th>
                <th style={{ padding: '8px 12px' }}>Reason</th>
                <th style={{ padding: '8px 12px' }}></th>
              </tr>
            </thead>
            <tbody>
              {events.map(ev => (
                <tr key={ev.id} style={{ borderBottom: '1px solid var(--border)' }}>
                  <td style={{ padding: '8px 12px' }}>
                    <span style={{
                      padding: '2px 8px', borderRadius: 4, fontSize: 11, fontWeight: 600,
                      background: ev.state === 'faulted' ? 'rgba(239,68,68,.15)' : ev.state === 'idle' ? 'rgba(245,158,11,.15)' : 'rgba(34,197,94,.15)',
                      color: ev.state === 'faulted' ? '#ef4444' : ev.state === 'idle' ? '#f59e0b' : '#22c55e',
                    }}>
                      {ev.state}
                    </span>
                  </td>
                  <td style={{ padding: '8px 12px' }}>{ev.device_id}</td>
                  <td style={{ padding: '8px 12px' }}>{formatTime(ev.started_at)}</td>
                  <td style={{ padding: '8px 12px' }}>{ev.ended_at ? formatTime(ev.ended_at) : 'Open'}</td>
                  <td style={{ padding: '8px 12px' }}>{formatDuration(ev.duration_secs)}</td>
                  <td style={{ padding: '8px 12px', maxWidth: 200, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {ev.reason || '--'}
                  </td>
                  <td style={{ padding: '8px 12px' }}>
                    {!ev.ended_at && <CloseEventButton eventId={ev.id} />}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p style={{ color: 'var(--muted)', marginBottom: 24 }}>
          {deviceId ? 'No downtime events in this period.' : 'Enter a Device ID to view events.'}
        </p>
      )}

      {/* Shift Configuration */}
      <ShiftManager shifts={shifts} />
    </div>
  )
}

function StatBox({ label, value }: { label: string; value: string }) {
  return (
    <div style={{
      padding: '12px 20px', background: 'var(--bg2)', borderRadius: 8,
      border: '1px solid var(--border)', textAlign: 'center', minWidth: 120,
    }}>
      <div style={{ fontSize: 20, fontWeight: 700, color: 'var(--fg)' }}>{value}</div>
      <div style={{ fontSize: 12, color: 'var(--muted)', marginTop: 4 }}>{label}</div>
    </div>
  )
}

function CloseEventButton({ eventId }: { eventId: number }) {
  const closeMutation = useCloseDowntimeEvent()
  const [reason, setReason] = useState('')

  return (
    <div style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
      <input
        placeholder="Reason..."
        value={reason}
        onChange={e => setReason(e.target.value)}
        style={{
          padding: '2px 6px', fontSize: 11, background: 'var(--bg2)',
          border: '1px solid var(--border)', borderRadius: 4, color: 'var(--fg)', width: 100,
        }}
      />
      <button
        onClick={() => closeMutation.mutate({ id: eventId, reason: reason || undefined })}
        disabled={closeMutation.isPending}
        style={{
          padding: '3px 10px', fontSize: 11, cursor: 'pointer',
          background: 'var(--cyan)', color: '#000', border: 'none', borderRadius: 4,
          fontWeight: 600,
        }}
      >
        Close
      </button>
    </div>
  )
}

function ShiftManager({ shifts }: { shifts: { id: number; name: string; start_hour: number; start_minute: number; duration_hours: number; days_mask: number }[] | undefined }) {
  const createMutation = useCreateShift()
  const deleteMutation = useDeleteShift()
  const [showForm, setShowForm] = useState(false)
  const [name, setName] = useState('')
  const [startHour, setStartHour] = useState(6)
  const [startMinute, setStartMinute] = useState(0)
  const [durationHours, setDurationHours] = useState(8)
  const [daysMask, setDaysMask] = useState(31)

  const handleCreate = () => {
    if (!name.trim()) return
    createMutation.mutate({
      name: name.trim(),
      start_hour: startHour,
      start_minute: startMinute,
      duration_hours: durationHours,
      days_mask: daysMask,
    }, {
      onSuccess: () => {
        setName('')
        setShowForm(false)
      },
    })
  }

  const toggleDay = (bit: number) => {
    setDaysMask(prev => prev ^ (1 << bit))
  }

  return (
    <div>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 8 }}>
        <h3 style={{ margin: 0 }}>Shift Configuration</h3>
        <button
          onClick={() => setShowForm(!showForm)}
          style={{
            padding: '4px 12px', fontSize: 12, cursor: 'pointer',
            background: 'var(--cyan)', color: '#000', border: 'none', borderRadius: 4, fontWeight: 600,
          }}
        >
          {showForm ? 'Cancel' : '+ Add Shift'}
        </button>
      </div>

      {showForm && (
        <div style={{
          padding: 16, marginBottom: 12, background: 'var(--bg2)',
          border: '1px solid var(--border)', borderRadius: 8,
        }}>
          <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', marginBottom: 12 }}>
            <input placeholder="Shift name" value={name} onChange={e => setName(e.target.value)}
              style={{ padding: '4px 10px', background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 4, color: 'var(--fg)', fontSize: 13, width: 140 }} />
            <label style={{ display: 'flex', alignItems: 'center', gap: 4, fontSize: 13, color: 'var(--muted)' }}>
              Start:
              <input type="number" min={0} max={23} value={startHour} onChange={e => setStartHour(Number(e.target.value))}
                style={{ width: 50, padding: '4px 6px', background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 4, color: 'var(--fg)', fontSize: 13 }} />
              :
              <input type="number" min={0} max={59} value={startMinute} onChange={e => setStartMinute(Number(e.target.value))}
                style={{ width: 50, padding: '4px 6px', background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 4, color: 'var(--fg)', fontSize: 13 }} />
            </label>
            <label style={{ display: 'flex', alignItems: 'center', gap: 4, fontSize: 13, color: 'var(--muted)' }}>
              Duration:
              <input type="number" min={1} max={24} value={durationHours} onChange={e => setDurationHours(Number(e.target.value))}
                style={{ width: 50, padding: '4px 6px', background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 4, color: 'var(--fg)', fontSize: 13 }} />
              h
            </label>
          </div>
          <div style={{ display: 'flex', gap: 6, marginBottom: 12 }}>
            {DAYS.map((d, i) => (
              <button
                key={d}
                onClick={() => toggleDay(i)}
                style={{
                  padding: '4px 8px', fontSize: 11, cursor: 'pointer', border: '1px solid var(--border)',
                  borderRadius: 4, fontWeight: 600,
                  background: daysMask & (1 << i) ? 'var(--cyan)' : 'var(--bg)',
                  color: daysMask & (1 << i) ? '#000' : 'var(--muted)',
                }}
              >
                {d}
              </button>
            ))}
          </div>
          <button
            onClick={handleCreate}
            disabled={!name.trim() || createMutation.isPending}
            style={{
              padding: '6px 16px', fontSize: 13, cursor: 'pointer',
              background: name.trim() ? 'var(--cyan)' : 'var(--border)',
              color: name.trim() ? '#000' : 'var(--muted)',
              border: 'none', borderRadius: 4, fontWeight: 600,
            }}
          >
            Create Shift
          </button>
        </div>
      )}

      {shifts && shifts.length > 0 ? (
        <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap' }}>
          {shifts.map(shift => (
            <div key={shift.id} style={{
              padding: '10px 16px', background: 'var(--bg2)', borderRadius: 8,
              border: '1px solid var(--border)', minWidth: 180,
            }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <span style={{ fontWeight: 600, fontSize: 14 }}>{shift.name}</span>
                <button
                  onClick={() => deleteMutation.mutate(shift.id)}
                  style={{
                    padding: '2px 8px', fontSize: 11, cursor: 'pointer',
                    background: 'rgba(239,68,68,.1)', color: '#ef4444',
                    border: '1px solid rgba(239,68,68,.3)', borderRadius: 4,
                  }}
                >
                  Delete
                </button>
              </div>
              <div style={{ fontSize: 12, color: 'var(--muted)', marginTop: 4 }}>
                {String(shift.start_hour).padStart(2, '0')}:{String(shift.start_minute).padStart(2, '0')} -- {shift.duration_hours}h
              </div>
              <div style={{ fontSize: 11, color: 'var(--muted)' }}>
                {daysMaskToString(shift.days_mask)}
              </div>
            </div>
          ))}
        </div>
      ) : (
        <p style={{ color: 'var(--muted)', fontSize: 13 }}>No shifts configured.</p>
      )}
    </div>
  )
}
