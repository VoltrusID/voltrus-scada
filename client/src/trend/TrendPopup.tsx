import { useEffect, useMemo, useState, useSyncExternalStore } from 'react'
import type { SensorConfig } from '../types'
import { fetchJson } from '../queries/fetch'
import { historyUrl } from '../historyQuery'
import { expandCompactData } from '../utils'
import { MAX_PENS, addPen, normalisePoints, penAxis, penColor, tickLabel, timeRange, yOf } from './trendModel'
import type { PenAxis, Point } from './trendModel'

// ── Which tags the popup shows: open from anywhere with openTrend(tagId) ──

let pens: number[] = []
const listeners = new Set<() => void>()
const emit = () => listeners.forEach(l => l())

/** Open the trend popup for a tag (or add it as a pen when the popup is open). */
export function openTrend(tagId: number) {
  pens = addPen(pens, tagId)
  emit()
}

function setPens(next: number[]) {
  pens = next
  emit()
}

function usePens(): number[] {
  return useSyncExternalStore(
    l => { listeners.add(l); return () => { listeners.delete(l) } },
    () => pens,
  )
}

const RANGES = ['15m', '1h', '8h', '24h'] as const

const W = 640
const H = 260
const AXIS_W = 46

/** The trend popup: every pen on its own autoscaled axis; add pens from any tag. */
export function TrendPopup({ sensors }: { sensors: SensorConfig[] }) {
  const ids = usePens()
  const [range, setRange] = useState<(typeof RANGES)[number]>('1h')
  const [data, setData] = useState<Record<number, Point[]>>({})
  const [error, setError] = useState('')
  const [adding, setAdding] = useState('')

  const tags = useMemo(() => ids.map(id => sensors.find(s => s.id === id)).filter(Boolean) as SensorConfig[], [ids, sensors])

  useEffect(() => {
    if (tags.length === 0) return
    let cancelled = false
    const load = () => {
      const keys = [...new Set(tags.map(t => t.key))].join(',')
      fetchJson<{ data: Record<string, any> }>(historyUrl({ range, sensor: keys, keyedBy: 'id' }))
        .then(r => {
          if (cancelled) return
          const expanded = expandCompactData(r.data ?? {})
          const next: Record<number, Point[]> = {}
          for (const t of tags) next[t.id] = normalisePoints(expanded[String(t.id)] ?? [])
          setData(next)
          setError('')
        })
        .catch(e => { if (!cancelled) setError(e instanceof Error ? e.message : String(e)) })
    }
    load()
    const timer = setInterval(load, 10_000)
    return () => { cancelled = true; clearInterval(timer) }
  }, [tags, range])

  useEffect(() => {
    if (ids.length === 0) return
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setPens([]) }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [ids.length])

  if (ids.length === 0) return null

  const series = tags.map((t, i) => ({ tag: t, color: penColor(i), points: data[t.id] ?? [] }))
  const axes: (PenAxis | null)[] = series.map(s => penAxis(s.points))
  const span = timeRange(series.map(s => s.points))
  const left = AXIS_W * series.length
  const plotW = W - left
  const x = (ts: number) => (span ? left + ((ts - span[0]) / (span[1] - span[0])) * plotW : left)
  const fmtTime = (ts: number) => new Date(ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })

  return (
    <div role="dialog" aria-label="Trend" style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.5)', zIndex: 90, display: 'flex', alignItems: 'center', justifyContent: 'center' }} onClick={() => setPens([])}>
      <div onClick={e => e.stopPropagation()} style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, width: W + 32, maxWidth: '96vw', padding: 16, color: 'var(--fg)', fontSize: 12 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10, flexWrap: 'wrap' }}>
          <strong style={{ fontSize: 14 }}>Trend</strong>
          <div role="group" aria-label="Time range" style={{ display: 'flex', gap: 2 }}>
            {RANGES.map(r => (
              <button key={r} onClick={() => setRange(r)} aria-pressed={range === r} style={{
                padding: '2px 8px', borderRadius: 4, fontSize: 11, cursor: 'pointer',
                border: `1px solid ${range === r ? 'var(--cyan)' : 'var(--border)'}`, background: 'transparent', color: range === r ? 'var(--cyan)' : 'var(--fg)',
              }}>{r}</button>
            ))}
          </div>
          <select aria-label="Add pen" value={adding} disabled={ids.length >= MAX_PENS}
            onChange={e => { const id = Number(e.target.value); if (id) setPens(addPen(ids, id)); setAdding('') }}
            style={{ marginLeft: 'auto', padding: '3px 6px', fontSize: 11, background: 'var(--input-bg)', color: 'var(--fg)', border: '1px solid var(--border)', borderRadius: 4, maxWidth: 220 }}>
            <option value="">{ids.length >= MAX_PENS ? `Up to ${MAX_PENS} pens` : '+ Add pen'}</option>
            {sensors.filter(s => !ids.includes(s.id)).map(s => <option key={s.id} value={s.id}>{s.name} ({s.key})</option>)}
          </select>
          <button onClick={() => setPens([])} aria-label="Close trend" style={{ background: 'none', border: 'none', color: 'var(--text-muted)', cursor: 'pointer', fontSize: 18, lineHeight: 1 }}>×</button>
        </div>

        <svg width="100%" viewBox={`0 0 ${W} ${H + 18}`} role="img" aria-label={`Trend of ${series.map(s => s.tag.name).join(', ')}`} style={{ display: 'block' }}>
          <rect x={left} y={0} width={plotW} height={H} fill="none" stroke="var(--border)" />
          {[0.25, 0.5, 0.75].map(f => <line key={f} x1={left} x2={W} y1={H * f} y2={H * f} stroke="var(--border)" strokeDasharray="2 4" />)}
          {series.map((s, i) => {
            const axis = axes[i]
            if (!axis) return null
            const ax = AXIS_W * (i + 1) - 4
            return (
              <g key={s.tag.id}>
                <line x1={ax} x2={ax} y1={0} y2={H} stroke={s.color} strokeWidth={2} />
                {axis.ticks.map((t, k) => (
                  <text key={k} x={ax - 4} y={Math.min(H - 2, Math.max(10, yOf(axis, t, H) + 4))} textAnchor="end" fontSize={10} fill={s.color} fontFamily="var(--font-mono)">{tickLabel(t, axis)}</text>
                ))}
                {s.points.length > 1 && (
                  <path fill="none" stroke={s.color} strokeWidth={1.6} strokeLinejoin="round"
                    d={s.points.map((p, k) => `${k === 0 ? 'M' : 'L'}${x(p.ts).toFixed(1)} ${yOf(axis, p.value, H).toFixed(1)}`).join(' ')} />
                )}
              </g>
            )
          })}
          {span && <>
            <text x={left} y={H + 14} fontSize={10} fill="var(--text-muted)">{fmtTime(span[0])}</text>
            <text x={W} y={H + 14} fontSize={10} fill="var(--text-muted)" textAnchor="end">{fmtTime(span[1])}</text>
          </>}
          {!span && <text x={left + plotW / 2} y={H / 2} textAnchor="middle" fontSize={12} fill="var(--text-muted)">{error || 'No history in this range yet'}</text>}
        </svg>

        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, marginTop: 10 }}>
          {series.map((s, i) => {
            const last = s.points[s.points.length - 1]
            return (
              <span key={s.tag.id} style={{ display: 'inline-flex', alignItems: 'center', gap: 6, padding: '3px 8px', borderRadius: 4, border: `1px solid ${s.color}` }}>
                <span style={{ width: 10, height: 3, background: s.color }} />
                <span>{s.tag.name}</span>
                <span style={{ fontFamily: 'var(--font-mono)', color: 'var(--text-muted)' }}>
                  {last ? last.value.toFixed(s.tag.decimals ?? 1) : '—'} {s.tag.unit}
                  {axes[i] ? ` · axis ${tickLabel(axes[i]!.lo, axes[i]!)}–${tickLabel(axes[i]!.hi, axes[i]!)}` : ''}
                </span>
                <button onClick={() => setPens(ids.filter(id => id !== s.tag.id))} aria-label={`Remove ${s.tag.name} from the trend`} style={{ background: 'none', border: 'none', color: 'var(--text-muted)', cursor: 'pointer', padding: 0, fontSize: 14, lineHeight: 1 }}>×</button>
              </span>
            )
          })}
        </div>
      </div>
    </div>
  )
}
