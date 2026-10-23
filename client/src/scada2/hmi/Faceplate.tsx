import React, { useEffect, useMemo, useState } from 'react'
import type { DataPoint, SensorConfig, SensorData } from '../../types'
import type { WidgetInstance } from '../widgets/types'
import { useChartData } from '../../store/dashboardStore'
import { resolveSymbol } from './resolve'
import { symbolLiveState, symbolWritable, stateText, stateColor } from './SymbolView'

export const FACEPLATE_WIDTH = 320

interface Props {
  widget: WidgetInstance
  sensors: SensorConfig[]
  currentValues: Record<string, SensorData>
  /** Resolves true once the write was accepted. */
  onCommand: (sensorId: number, value: number) => Promise<boolean>
  onOpenDetail: (sensorId: number) => void
  onClose: () => void
}

/** History backfill stamps points in seconds, live samples in ms; compare them in seconds. */
const secs = (ts: number) => (ts > 1e11 ? ts / 1000 : ts)

const RELATED_TEXT: Record<string, [string, string]> = {
  'Fault': ['TRIPPED', 'HEALTHY'], 'Run feedback': ['RUNNING', 'STOPPED'], 'Open feedback': ['OPEN', 'CLOSED'],
  'Run': ['RUNNING', 'STOPPED'], 'Backwash': ['IN PROGRESS', 'IDLE'],
}

const fmt = (v: number | undefined | null, decimals = 1) => (v == null || Number.isNaN(v) ? '----' : v.toFixed(decimals))

function lowLimits(sensor: SensorConfig): { lw: number | null; lc: number | null } {
  try {
    const cfg = JSON.parse(sensor.source_config || '{}')
    return { lw: typeof cfg.low_warn === 'number' ? cfg.low_warn : null, lc: typeof cfg.low_crit === 'number' ? cfg.low_crit : null }
  } catch {
    return { lw: null, lc: null }
  }
}

/** A limit counts only when it sits inside the tag's range; 0 means "not configured". */
const limit = (v: number | undefined, s: SensorConfig) => (v != null && v !== 0 && v > s.min_val && v <= s.max_val ? v : null)

