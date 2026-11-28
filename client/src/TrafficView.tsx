import { useEffect, useMemo, useRef, useState } from 'react'
import { fetchJson } from './queries'
import { inputStyle, btnStyle } from './styles'
import { useModalFocus } from './components/Dialog'
import {
  EMPTY_FILTERS, MODBUS_FUNCTIONS, fieldAt, formatLatency, formatTime, hexBytes, mergeFrames, nextAfter, pathTo, trafficQuery,
  type TrafficField, type TrafficFilters, type TrafficFrame, type TrafficResponse,
} from './traffic'

const POLL_MS = 1000

/**
 * The Modbus requests and responses between Voltrus and one device, decoded
 * like Wireshark: frame list, field tree and hex pane with the selected
 * field's bytes highlighted (docs/traffic.md). Opening it starts the
 * device's capture on the server.
 */
export function TrafficView({ deviceId, deviceName, onClose }: { deviceId: string; deviceName: string; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null)
  useModalFocus(ref, onClose)
  const [filters, setFilters] = useState<TrafficFilters>(EMPTY_FILTERS)
  const [paused, setPaused] = useState(false)
  const [frames, setFrames] = useState<TrafficFrame[]>([])
  const [meta, setMeta] = useState<Omit<TrafficResponse, 'frames'> | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [selectedSeq, setSelectedSeq] = useState<number | null>(null)
  const [selectedField, setSelectedField] = useState<TrafficField | null>(null)
  const framesRef = useRef(frames)
  framesRef.current = frames

  // A filter change starts the list again.
  useEffect(() => { setFrames([]); setSelectedSeq(null) }, [filters])

  useEffect(() => {
    if (paused) return
    let stopped = false
    let timer: ReturnType<typeof setTimeout> | undefined
    const poll = async () => {
      try {
        const url = `/api/v1/devices/${encodeURIComponent(deviceId)}/traffic?${trafficQuery(filters, nextAfter(framesRef.current))}`
        const resp = await fetchJson<TrafficResponse>(url)
        if (stopped) return
        const { frames: incoming, ...rest } = resp
        setMeta(rest)
        setFrames(old => mergeFrames(old, incoming))
        setError(null)
      } catch (e) {
        if (!stopped) setError(e instanceof Error ? e.message : String(e))
      }
      if (!stopped) timer = setTimeout(poll, POLL_MS)
    }
    poll()
    return () => { stopped = true; clearTimeout(timer) }
  }, [deviceId, filters, paused])

  const selected = useMemo(() => frames.find(f => f.seq === selectedSeq) ?? null, [frames, selectedSeq])
  useEffect(() => { setSelectedField(null) }, [selectedSeq])

  const setFilter = <K extends keyof TrafficFilters>(k: K, v: TrafficFilters[K]) => setFilters(f => ({ ...f, [k]: v }))
  const mono: React.CSSProperties = { fontFamily: 'var(--font-mono)', fontSize: 12 }
  const th: React.CSSProperties = { textAlign: 'left', padding: '6px 8px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600, borderBottom: '1px solid var(--border)', position: 'sticky', top: 0, background: 'var(--card)' }
  const td: React.CSSProperties = { padding: '4px 8px', fontSize: 12, borderBottom: '1px solid var(--border)', whiteSpace: 'nowrap' }

  return (
    <div ref={ref} role="dialog" aria-modal="true" aria-label={`Traffic of ${deviceName}`} tabIndex={-1}
      style={{ position: 'fixed', inset: 0, background: 'var(--bg)', zIndex: 60, display: 'flex', flexDirection: 'column' }}>
      <div style={{ padding: '12px 24px', display: 'flex', alignItems: 'center', gap: 12, borderBottom: '1px solid var(--border)', flexWrap: 'wrap' }}>
        <h2 style={{ fontSize: 18, fontWeight: 700, margin: 0 }}>Traffic: {deviceName}</h2>
        <span style={{ fontSize: 12, color: 'var(--muted)' }}>
          {meta ? `${frames.length} shown, ${meta.kept} kept of ${meta.capacity}${meta.capturing ? '' : ' (capture stopped)'}` : 'Starting capture...'}
        </span>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
          <select aria-label="Function" value={filters.function} onChange={e => setFilter('function', e.target.value)} style={{ ...inputStyle, width: 220 }}>
            <option value="">All functions</option>
            {MODBUS_FUNCTIONS.map(([code, name]) => <option key={code} value={code}>{code}: {name}</option>)}
          </select>
          <select aria-label="Data table" value={filters.space} onChange={e => setFilter('space', e.target.value)} style={{ ...inputStyle, width: 130 }}>
            <option value="">All tables</option>
            <option value="coil">Coils</option>
            <option value="discrete">Discrete inputs</option>
            <option value="input">Input registers</option>
            <option value="holding">Holding registers</option>
          </select>
          <input aria-label="Address from" placeholder="Addr from" inputMode="numeric" value={filters.addrFrom} onChange={e => setFilter('addrFrom', e.target.value)} style={{ ...inputStyle, width: 90 }} />
          <input aria-label="Address to" placeholder="to" inputMode="numeric" value={filters.addrTo} onChange={e => setFilter('addrTo', e.target.value)} style={{ ...inputStyle, width: 70 }} />
          <label style={{ display: 'flex', alignItems: 'center', gap: 4, fontSize: 13 }}>
            <input type="checkbox" checked={filters.errors} onChange={e => setFilter('errors', e.target.checked)} /> Errors only
          </label>
          <button style={{ ...btnStyle, borderColor: paused ? 'var(--green)' : 'var(--warn)', color: paused ? 'var(--green)' : 'var(--warn)' }} onClick={() => setPaused(p => !p)} aria-pressed={paused}>
            {paused ? 'Resume' : 'Pause'}
          </button>
          <button onClick={onClose} aria-label="Close traffic" style={btnStyle}>Close</button>
        </div>
      </div>
      {(error || meta?.note) && (
        <div role={error ? 'alert' : 'status'} style={{ padding: '8px 24px', fontSize: 13, color: error ? 'var(--crit)' : 'var(--muted)' }}>{error ?? meta?.note}</div>
      )}
      <div style={{ flex: 1, minHeight: 0, display: 'flex', gap: 12, padding: 12 }}>
        <div style={{ flex: '1 1 55%', minWidth: 0, overflow: 'auto', background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10 }}>
          <table style={{ width: '100%', borderCollapse: 'collapse' }}>
            <thead>
              <tr><th style={th}>Time</th><th style={th}>Dir</th><th style={th}>Summary</th><th style={th}>Latency</th><th style={th}>Error</th></tr>
            </thead>
            <tbody>
              {frames.length === 0 && (
                <tr><td colSpan={5} style={{ ...td, color: 'var(--muted)', padding: 16 }}>No frames yet. They appear as the device is polled.</td></tr>
              )}
              {frames.map(f => {
                const isSel = f.seq === selectedSeq
                const isPair = selected != null && (f.seq === selected.pair_id)
                return (
                  <tr key={f.seq} onClick={() => setSelectedSeq(f.seq)} aria-selected={isSel}
                    style={{ cursor: 'pointer', background: isSel ? 'rgba(34,211,238,.12)' : isPair ? 'rgba(34,211,238,.05)' : undefined }}>
                    <td style={{ ...td, ...mono }}>{formatTime(f.ts_us)}</td>
                    <td style={{ ...td, color: f.dir === 'tx' ? 'var(--cyan)' : 'var(--fg)' }} title={f.dir === 'tx' ? 'Request from Voltrus' : 'Response from the device'}>{f.dir === 'tx' ? '→ req' : '← resp'}</td>
                    <td style={{ ...td, whiteSpace: 'normal' }}>{f.summary}</td>
                    <td style={{ ...td, ...mono }}>{formatLatency(f.latency_us)}</td>
                    <td style={{ ...td, color: 'var(--crit)' }}>{f.error?.title ?? ''}</td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
        <div style={{ flex: '1 1 45%', minWidth: 0, overflow: 'auto', display: 'flex', flexDirection: 'column', gap: 12 }}>
          {selected ? <FrameDetail frame={selected} field={selectedField} onField={setSelectedField} /> : (
            <div style={{ color: 'var(--muted)', fontSize: 13, padding: 12 }}>Select a frame to see its fields and bytes.</div>
          )}
        </div>
      </div>
    </div>
  )
}

function FrameDetail({ frame, field, onField }: { frame: TrafficFrame; field: TrafficField | null; onField: (f: TrafficField | null) => void }) {
  const box: React.CSSProperties = { background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, padding: 12 }
  const open = useMemo(() => new Set(field ? pathTo(frame.fields, field) ?? [] : []), [frame, field])
  return (<>
    {frame.error && (
      <div style={{ ...box, borderColor: 'var(--crit)' }}>
        <div style={{ fontWeight: 700, color: 'var(--crit)', marginBottom: 4 }}>{frame.error.title}</div>
        <div style={{ fontSize: 13, lineHeight: 1.5 }}>{frame.error.explanation}</div>
      </div>
    )}
    {frame.tag_refs && frame.tag_refs.length > 0 && (
      <div style={box}>
        <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', marginBottom: 6 }}>Tags</div>
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
          {frame.tag_refs.map(t => (
            <button key={t.key} disabled={t.offset == null}
              onClick={() => t.offset != null && onField({ id: '', label: '', value: null, display: t.tag_name ?? t.key, offset: t.offset, len: t.len ?? 0 })}
              title={t.offset == null ? 'This frame names the register but carries no value for it' : 'Highlight its bytes'}
              style={{ padding: '2px 8px', borderRadius: 4, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: t.offset == null ? 'default' : 'pointer', fontSize: 12 }}>
              {t.tag_name ? `${t.tag_name} (${t.key})` : t.key}
            </button>
          ))}
        </div>
      </div>
    )}
    <div style={box}>
      <div role="tree" aria-label="Decoded fields" style={{ fontSize: 12, fontFamily: 'var(--font-mono)' }}>
        {(frame.fields ?? []).map((f, i) => <FieldNode key={i} field={f} depth={0} selected={field} open={open} onSelect={onField} />)}
      </div>
    </div>
    <div style={box}>
      <HexPane raw={frame.raw} field={field} onByte={i => onField(fieldAt(frame.fields, i) ?? null)} />
    </div>
  </>)
}

function FieldNode({ field, depth, selected, open, onSelect }: {
  field: TrafficField; depth: number; selected: TrafficField | null; open: Set<TrafficField>; onSelect: (f: TrafficField) => void
}) {
  const kids = field.children ?? []
  const [expanded, setExpanded] = useState(depth < 2)
  const isOpen = expanded || open.has(field)
  const isSel = selected === field
  return (
    <div role="treeitem" aria-expanded={kids.length ? isOpen : undefined} aria-selected={isSel}>
      <div onClick={() => { onSelect(field); if (kids.length) setExpanded(!isOpen) }}
        title={field.id ? `${field.id} (bytes ${field.offset}-${field.offset + field.len - 1})` : undefined}
        style={{ cursor: 'pointer', padding: '1px 4px', marginLeft: depth * 14, borderRadius: 3, background: isSel ? 'rgba(34,211,238,.18)' : undefined }}>
        <span style={{ display: 'inline-block', width: 12, color: 'var(--muted)' }}>{kids.length ? (isOpen ? '▾' : '▸') : ''}</span>
        {field.display}
      </div>
      {isOpen && kids.map((k, i) => <FieldNode key={i} field={k} depth={depth + 1} selected={selected} open={open} onSelect={onSelect} />)}
    </div>
  )
}

function HexPane({ raw, field, onByte }: { raw?: string; field: TrafficField | null; onByte: (i: number) => void }) {
  const bytes = hexBytes(raw)
  const rows: number[][] = []
  for (let i = 0; i < bytes.length; i += 16) rows.push(bytes.slice(i, i + 16))
  const lit = (i: number) => field != null && i >= field.offset && i < field.offset + field.len
  const cell = (i: number): React.CSSProperties => ({ cursor: 'pointer', padding: '0 1px', borderRadius: 2, background: lit(i) ? 'rgba(34,211,238,.35)' : undefined })
  return (
    <div aria-label="Frame bytes" style={{ fontFamily: 'var(--font-mono)', fontSize: 12, lineHeight: 1.7, whiteSpace: 'pre', overflowX: 'auto' }}>
      {rows.map((row, r) => (
        <div key={r} style={{ display: 'flex', gap: 12 }}>
          <span style={{ color: 'var(--muted)' }}>{(r * 16).toString(16).padStart(4, '0')}</span>
          <span>{row.map((b, j) => {
            const i = r * 16 + j
            return <span key={j} onClick={() => onByte(i)} style={cell(i)}>{b.toString(16).padStart(2, '0')}{j === 7 ? '  ' : ' '}</span>
          })}</span>
          <span style={{ color: 'var(--muted)' }}>{row.map((b, j) => {
            const i = r * 16 + j
            return <span key={j} onClick={() => onByte(i)} style={cell(i)}>{b >= 32 && b < 127 ? String.fromCharCode(b) : '.'}</span>
          })}</span>
        </div>
      ))}
    </div>
  )
}
