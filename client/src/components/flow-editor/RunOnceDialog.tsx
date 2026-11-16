import { useEffect, useState } from 'react'
import type { FlowTestInput } from '../../types'
import { btnStyle, labelStyle, labelTextStyle, monoInputStyle } from '../../styles'
import { useRunFlow } from '../../queries'
import { isSignatureCancelled } from '../../signature/logic'

/**
 * The run's input from what was typed: `undefined` when empty (the server
 * runs on its default input), `null` when it is not JSON. An object with a
 * `payload` key is taken as `{ payload, metadata }`; anything else is the
 * payload itself.
 */
export function parseRunInput(text: string): FlowTestInput | undefined | null {
  if (!text.trim()) return undefined
  try {
    const v = JSON.parse(text)
    if (v && typeof v === 'object' && !Array.isArray(v) && 'payload' in v) {
      return { payload: v.payload, metadata: v.metadata ?? {} }
    }
    return { payload: v, metadata: {} }
  } catch {
    return null
  }
}

/**
 * Run the saved flow once, for real, with an optional JSON input. For
 * operators, who may run a flow but not edit it. An edition that wants the
 * run signed answers 428; the shared signature prompt then asks.
 */
export function RunOnceDialog({ flowId, flowName, onClose, onDone }: {
  flowId: string
  flowName: string
  onClose: () => void
  onDone: () => void
}) {
  const [text, setText] = useState('')
  const [error, setError] = useState<string | null>(null)
  const runMut = useRunFlow()
  const busy = runMut.isPending

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape' && !busy) onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [busy, onClose])

  const submit = async (e: React.FormEvent) => {
    e.preventDefault()
    const input = parseRunInput(text)
    if (input === null) { setError('The input is not valid JSON.'); return }
    setError(null)
    try {
      await runMut.mutateAsync({ id: flowId, input })
      onDone()
    } catch (err) {
      // Closing the signature prompt abandons the run: nothing to report.
      if (isSignatureCancelled(err)) return
      setError(`Run failed: ${err instanceof Error ? err.message : String(err)}`)
    }
  }

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.5)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 900 }}
      onClick={() => { if (!busy) onClose() }}>
      <form role="dialog" aria-modal="true" aria-labelledby="run-once-title" onSubmit={submit} onClick={e => e.stopPropagation()}
        style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, width: 460, maxWidth: 'calc(100vw - 32px)', padding: 20, display: 'flex', flexDirection: 'column', gap: 12 }}>
        <div id="run-once-title" style={{ fontSize: 15, fontWeight: 600 }}>Run “{flowName}” once</div>
        <div style={{ fontSize: 13, color: 'var(--muted)' }}>
          Runs the saved flow now, for real: writes, alarms and webhooks act.
        </div>
        <label style={labelStyle}>
          <span style={labelTextStyle}>Input JSON (optional)</span>
          <textarea aria-label="Run input JSON" spellCheck={false} value={text} rows={6} autoFocus
            placeholder={'{ "payload": { "value": 1 } }'}
            onChange={e => { setText(e.target.value); setError(null) }}
            style={{ ...monoInputStyle, resize: 'vertical', fontSize: 12 }} />
        </label>
        {error && <div role="alert" style={{ fontSize: 12.5, color: 'var(--crit)' }}>{error}</div>}
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
          <button type="button" style={btnStyle} onClick={onClose} disabled={busy}>Cancel</button>
          <button type="submit" disabled={busy} style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }}>
            {busy ? 'Running…' : 'Run now'}
          </button>
        </div>
      </form>
    </div>
  )
}