/** ISA-101 faceplate: identity, state, PV against its alarm limits, 30-min trend, related tags, and armed operator commands. */
export const Faceplate: React.FC<Props> = ({ widget, sensors, currentValues, onCommand, onOpenDetail, onClose }) => {
  const r = resolveSymbol(widget)
  const config = r?.config ?? widget.config ?? {}
  const sensor = widget.sensorId != null ? sensors.find(s => s.id === widget.sensorId) : undefined
  const data = widget.sensorId != null ? currentValues[String(widget.sensorId)] : undefined
  const live = r ? symbolLiveState(r.def, config, widget.sensorId, sensor, data, currentValues) : null
  const writable = !!r && widget.sensorId != null && !!data && symbolWritable(r.def, config, r.defaultWritable)
  const isValve = r?.def.category === 'Valves'

  const [armed, setArmed] = useState<number | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    if (armed == null) return
    const t = setTimeout(() => setArmed(null), 6000)
    return () => clearTimeout(t)
  }, [armed])
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  const title = config.label || sensor?.name || r?.def.name || 'Tag'
  const alarm = live?.alarm ?? (data?.status === 'crit' ? 'crit' : data?.status === 'warn' ? 'warn' : null)
  const analog = !!sensor && sensor.max_val !== sensor.min_val && r?.def.binding !== 'state'

  const related: { label: string; id: number }[] = []
  if (r?.def.binding === 'state' && widget.sensorId != null) related.push({ label: isValve ? 'Open feedback' : 'Run feedback', id: widget.sensorId })
  if (config.runSensorId != null) related.push({ label: r?.def.key === 'gravity_filter' ? 'Backwash' : 'Run', id: Number(config.runSensorId) })
  if (config.faultSensorId != null) related.push({ label: 'Fault', id: Number(config.faultSensorId) })

  const send = async (value: number) => {
    if (widget.sensorId == null) return
    setBusy(true)
    try { await onCommand(widget.sensorId, value) } finally { setBusy(false); setArmed(null) }
  }

  const onWord = isValve ? 'OPEN' : 'START'
  const offWord = isValve ? 'CLOSE' : 'STOP'

  return (
    <div role="dialog" aria-label={`${title} faceplate`} onMouseDown={e => e.stopPropagation()} style={{
      position: 'absolute', top: 12, right: 12, width: FACEPLATE_WIDTH, zIndex: 30, maxHeight: 'calc(100% - 24px)', overflowY: 'auto',
      background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 8, boxShadow: '0 16px 48px rgba(0,0,0,.45)',
      fontSize: 12, color: 'var(--fg)',
    }}>
      {/* Identity */}
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 8, padding: '10px 12px', borderBottom: '1px solid var(--border)' }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 14, fontWeight: 700, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{title}</div>
          <div style={{ marginTop: 2, color: 'var(--text-muted)', fontFamily: 'var(--font-mono)', fontSize: 11, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
            {[r?.def.name, sensor?.key].filter(Boolean).join(' · ')}
          </div>
        </div>
        <button onClick={onClose} aria-label="Close faceplate" style={{
          width: 24, height: 24, border: 'none', background: 'none', color: 'var(--text-muted)', cursor: 'pointer', fontSize: 18, lineHeight: 1, padding: 0,
        }}>×</button>
      </div>

      {/* State + alarm */}
      <div style={{ display: 'flex', gap: 6, padding: '10px 12px 0', flexWrap: 'wrap' }}>
        {live && r && r.def.binding === 'state' && <Pill color={stateColor(live.state)} blink={live.state === 'fault'}>{stateText(r.def, live.state)}</Pill>}
        {live?.state === 'fault' && r?.def.binding !== 'state' && <Pill color="var(--hmi-fault)" blink>FAULT</Pill>}
        {alarm && <Pill color={alarm === 'crit' ? 'var(--hmi-fault)' : 'var(--hmi-warn)'} blink={alarm === 'crit'}>{alarm === 'crit' ? 'CRITICAL ALARM' : 'WARNING'}</Pill>}
        {!data && widget.sensorId != null && <Pill color="var(--hmi-stale)">NO DATA</Pill>}
        {data && !alarm && (!live || live.state !== 'fault') && <Pill color="var(--text-muted)">QUALITY GOOD</Pill>}
      </div>

      {/* Process value (analog); run feedback for on/off equipment is the state pill above */}
      {sensor && r?.def.binding !== 'state' && (
        <div style={{ padding: '10px 12px 4px' }}>
          <div style={{ display: 'flex', alignItems: 'baseline', gap: 6 }}>
            <span style={{ fontFamily: 'var(--font-mono)', fontSize: 28, fontWeight: 700, color: data ? 'var(--fg)' : 'var(--hmi-stale)' }}>
              {fmt(data?.value, sensor.decimals ?? 1)}
            </span>
            <span style={{ color: 'var(--text-muted)' }}>{sensor.unit}</span>
          </div>
          {analog && <AnalogBar sensor={sensor} value={data?.value} />}
        </div>
      )}

      {/* Trend */}
      {sensor && <Sparkline sensor={sensor} digital={r?.def.binding === 'state'} />}

      {/* Related tags */}
      {related.length > 0 && (
        <div style={{ padding: '4px 12px 8px' }}>
          {related.map(t => {
            const s = sensors.find(x => x.id === t.id)
            const v = currentValues[String(t.id)]?.value
            const on = v != null && v > 0.5
            return (
              <div key={t.label} style={{ display: 'flex', justifyContent: 'space-between', padding: '5px 0', borderTop: '1px solid var(--border)' }}>
                <span style={{ color: 'var(--text-dim)' }}>{t.label} <span style={{ color: 'var(--text-muted)', fontFamily: 'var(--font-mono)', fontSize: 11 }}>{s?.key ?? `#${t.id}`}</span></span>
                <span style={{ fontWeight: 700, fontSize: 11, letterSpacing: '.06em', color: v == null ? 'var(--hmi-stale)' : on ? (t.label === 'Fault' ? 'var(--hmi-fault)' : 'var(--hmi-run)') : 'var(--text-muted)' }}>
                  {v == null ? 'NO DATA' : (RELATED_TEXT[t.label] ?? ['ON', 'OFF'])[on ? 0 : 1]}
                </span>
              </div>
            )
          })}
        </div>
      )}

      {/* Commands */}
      {writable && (
        <div style={{ padding: '10px 12px', borderTop: '1px solid var(--border)' }}>
          <div style={{ fontSize: 10, fontWeight: 700, letterSpacing: '.08em', color: 'var(--text-muted)', marginBottom: 6 }}>OPERATOR COMMAND</div>
          {armed == null ? (
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 6 }}>
              <CmdButton disabled={busy || live?.running} onClick={() => setArmed(1)}>{onWord}</CmdButton>
              <CmdButton disabled={busy || !live?.running} onClick={() => setArmed(0)}>{offWord}</CmdButton>
            </div>
          ) : (
            <div style={{ display: 'grid', gridTemplateColumns: '1fr auto', gap: 6, alignItems: 'center' }}>
              <CmdButton armed disabled={busy} onClick={() => send(armed)}>{busy ? 'Sending…' : `Confirm ${armed ? onWord : offWord}`}</CmdButton>
              <button onClick={() => setArmed(null)} style={{
                padding: '7px 10px', borderRadius: 5, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: 'pointer', fontSize: 12,
              }}>Cancel</button>
            </div>
          )}
          {live?.state === 'fault' && <div style={{ marginTop: 6, color: 'var(--hmi-fault)', fontSize: 11 }}>Equipment is tripped. Reset the fault in the field before restarting.</div>}
        </div>
      )}

      {widget.sensorId != null && (
        <button onClick={() => onOpenDetail(widget.sensorId!)} style={{
          display: 'block', width: '100%', padding: '9px 12px', border: 'none', borderTop: '1px solid var(--border)', background: 'transparent',
          color: 'var(--cyan)', textAlign: 'left', cursor: 'pointer', fontSize: 12, fontWeight: 600, borderRadius: '0 0 8px 8px',
        }}>Open tag history and details →</button>
      )}
    </div>
  )
}

