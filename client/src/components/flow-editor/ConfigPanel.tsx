import { useState } from 'react'
import type { FlowCatalogue, FlowTriggerConfig } from '../../types'
import { btnStyle, inputStyle, monoInputStyle } from '../../styles'
import { useWebhookToken } from '../../queries'
import { colorFor, iconFor } from './catalogue'
import { FieldForm, type PortMap } from './FieldForm'
import { outputsFor, validateFields, type EdNode, type SwitchRule } from './model'

const section: React.CSSProperties = { padding: 16, borderBottom: '1px solid var(--border)', display: 'flex', flexDirection: 'column', gap: 12 }
const small = { ...btnStyle, padding: '4px 10px', fontSize: 11.5 }
const heading: React.CSSProperties = { fontSize: 10.5, fontWeight: 700, textTransform: 'uppercase', letterSpacing: '.06em', color: 'var(--muted)' }

function Header({ type, category, title, subtitle, description }: { type: string; category: string; title: string; subtitle: string; description?: string }) {
  const color = colorFor(category)
  return (
    <div style={{ display: 'flex', gap: 10, alignItems: 'flex-start' }}>
      <span style={{
        width: 34, height: 34, borderRadius: 8, flexShrink: 0, display: 'flex', alignItems: 'center', justifyContent: 'center',
        background: `color-mix(in srgb, ${color} 16%, transparent)`, border: `1px solid color-mix(in srgb, ${color} 35%, transparent)`,
      }}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d={iconFor(type, category)} /></svg>
      </span>
      <div style={{ minWidth: 0 }}>
        <div style={{ fontSize: 14, fontWeight: 700 }}>{title}</div>
        <div style={{ fontSize: 11, color: 'var(--muted)', fontFamily: 'var(--font-mono)' }}>{subtitle}</div>
        {description && <div style={{ fontSize: 12, color: 'var(--muted)', marginTop: 6, lineHeight: 1.45 }}>{description}</div>}
      </div>
    </div>
  )
}

function ErrorBox({ message }: { message: string }) {
  return (
    <div role="alert" style={{
      fontSize: 12, color: 'var(--crit)', border: '1px solid color-mix(in srgb, var(--crit) 45%, transparent)',
      background: 'color-mix(in srgb, var(--crit) 8%, transparent)', borderRadius: 7, padding: '8px 10px', lineHeight: 1.45,
    }}>{message}</div>
  )
}

export function PanelShell({ children }: { children: React.ReactNode }) {
  return (
    <aside aria-label="Node settings" style={{
      width: 320, flexShrink: 0, borderLeft: '1px solid var(--border)', background: 'var(--card)',
      overflow: 'auto', minHeight: 0,
    }}>{children}</aside>
  )
}

interface NodePanelProps {
  node: EdNode
  catalogue: FlowCatalogue
  readOnly: boolean
  serverError?: string
  onConfig: (key: string, value: unknown) => void
  onName: (name: string) => void
  onRules: (rules: SwitchRule[], map: PortMap) => void
  onDuplicate: () => void
  onDelete: () => void
}

export function NodePanel({ node, catalogue, readOnly, serverError, onConfig, onName, onRules, onDuplicate, onDelete }: NodePanelProps) {
  const def = catalogue.nodes.find(d => d.type === node.data.type)
  const fields = def?.fields ?? []
  const errors = validateFields(fields, node.data.config)
  const outputs = outputsFor(node.data.type, node.data.config, def)
  return (
    <PanelShell>
      <div style={section}>
        <Header type={node.data.type} category={def?.category ?? 'advanced'} title={def?.label ?? node.data.type}
          subtitle={`${node.data.type} · ${node.id}`} description={def?.description} />
        {!def && <ErrorBox message={`The server does not know the node type “${node.data.type}”. Its settings are kept as they are.`} />}
        {serverError && <ErrorBox message={serverError} />}
        {def?.side_effect && (
          <div style={{ fontSize: 11.5, color: 'var(--warn)' }}>Has side effects: skipped in test runs, real in manual and live runs.</div>
        )}
        <label style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
          <span style={{ fontSize: 11, color: 'var(--muted)', fontWeight: 600 }}>Label</span>
          <input style={inputStyle} placeholder={def?.label ?? node.data.type} value={node.data.name ?? ''}
            readOnly={readOnly} onChange={e => onName(e.target.value)} />
        </label>
      </div>
      <div style={section}>
        <div style={heading}>Settings</div>
        <FieldForm fields={fields} config={node.data.config} errors={errors} readOnly={readOnly}
          nodeType={node.data.type} onChange={onConfig} onRules={onRules} />
      </div>
      <div style={section}>
        <div style={heading}>Outputs</div>
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
          {outputs.map((o, i) => (
            <span key={i} style={{
              fontSize: 11, fontFamily: 'var(--font-mono)', padding: '2px 8px', borderRadius: 999,
              border: '1px solid var(--border)', color: 'var(--muted)',
            }}>{i}: {o}</span>
          ))}
        </div>
      </div>
      {!readOnly && (
        <div style={{ ...section, flexDirection: 'row', borderBottom: 'none' }}>
          <button type="button" style={small} onClick={onDuplicate}>Duplicate</button>
          <button type="button" style={{ ...small, color: 'var(--crit)' }} onClick={onDelete}>Delete</button>
        </div>
      )}
    </PanelShell>
  )
}

