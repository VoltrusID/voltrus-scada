import { useState } from 'react'
import type { FlowCatalogue, FlowRun, FlowTestResult, FlowTraceStep } from '../../types'
import { btnStyle, monoInputStyle } from '../../styles'
import { colorFor } from './catalogue'
import { TRIGGER_ID, outputsFor, runTrace, wouldText, type EdNode } from './model'

export type TraceSource =
  | { kind: 'test'; result: FlowTestResult }
  | { kind: 'run'; run: FlowRun }

interface Props {
  open: boolean
  onToggle: () => void
  tab: 'test' | 'runs'
  onTab: (t: 'test' | 'runs') => void
  readOnly: boolean
  catalogue: FlowCatalogue
  nodes: EdNode[]
  input: string
  onInput: (s: string) => void
  onResetInput: () => void
  onTest: () => void
  testing: boolean
  testError: string | null
  source: TraceSource | null
  onClearTrace: () => void
  selectedNodeId: string | null
  onSelectNode: (id: string) => void
  runs: FlowRun[] | undefined
  runsError: string | null
  onShowRun: (run: FlowRun) => void
  saved: boolean
}

const small = { ...btnStyle, padding: '4px 10px', fontSize: 11.5 }

function tabBtn(active: boolean): React.CSSProperties {
  return {
    background: 'none', border: 'none', borderBottom: `2px solid ${active ? 'var(--green)' : 'transparent'}`,
    color: active ? 'var(--fg)' : 'var(--muted)', fontSize: 12, fontWeight: 600, padding: '9px 2px', cursor: 'pointer',
    fontFamily: 'var(--font-ui)',
  }
}

export function fmtWhen(ts: number): string {
  const ms = ts < 1e12 ? ts * 1000 : ts
  const d = new Date(ms)
  const diff = Date.now() - ms
  if (diff >= 0 && diff < 60_000) return 'just now'
  if (diff >= 0 && diff < 3_600_000) return `${Math.floor(diff / 60_000)} min ago`
  if (diff >= 0 && diff < 86_400_000) return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  return d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
}

/** "2 outputs reached: Write tag, Debug"; early servers sent only a count. */
export function reachedText(reached: string[] | number | undefined, nodes: EdNode[]): string {
  const ids = Array.isArray(reached) ? reached : []
  const n = Array.isArray(reached) ? reached.length : Number(reached) || 0
  const head = `${n} output${n === 1 ? '' : 's'} reached`
  if (!ids.length) return head
  const names = ids.map(id => nodes.find(x => x.id === id)?.data.name || id)
  return `${head}: ${names.join(', ')}`
}

/** What a skipped side-effect node would have done, from the server's `would_*` output. */
export function wouldFromOutput(output: unknown): string | null {
  if (!output || typeof output !== 'object') return null
  const o = output as Record<string, Record<string, unknown> | undefined>
  const w = o.would_write
  if (w) return `would write ${JSON.stringify(w.value)} to tag ${w.data_source_id}`
  const n = o.would_notify
  if (n) return `would notify via ${n.channel}${n.to ? ` to ${n.to}` : ''}: ${n.message}`
  const p = o.would_post
  if (p) return `would POST to ${p.url}`
  const a = o.would_raise_alarm
  if (a) return `would raise ${a.severity ?? ''} alarm "${a.message ?? ''}"`.replace('  ', ' ')
  const s = o.would_set
  if (s) return `would set ${s.source_key ?? s.metric ?? 'a flow tag'} to ${JSON.stringify(s.value)}`
  return null
}