function Pill({ color, blink, children }: { color: string; blink?: boolean; children: React.ReactNode }) {
  return (
    <span className={blink ? 'hmi-blink' : undefined} style={{
      display: 'inline-flex', alignItems: 'center', gap: 5, padding: '2px 7px', borderRadius: 3, fontSize: 10, fontWeight: 700, letterSpacing: '.07em',
      color, border: `1px solid color-mix(in srgb, ${color} 55%, transparent)`, background: `color-mix(in srgb, ${color} 12%, transparent)`,
    }}>
      <span style={{ width: 6, height: 6, borderRadius: 1, background: color, transform: 'rotate(45deg)' }} />
      {children}
    </span>
  )
}

function CmdButton({ children, onClick, disabled, armed }: { children: React.ReactNode; onClick: () => void; disabled?: boolean; armed?: boolean }) {
  return (
    <button onClick={onClick} disabled={disabled} style={{
      padding: '7px 10px', borderRadius: 5, fontSize: 12, fontWeight: 700, letterSpacing: '.06em', cursor: disabled ? 'default' : 'pointer',
      border: `1px solid ${armed ? 'var(--hmi-warn)' : 'var(--border)'}`,
      background: armed ? 'color-mix(in srgb, var(--hmi-warn) 18%, transparent)' : 'var(--input-bg)',
      color: armed ? 'var(--hmi-warn)' : 'var(--fg)', opacity: disabled ? 0.4 : 1,
    }}>{children}</button>
  )
}

/** Horizontal analog indicator: range, alarm zones, and a pointer at the PV. */
function AnalogBar({ sensor, value }: { sensor: SensorConfig; value: number | undefined }) {
  const { min_val: min, max_val: max } = sensor
  const span = max - min || 1
  const pct = (v: number) => Math.max(0, Math.min(100, ((v - min) / span) * 100))
  const warn = limit(sensor.warn_val, sensor)
  const crit = limit(sensor.crit_val, sensor)
  const { lw, lc } = lowLimits(sensor)
  const marks = [
    lc != null && { v: lc, t: 'LL', c: 'var(--hmi-fault)' }, lw != null && { v: lw, t: 'L', c: 'var(--hmi-warn)' },
    warn != null && { v: warn, t: 'H', c: 'var(--hmi-warn)' }, crit != null && { v: crit, t: 'HH', c: 'var(--hmi-fault)' },
  ].filter(Boolean) as { v: number; t: string; c: string }[]
  const zone = (from: number, to: number, c: string, a: number) => (
    <div style={{ position: 'absolute', top: 0, bottom: 0, left: `${pct(from)}%`, width: `${pct(to) - pct(from)}%`, background: `color-mix(in srgb, ${c} ${a}%, transparent)` }} />
  )
  return (
    <div style={{ marginTop: 8 }}>
      <div style={{ position: 'relative', height: 12, borderRadius: 2, background: 'var(--hmi-badge)', border: '1px solid var(--border)', overflow: 'hidden' }}>
        {value != null && <div style={{ position: 'absolute', top: 0, bottom: 0, left: 0, width: `${pct(value)}%`, background: 'color-mix(in srgb, var(--hmi-stroke) 28%, transparent)', transition: 'width .5s ease' }} />}
        {lc != null && zone(min, lc, 'var(--hmi-fault)', 40)}
        {lw != null && zone(lc ?? min, lw, 'var(--hmi-warn)', 32)}
        {warn != null && zone(warn, crit ?? max, 'var(--hmi-warn)', 32)}
        {crit != null && zone(crit, max, 'var(--hmi-fault)', 40)}
        {value != null && <div style={{ position: 'absolute', top: -1, bottom: -1, width: 3, marginLeft: -1.5, left: `${pct(value)}%`, background: 'var(--fg)', transition: 'left .5s ease' }} />}
      </div>
      <div style={{ display: 'flex', justifyContent: 'space-between', gap: 8, marginTop: 4, fontFamily: 'var(--font-mono)', fontSize: 10, color: 'var(--text-muted)' }}>
        <span>{min}–{max} {sensor.unit}</span>
        <span style={{ display: 'flex', gap: 8 }}>
          {marks.map(m => <span key={m.t} style={{ color: m.c }}>{m.t} {m.v}</span>)}
        </span>
      </div>
    </div>
  )
}