interface TriggerPanelProps {
  trigger: FlowTriggerConfig
  catalogue: FlowCatalogue
  readOnly: boolean
  serverError?: string
  flowId: string
  /** The flow exists on the server (webhook tokens need a saved flow). */
  saved: boolean
  /** May create webhook tokens (admins only on the server). */
  canManageWebhook?: boolean
  onChange: (key: string, value: unknown) => void
  onType: (type: string) => void
}

export function TriggerPanel({ trigger, catalogue, readOnly, serverError, flowId, saved, canManageWebhook = true, onChange, onType }: TriggerPanelProps) {
  const def = catalogue.triggers.find(t => t.type === trigger.type)
  const fields = def?.fields ?? []
  const errors = validateFields(fields, trigger)
  const types = catalogue.triggers.some(t => t.type === trigger.type)
    ? catalogue.triggers : [...catalogue.triggers, { type: trigger.type, label: trigger.type, fields: [] }]
  return (
    <PanelShell>
      <div style={section}>
        <Header type={trigger.type} category="trigger" title={def?.label ?? trigger.type} subtitle="trigger" description={def?.description} />
        {serverError && <ErrorBox message={serverError} />}
        <label style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
          <span style={{ fontSize: 11, color: 'var(--muted)', fontWeight: 600 }}>Trigger type</span>
          <select aria-label="Trigger type" style={inputStyle} value={trigger.type} disabled={readOnly} onChange={e => onType(e.target.value)}>
            {types.map(t => <option key={t.type} value={t.type}>{t.label}</option>)}
          </select>
        </label>
      </div>
      <div style={section}>
        <div style={heading}>Settings</div>
        <FieldForm fields={fields} config={trigger} errors={errors} readOnly={readOnly} onChange={onChange} />
      </div>
      {trigger.type === 'webhook' && (
        <div style={section}>
          <WebhookPanel flowId={flowId} saved={saved} readOnly={readOnly || !canManageWebhook} />
        </div>
      )}
    </PanelShell>
  )
}

function CopyButton({ text, label = 'Copy' }: { text: string; label?: string }) {
  const [done, setDone] = useState(false)
  return (
    <button type="button" style={small} onClick={() => {
      navigator.clipboard?.writeText(text).then(() => { setDone(true); setTimeout(() => setDone(false), 1500) }).catch(() => {})
    }}>{done ? 'Copied' : label}</button>
  )
}

export function WebhookPanel({ flowId, saved, readOnly }: { flowId: string; saved: boolean; readOnly: boolean }) {
  const tokenMut = useWebhookToken()
  const [token, setToken] = useState<string | null>(null)
  const url = `${typeof location !== 'undefined' ? location.origin : ''}/api/v1/flows/${flowId}/webhook`
  const curl = `curl -X POST '${url}' \\\n  -H 'X-Flow-Token: ${token ?? '<token>'}' \\\n  -H 'Content-Type: application/json' \\\n  -d '{"value": 42}'`
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
      <div style={heading}>Webhook</div>
      <div style={{ display: 'flex', gap: 6 }}>
        <input readOnly aria-label="Webhook URL" value={url} style={{ ...monoInputStyle, fontSize: 11 }} />
        <CopyButton text={url} />
      </div>
      {!saved ? (
        <div style={{ fontSize: 12, color: 'var(--muted)' }}>Save the flow to create its webhook token.</div>
      ) : readOnly ? (
        <div style={{ fontSize: 12, color: 'var(--muted)' }}>An admin creates the token; requests send it in the X-Flow-Token header.</div>
      ) : (
        <>
          <button type="button" style={{ ...small, alignSelf: 'flex-start' }} disabled={tokenMut.isPending}
            onClick={() => {
              if (token === null && !confirm('Generate a new token? Any previous token stops working.')) return
              tokenMut.mutate(flowId, { onSuccess: r => setToken(r.token) })
            }}>
            {tokenMut.isPending ? 'Generating…' : token ? 'Regenerate token' : 'Generate token'}
          </button>
          {tokenMut.error && <ErrorBox message={tokenMut.error.message} />}
          {token && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
              <div style={{ fontSize: 11.5, color: 'var(--warn)' }}>Copy it now: it is shown only once.</div>
              <div style={{ display: 'flex', gap: 6 }}>
                <input readOnly aria-label="Webhook token" value={token} style={{ ...monoInputStyle, fontSize: 11 }} />
                <CopyButton text={token} />
              </div>
            </div>
          )}
        </>
      )}
      <div style={{ position: 'relative' }}>
        <pre style={{
          margin: 0, fontSize: 10.5, fontFamily: 'var(--font-mono)', background: 'var(--code-bg)', border: '1px solid var(--border)',
          borderRadius: 7, padding: '8px 10px', overflow: 'auto', whiteSpace: 'pre',
        }}>{curl}</pre>
        <div style={{ position: 'absolute', top: 6, right: 6 }}><CopyButton text={curl} label="Copy curl" /></div>
      </div>
    </div>
  )
}

export function EmptyPanel({ count, readOnly }: { count: number; readOnly: boolean }) {
  return (
    <PanelShell>
      <div style={{ ...section, borderBottom: 'none', color: 'var(--muted)', fontSize: 12.5, lineHeight: 1.55 }}>
        {count > 1
          ? <div><b style={{ color: 'var(--fg)' }}>{count} nodes selected.</b> Drag to move them together, copy with Ctrl+C, or delete with Delete.</div>
          : readOnly
            ? <div>Select a node to see its settings. You have read-only access to flows.</div>
            : <div>Select a node to edit it. Drag from an output port to a node's input to connect them. Press <kbd>?</kbd> for shortcuts.</div>}
      </div>
    </PanelShell>
  )
}