export function TestPanel(p: Props) {
  return (
    <section aria-label="Test and runs" style={{
      borderTop: '1px solid var(--border)', background: 'var(--card)', flexShrink: 0,
      height: p.open ? 300 : 38, display: 'flex', flexDirection: 'column', transition: 'height .18s ease',
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 16, padding: '0 14px', borderBottom: p.open ? '1px solid var(--border)' : 'none', flexShrink: 0 }}>
        <button type="button" aria-label={p.open ? 'Collapse panel' : 'Expand panel'} onClick={p.onToggle}
          style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', padding: 4, display: 'flex' }}>
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5"
            style={{ transform: p.open ? 'none' : 'rotate(180deg)', transition: 'transform .15s' }}><path d="M6 9l6 6 6-6" /></svg>
        </button>
        {!p.readOnly && <button type="button" style={tabBtn(p.tab === 'test')} onClick={() => { p.onTab('test'); if (!p.open) p.onToggle() }}>Test run</button>}
        <button type="button" style={tabBtn(p.tab === 'runs')} onClick={() => { p.onTab('runs'); if (!p.open) p.onToggle() }}>
          Recent runs{p.runs?.length ? ` (${p.runs.length})` : ''}
        </button>
        <div style={{ flex: 1 }} />
        {p.source && (
          <>
            <span style={{ fontSize: 11.5, color: 'var(--muted)' }}>
              {p.source.kind === 'test' ? 'Showing test trace' : `Showing run #${p.source.run.id}`}
            </span>
            <button type="button" style={small} onClick={p.onClearTrace}>Clear highlight</button>
          </>
        )}
      </div>
      {p.open && (
        <div style={{ flex: 1, minHeight: 0, display: 'flex' }}>
          {p.tab === 'test' && !p.readOnly ? <TestTab {...p} /> : <RunsTab {...p} />}
        </div>
      )}
    </section>
  )
}

