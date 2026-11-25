import React, { useEffect, useState } from 'react'
import type { SensorConfig, SensorData } from '../../types'
import { useTrackedCurrentValues } from '../../store/dashboardStore'
import { FACEPLATE_WIDTH } from './Faceplate'
import { commandWords, missingRoles, stateWords } from './faceplateMap'
import type { FaceplateDef, FaceplateMap } from './faceplateMap'
import { valueStyle } from '../../valueStyle'

interface Props {
  def: FaceplateDef
  map: FaceplateMap
  title: string
  sensors: SensorConfig[]
  onCommand: (sensorId: number, value: number) => Promise<boolean>
  onOpenDetail: (sensorId: number) => void
  onClose: () => void
  canOperate: boolean
}

const fmt = (v: number | undefined, decimals = 1) => (v == null || Number.isNaN(v) ? '----' : v.toFixed(decimals))

/** A faceplate definition opened for one symbol: every role shows or commands the tag the symbol maps to it. */
export const MappedFaceplate: React.FC<Props> = ({ def, map, title, sensors, onCommand, onOpenDetail, onClose, canOperate }) => {
  const values = useTrackedCurrentValues()
  const [armed, setArmed] = useState<{ tag: number; value: number } | null>(null)
  const [busy, setBusy] = useState(false)
  const [setpoints, setSetpoints] = useState<Record<string, string>>({})
  useEffect(() => {
    if (!armed) return
    const t = setTimeout(() => setArmed(null), 6000)
    return () => clearTimeout(t)
  }, [armed])
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  const send = async (tag: number, value: number) => {
    setBusy(true)
    try { await onCommand(tag, value) } finally { setBusy(false); setArmed(null) }
  }
  const missing = missingRoles(def, map)
  const [onWord, offWord] = commandWords(def.kind)
  const tagOf = (id: number) => sensors.find(s => s.id === id)
  const runRole = def.roles.find(r => r.kind === 'state' && map[r.role] != null)
  const running = runRole ? (values[String(map[runRole.role])]?.value ?? 0) > 0.5 : undefined

  return (
    <div role="dialog" aria-label={`${title} faceplate`} onMouseDown={e => e.stopPropagation()} style={{
      position: 'absolute', top: 12, right: 12, width: FACEPLATE_WIDTH, zIndex: 30, maxHeight: 'calc(100% - 24px)', overflowY: 'auto',
      background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 8, boxShadow: '0 16px 48px rgba(0,0,0,.45)',
      fontSize: 12, color: 'var(--fg)',
    }}>
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 8, padding: '10px 12px', borderBottom: '1px solid var(--border)' }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 14, fontWeight: 700, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{title}</div>
          <div style={{ marginTop: 2, color: 'var(--text-muted)', fontSize: 11 }}>{def.name} faceplate</div>
        </div>
        <button onClick={onClose} aria-label="Close faceplate" style={{ width: 24, height: 24, border: 'none', background: 'none', color: 'var(--text-muted)', cursor: 'pointer', fontSize: 18, lineHeight: 1, padding: 0 }}>×</button>
      </div>
      {missing.length > 0 && (
        <div style={{ padding: '8px 12px', color: 'var(--warn)', fontSize: 11 }}>Not mapped on this symbol: {missing.join(', ')}</div>
      )}
      {def.roles.map(r => {
        const tag = map[r.role]
        const s = tag != null ? tagOf(tag) : undefined
        const d: SensorData | undefined = tag != null ? values[String(tag)] : undefined
        const label = r.label || r.role
        const row = (content: React.ReactNode) => (
          <div key={r.role} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, padding: '7px 12px', borderTop: '1px solid var(--border)' }}>
            <span style={{ color: 'var(--text-dim)' }}>{label} <span style={{ color: 'var(--text-muted)', fontFamily: 'var(--font-mono)', fontSize: 11 }}>{s?.key ?? (tag != null ? `#${tag}` : 'not mapped')}</span></span>
            {content}
          </div>
        )
        if (tag == null) return row(<span style={{ color: 'var(--text-muted)' }}>—</span>)
        if (r.kind === 'state' || r.kind === 'fault') {
          const on = (d?.value ?? 0) > 0.5
          const [onT, offT] = stateWords(def.kind, r)
          const color = d == null ? 'var(--hmi-stale)' : on ? (r.kind === 'fault' ? 'var(--hmi-fault)' : 'var(--hmi-run)') : 'var(--text-muted)'
          return row(<span className={r.kind === 'fault' && on ? 'hmi-blink' : undefined} style={{ fontWeight: 700, fontSize: 11, letterSpacing: '.06em', color }}>{d == null ? 'NO DATA' : on ? onT : offT}</span>)
        }
        if (r.kind === 'analog') {
          const st = valueStyle(d)
          return row(
            <button onClick={() => onOpenDetail(tag)} title="Open the trend" style={{ border: 'none', background: 'none', cursor: 'pointer', padding: 0, fontFamily: 'var(--font-mono)', fontWeight: 700, fontSize: 13, color: st.color ?? 'var(--fg)' }}>
              {st.marker && <span aria-hidden style={{ marginRight: 4 }}>{st.marker}</span>}{fmt(d?.value, s?.decimals ?? 1)} <span style={{ fontWeight: 400, color: 'var(--text-muted)', fontSize: 11 }}>{s?.unit}</span>
            </button>,
          )
        }
        if (r.kind === 'setpoint') {
          return row(
            <span style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
              <span style={{ fontFamily: 'var(--font-mono)' }}>{fmt(d?.value, s?.decimals ?? 1)}</span>
              {canOperate && <>
                <input aria-label={`New ${label}`} value={setpoints[r.role] ?? ''} onChange={e => setSetpoints({ ...setpoints, [r.role]: e.target.value })} inputMode="decimal" style={{ width: 60, padding: '2px 4px', fontSize: 11, background: 'var(--input-bg)', color: 'var(--fg)', border: '1px solid var(--border)', borderRadius: 4 }} />
                <button disabled={busy || !Number.isFinite(parseFloat(setpoints[r.role] ?? ''))} onClick={() => send(tag, parseFloat(setpoints[r.role]))} style={{ fontSize: 11, padding: '2px 6px', borderRadius: 4, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: 'pointer' }}>Set</button>
              </>}
            </span>,
          )
        }
        // command
        if (!canOperate) return row(<span style={{ color: 'var(--text-muted)' }}>no permission</span>)
        return row(
          armed?.tag === tag ? (
            <span style={{ display: 'flex', gap: 4 }}>
              <button disabled={busy} onClick={() => send(tag, armed.value)} style={{ fontSize: 11, fontWeight: 700, padding: '3px 8px', borderRadius: 4, border: '1px solid var(--hmi-warn)', background: 'color-mix(in srgb, var(--hmi-warn) 18%, transparent)', color: 'var(--hmi-warn)', cursor: 'pointer' }}>{busy ? 'Sending…' : `Confirm ${armed.value ? onWord : offWord}`}</button>
              <button onClick={() => setArmed(null)} style={{ fontSize: 11, padding: '3px 6px', borderRadius: 4, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: 'pointer' }}>Cancel</button>
            </span>
          ) : (
            <span style={{ display: 'flex', gap: 4 }}>
              <button disabled={busy || running === true} onClick={() => setArmed({ tag, value: 1 })} style={{ fontSize: 11, fontWeight: 700, padding: '3px 8px', borderRadius: 4, border: '1px solid var(--border)', background: 'var(--input-bg)', color: 'var(--fg)', cursor: 'pointer', opacity: running === true ? 0.4 : 1 }}>{onWord}</button>
              <button disabled={busy || running === false} onClick={() => setArmed({ tag, value: 0 })} style={{ fontSize: 11, fontWeight: 700, padding: '3px 8px', borderRadius: 4, border: '1px solid var(--border)', background: 'var(--input-bg)', color: 'var(--fg)', cursor: 'pointer', opacity: running === false ? 0.4 : 1 }}>{offWord}</button>
            </span>
          ),
        )
      })}
    </div>
  )
}
