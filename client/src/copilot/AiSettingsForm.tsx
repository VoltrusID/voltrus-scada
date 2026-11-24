import { useState } from 'react'
import { useAiSettings, useUpdateAiSettings, useTestAiSettings } from '../queries/copilot'
import { availableAiProviders, BASE_URL_EXAMPLES, OPENAI_COMPATIBLE, buildAiSettingsBody, keyWillBeRemoved } from './aiSettings'

const labelStyle = { display: 'block', fontSize: 12, color: 'var(--muted)', marginBottom: 4 } as const
const inputStyle = { width: '100%', padding: '8px 12px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)', fontSize: 14, boxSizing: 'border-box' } as const
const helpStyle = { fontSize: 12, color: 'var(--muted)', marginTop: 4, lineHeight: 1.4 } as const

export function AiSettingsForm() {
  const { data: settings, isLoading } = useAiSettings()
  const update = useUpdateAiSettings()
  const test = useTestAiSettings()
  const [provider, setProvider] = useState('')
  const [apiKey, setApiKey] = useState('')
  const [model, setModel] = useState('')
  const [baseUrl, setBaseUrl] = useState('')
  const [clearKey, setClearKey] = useState(false)
  const [formError, setFormError] = useState<string | null>(null)
  const [testResult, setTestResult] = useState<{ ok: boolean; error?: string } | null>(null)

  if (isLoading) return <div style={{ padding: 16, color: 'var(--muted)' }}>Loading settings...</div>
  if (!settings) return null

  const form = { provider, apiKey, model, baseUrl, clearKey }
  const current = provider || settings.provider
  const compatible = current === OPENAI_COMPATIBLE
  const removesKey = keyWillBeRemoved(form, settings, settings.api_key_set)

  const handleSave = () => {
    setFormError(null)
    const result = buildAiSettingsBody(form, settings)
    if ('error' in result) {
      setFormError(result.error)
      return
    }
    update.mutate(result.body, {
      onSuccess: () => { setApiKey(''); setProvider(''); setModel(''); setBaseUrl(''); setClearKey(false) },
      onError: (e: Error) => setFormError(e.message),
    })
  }

  const handleTest = async () => {
    setTestResult(null)
    try {
      const result = await test.mutateAsync()
      setTestResult(result)
    } catch (e: any) {
      setTestResult({ ok: false, error: e.message })
    }
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12, padding: 16 }}>
      <div>
        <label htmlFor="ai-provider" style={labelStyle}>Provider</label>
        <select id="ai-provider" value={current} onChange={e => setProvider(e.target.value)} style={inputStyle}>
          {availableAiProviders(settings.hosted_ai_available, settings.provider).map(p => <option key={p.value} value={p.value}>{p.label}</option>)}
        </select>
      </div>

      {compatible && (
        <div>
          <label htmlFor="ai-base-url" style={labelStyle}>Base URL</label>
          <input id="ai-base-url" type="url" placeholder={settings.base_url || 'http://localhost:11434/v1'}
            value={baseUrl} onChange={e => setBaseUrl(e.target.value)} style={inputStyle} />
          <div style={helpStyle}>
            Any server with an OpenAI-style <code>/chat/completions</code> endpoint. For example:{' '}
            {BASE_URL_EXAMPLES.map((ex, i) => (
              <span key={ex.name}>{i > 0 && ', '}{ex.name} <code>{ex.url}</code></span>
            ))}.
            Ollama and LM Studio run on this network and need no key.
          </div>
        </div>
      )}

      <div>
        <label htmlFor="ai-model" style={labelStyle}>{compatible ? 'Model' : 'Default Model'}</label>
        <input id="ai-model" type="text"
          placeholder={settings.default_model || (compatible ? 'e.g. llama3.2, qwen2.5:7b, deepseek-chat' : 'e.g. gpt-4o, claude-sonnet-4-20250514')}
          value={model} onChange={e => setModel(e.target.value)} style={inputStyle} />
        {compatible && <div style={helpStyle}>The model name as the server knows it (for Ollama, as shown by <code>ollama list</code>).</div>}
      </div>

      <div>
        <label htmlFor="ai-key" style={labelStyle}>
          API Key{compatible && ' (optional)'} {settings.api_key_set && <span style={{ color: 'var(--success)' }}>(set: {settings.api_key_preview})</span>}
        </label>
        <input id="ai-key" type="password" placeholder={settings.api_key_set ? 'Enter new key to replace' : compatible ? 'Leave empty for local servers' : 'Enter API key'}
          value={apiKey} onChange={e => setApiKey(e.target.value)} style={inputStyle} />
        {settings.api_key_set && !apiKey && (
          <label style={{ ...helpStyle, display: 'flex', alignItems: 'center', gap: 6, cursor: 'pointer' }}>
            <input type="checkbox" checked={clearKey} onChange={e => setClearKey(e.target.checked)} />
            Remove the stored key
          </label>
        )}
        {removesKey && !clearKey && (
          <div style={helpStyle}>Saving with a different provider removes the stored key. Enter the key for the new provider if it needs one.</div>
        )}
      </div>

      {formError && (
        <div role="alert" style={{ padding: '8px 12px', borderRadius: 6, background: 'rgba(239,68,68,.1)', color: '#ef4444', fontSize: 13 }}>{formError}</div>
      )}

      <div style={{ display: 'flex', gap: 8, marginTop: 4 }}>
        <button onClick={handleSave} disabled={update.isPending}
          style={{ flex: 1, padding: '8px 16px', borderRadius: 6, border: 'none', background: 'var(--primary)', color: '#fff', fontWeight: 600, cursor: 'pointer', opacity: update.isPending ? 0.6 : 1 }}>
          {update.isPending ? 'Saving...' : 'Save Settings'}
        </button>
        <button onClick={handleTest} disabled={test.isPending} title="Sends a short request with the saved settings"
          style={{ padding: '8px 16px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)', cursor: 'pointer', opacity: test.isPending ? 0.6 : 1 }}>
          {test.isPending ? 'Testing...' : 'Test connection'}
        </button>
      </div>

      {testResult && (
        <div style={{ padding: '8px 12px', borderRadius: 6, background: testResult.ok ? 'rgba(34,197,94,.1)' : 'rgba(239,68,68,.1)', color: testResult.ok ? '#22c55e' : '#ef4444', fontSize: 13 }}>
          {testResult.ok ? 'Connection successful' : `Failed: ${testResult.error}`}
        </div>
      )}
    </div>
  )
}