function Sparkline({ sensor, digital }: { sensor: SensorConfig; digital?: boolean }) {
  const points = useChartData(sensor.id, 'live') as DataPoint[]
  const W = FACEPLATE_WIDTH - 24
  const H = 70
  const geo = useMemo(() => {
    if (points.length < 2) return null
    const vals = points.map(p => p.value)
    let lo = digital ? 0 : Math.min(...vals)
    let hi = digital ? 1 : Math.max(...vals)
    if (hi - lo < 1e-9) { lo -= 1; hi += 1 }
    const pad = (hi - lo) * 0.1
    lo -= pad; hi += pad
    const t0 = secs(points[0].ts)
    const t1 = secs(points[points.length - 1].ts)
    const x = (t: number) => ((secs(t) - t0) / ((t1 - t0) || 1)) * W
    const y = (v: number) => H - ((v - lo) / (hi - lo)) * H
    const d = digital
      ? points.map((p, i) => (i === 0 ? `M${x(p.ts)} ${y(p.value)}` : `H${x(p.ts)} V${y(p.value)}`)).join(' ')
      : points.map((p, i) => `${i === 0 ? 'M' : 'L'}${x(p.ts).toFixed(1)} ${y(p.value).toFixed(1)}`).join(' ')
    const lines = [limit(sensor.warn_val, sensor), limit(sensor.crit_val, sensor)]
      .map((v, i) => (v != null && v > lo && v < hi ? { y: y(v), color: i ? 'var(--hmi-fault)' : 'var(--hmi-warn)' } : null))
      .filter(Boolean) as { y: number; color: string }[]
    const mins = Math.max(1, Math.round((t1 - t0) / 60))
    return { d, lines, mins, min: Math.min(...vals), max: Math.max(...vals) }
  }, [points, digital, sensor, W])

  return (
    <div style={{ padding: '6px 12px 10px' }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 10, color: 'var(--text-muted)', marginBottom: 4 }}>
        <span style={{ fontWeight: 700, letterSpacing: '.08em' }}>TREND{geo ? ` · LAST ${geo.mins} MIN` : ''}</span>
        {geo && !digital && <span style={{ fontFamily: 'var(--font-mono)' }}>min {geo.min.toFixed(sensor.decimals ?? 1)} · max {geo.max.toFixed(sensor.decimals ?? 1)}</span>}
      </div>
      <svg width={W} height={H} style={{ display: 'block', background: 'var(--hmi-badge)', borderRadius: 3, border: '1px solid var(--border)' }}>
        {[0.25, 0.5, 0.75].map(f => <line key={f} x1={0} x2={W} y1={H * f} y2={H * f} stroke="var(--border)" strokeDasharray="2 4" />)}
        {geo?.lines.map((l, i) => <line key={i} x1={0} x2={W} y1={l.y} y2={l.y} stroke={l.color} strokeDasharray="4 3" opacity={0.8} />)}
        {geo ? <path d={geo.d} fill="none" stroke="var(--cyan)" strokeWidth={1.5} strokeLinejoin="round" />
          : <text x={W / 2} y={H / 2} textAnchor="middle" dominantBaseline="central" fontSize={11} fill="var(--text-muted)">Collecting samples…</text>}
      </svg>
    </div>
  )
}