function TestTab(p: Props) {
  const steps = p.source?.kind === 'test' ? p.source.result.trace : []
  return (
    <>
      <div style={{ width: 340, flexShrink: 0, padding: 12, borderRight: '1px solid var(--border)', display: 'flex', flexDirection: 'column', gap: 8 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <span style={{ fontSize: 11, color: 'var(--muted)', fontWeight: 600, flex: 1 }}>Input (payload + metadata)</span>
          <button type="button" style={{ ...small, padding: '2px 8px' }} onClick={p.onResetInput}>Sample</button>
        </div>
        <textarea aria-label="Test input JSON" spellCheck={false} value={p.input} onChange={e => p.onInput(e.target.value)}
          style={{ ...monoInputStyle, flex: 1, resize: 'none', fontSize: 11.5 }} />
        <button type="button" onClick={p.onTest} disabled={p.testing}
          style={{ ...btnStyle, justifyContent: 'center', borderColor: 'var(--green)', color: 'var(--green)' }}>
          {p.testing ? 'Running…' : 'Run test (dry run)'}
        </button>
      </div>
      <div style={{ flex: 1, minWidth: 0, overflow: 'auto' }}>
        {p.testError && <div role="alert" style={{ padding: 12, fontSize: 12, color: 'var(--crit)' }}>{p.testError}</div>}
        {!p.testError && p.source?.kind !== 'test' && (
          <div style={{ padding: 16, fontSize: 12, color: 'var(--muted)', lineHeight: 1.5 }}>
            Runs the graph as it is now, saved or not. Nodes with side effects are skipped, so nothing is written or sent.
          </div>
        )}
        {p.source?.kind === 'test' && (
          <>
            <div style={{ padding: '8px 12px', fontSize: 12, display: 'flex', gap: 12, borderBottom: '1px solid var(--border)' }}>
              <b style={{ color: p.source.result.ok ? 'var(--green)' : 'var(--crit)' }}>{p.source.result.ok ? 'Passed' : 'Failed'}</b>
              <span style={{ color: 'var(--muted)' }}>{steps.length} step{steps.length === 1 ? '' : 's'}</span>
              <span style={{ color: 'var(--muted)' }}>{reachedText(p.source.result.outputs_reached, p.nodes)}</span>
            </div>
            {!p.source.result.ok && p.source.result.error && (
              <div role="alert" style={{ padding: '8px 12px', fontSize: 12, color: 'var(--crit)', borderBottom: '1px solid var(--border)' }}>
                {p.source.result.error}
              </div>
            )}
            <TraceList steps={steps} {...p} />
          </>
        )}
      </div>
    </>
  )
}

export function TraceList({ steps, catalogue, nodes, selectedNodeId, onSelectNode }: {
  steps: (FlowTraceStep | { node_id: string; port: number | null; error: string | null })[]
  catalogue: FlowCatalogue; nodes: EdNode[]; selectedNodeId: string | null; onSelectNode: (id: string) => void
}) {
  const [open, setOpen] = useState<number | null>(null)
  if (!steps.length) return <div style={{ padding: 12, fontSize: 12, color: 'var(--muted)' }}>No node ran.</div>
  return (
    <ol aria-label="Trace" style={{ listStyle: 'none', margin: 0, padding: 0 }}>
      {steps.map((s, i) => {
        const node = nodes.find(n => n.id === s.node_id)
        const type = node?.data.type ?? ('type' in s ? s.type : '')
        const def = catalogue.nodes.find(d => d.type === type)
        const isTrigger = s.node_id === TRIGGER_ID
        const outs = node ? outputsFor(type, node.data.config, def) : isTrigger ? ['out'] : []
        const portName = s.port === null || s.port === undefined ? null : outs[s.port] ?? `port ${s.port}`
        const full = 'input' in s ? s : null
        const skipped = full?.side_effect_skipped
        const selected = selectedNodeId === s.node_id
        const expanded = open === i || (open === null && selected && !!full)
        return (
          <li key={i} style={{ borderBottom: '1px solid var(--border)' }}>
            <button type="button" data-testid={`trace-row-${s.node_id}`}
              onClick={() => { onSelectNode(s.node_id); setOpen(expanded ? -1 : i) }}
              style={{
                width: '100%', display: 'grid', gridTemplateColumns: '20px 1fr auto auto', gap: 10, alignItems: 'center',
                padding: '7px 12px', background: selected ? 'var(--hover-overlay)' : 'none', border: 'none',
                color: 'var(--fg)', cursor: 'pointer', textAlign: 'left', fontFamily: 'var(--font-ui)', fontSize: 12,
              }}>
              <span style={{ color: 'var(--muted)', fontFamily: 'var(--font-mono)', fontSize: 10.5 }}>{i + 1}</span>
              <span style={{ display: 'flex', gap: 8, alignItems: 'center', minWidth: 0 }}>
                <span style={{ width: 7, height: 7, borderRadius: 2, flexShrink: 0, background: colorFor(isTrigger ? 'trigger' : def?.category) }} />
                <b style={{ fontWeight: 600, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                  {node?.data.name || def?.label || (isTrigger ? 'Trigger' : type || s.node_id)}
                </b>
                <span style={{ color: 'var(--muted)', fontFamily: 'var(--font-mono)', fontSize: 10.5 }}>{s.node_id}</span>
                {s.error ? <span style={{ color: 'var(--crit)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{s.error}</span>
                  : skipped ? <span style={{ color: 'var(--warn)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                      skipped (dry run): {wouldFromOutput(full?.output) ?? wouldText(type, node?.data.config ?? {}, def)}</span>
                  : null}
              </span>
              <span style={{ fontFamily: 'var(--font-mono)', fontSize: 10.5, color: portName ? 'var(--green)' : 'var(--muted)' }}>
                {portName ? `→ ${portName}` : s.error ? 'error' : 'stopped'}
              </span>
              <span style={{ fontFamily: 'var(--font-mono)', fontSize: 10.5, color: 'var(--muted)', minWidth: 52, textAlign: 'right' }}>
                {full ? (full.duration_us >= 1000 ? `${(full.duration_us / 1000).toFixed(1)} ms` : `${full.duration_us} µs`) : ''}
              </span>
            </button>
            {expanded && full && (
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8, padding: '0 12px 10px 42px' }}>
                <JsonBlock label="Input" value={full.input} />
                <JsonBlock label="Output" value={full.output} />
              </div>
            )}
          </li>
        )
      })}
    </ol>
  )
}

function JsonBlock({ label, value }: { label: string; value: unknown }) {
  return (
    <div style={{ minWidth: 0 }}>
      <div style={{ fontSize: 10.5, color: 'var(--muted)', fontWeight: 600, marginBottom: 3 }}>{label}</div>
      <pre style={{
        margin: 0, fontSize: 10.5, fontFamily: 'var(--font-mono)', background: 'var(--code-bg)', border: '1px solid var(--border)',
        borderRadius: 6, padding: '6px 8px', maxHeight: 140, overflow: 'auto', whiteSpace: 'pre-wrap', wordBreak: 'break-word',
      }}>{value === undefined || value === null ? '—' : JSON.stringify(value, null, 2)}</pre>
    </div>
  )
}

function RunsTab(p: Props) {
  if (!p.saved) return <div style={{ padding: 16, fontSize: 12, color: 'var(--muted)' }}>Save the flow to run it and see its run log.</div>
  if (p.runsError) return <div style={{ padding: 16, fontSize: 12, color: 'var(--muted)' }}>Run log unavailable: {p.runsError}</div>
  if (!p.runs) return <div style={{ padding: 16, fontSize: 12, color: 'var(--muted)' }}>Loading runs…</div>
  if (!p.runs.length) return <div style={{ padding: 16, fontSize: 12, color: 'var(--muted)' }}>No runs yet.</div>
  const shown = p.source?.kind === 'run' ? p.source.run : null
  return (
    <>
      <ul aria-label="Recent runs" style={{ listStyle: 'none', margin: 0, padding: 0, width: 360, flexShrink: 0, overflow: 'auto', borderRight: '1px solid var(--border)' }}>
        {p.runs.map(r => {
          const steps = runTrace(r.trace)
          const active = shown?.id === r.id
          return (
            <li key={r.id}>
              <button type="button" onClick={() => p.onShowRun(r)} style={{
                width: '100%', display: 'flex', gap: 10, alignItems: 'center', padding: '7px 12px', fontSize: 12,
                border: 'none', borderBottom: '1px solid var(--border)', background: active ? 'var(--hover-overlay)' : 'none',
                color: 'var(--fg)', cursor: 'pointer', textAlign: 'left', fontFamily: 'var(--font-ui)',
              }}>
                <span style={{ width: 8, height: 8, borderRadius: '50%', background: r.success ? 'var(--green)' : 'var(--crit)', flexShrink: 0 }} />
                <span style={{ flex: 1, minWidth: 0 }}>
                  <span style={{ fontWeight: 600 }}>{r.success ? 'Succeeded' : 'Failed'}</span>
                  <span style={{ color: 'var(--muted)' }}> · {fmtWhen(r.started_at)}</span>
                  {r.error && <div style={{ color: 'var(--crit)', fontSize: 11, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{r.error}</div>}
                </span>
                <span style={{ color: 'var(--muted)', fontFamily: 'var(--font-mono)', fontSize: 10.5 }}>{r.duration_ms} ms</span>
                <span style={{ color: 'var(--muted)', fontSize: 10.5 }}>{steps.length ? `${steps.length} step${steps.length === 1 ? '' : 's'}` : ''}</span>
              </button>
            </li>
          )
        })}
      </ul>
      <div style={{ flex: 1, minWidth: 0, overflow: 'auto' }}>
        {shown ? (
          runTrace(shown.trace).length
            ? <TraceList steps={runTrace(shown.trace)} {...p} />
            : <div style={{ padding: 16, fontSize: 12, color: 'var(--muted)' }}>This run has no per-node trace.{shown.output ? ' Output:' : ''}
                {shown.output && <pre style={{ fontFamily: 'var(--font-mono)', fontSize: 11 }}>{shown.output}</pre>}</div>
        ) : <div style={{ padding: 16, fontSize: 12, color: 'var(--muted)' }}>Pick a run to overlay its path on the canvas.</div>}
      </div>
    </>
  )
}
