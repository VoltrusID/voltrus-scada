import { useState } from 'react'
import { useAiSettings, useUpdateAiSettings, useTestAiSettings } from '../queries/copilot'

const PROVIDERS = [
  { value: 'byok_openai', label: 'OpenAI (BYOK)' },
  { value: 'byok_anthropic', label: 'Anthropic (BYOK)' },
  { value: 'voltrus_ai', label: 'Voltrus AI' },
]

export function AiSettingsForm() {
  const { data: settings, isLoading } = useAiSettings()
  const update = useUpdateAiSettings()
  const test = useTestAiSettings()
  const [provider, setProvider] = useState('')
  const [apiKey, setApiKey] = useState('')
  const [model, setModel] = useState('')
  const [testResult, setTestResult] = useState<{ ok: boolean; error?: string } | null>(null)

  if (isLoading) return <div style={{ padding: 16, color: 'var(--muted)' }}>Loading settings...</div>
  if (!settings) return null

  const handleSave = () => {
    const body: Record<string, string> = {}
    if (provider) body.provider = provider
    if (apiKey) body.api_key = apiKey
    if (model) body.default_model = model
    update.mutate(body, { onSuccess: () => { setApiKey(''); setProvider(''); setModel('') } })
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
        <label style={{ display: 'block', fontSize: 12, color: 'var(--muted)', marginBottom: 4 }}>Provider</label>
        <select value={provider || settings.provider} onChange={e => setProvider(e.target.value)}
          style={{ width: '100%', padding: '8px 12px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)', fontSize: 14 }}>
          {PROVIDERS.map(p => <option key={p.value} value={p.value}>{p.label}</option>)}
        </select>
      </div>

      <div>
        <label style={{ display: 'block', fontSize: 12, color: 'var(--muted)', marginBottom: 4 }}>
          API Key {settings.api_key_set && <span style={{ color: 'var(--success)' }}>(set: {settings.api_key_preview})</span>}
        </label>
        <input type="password" placeholder={settings.api_key_set ? 'Enter new key to replace' : 'Enter API key'}
          value={apiKey} onChange={e => setApiKey(e.target.value)}
          style={{ width: '100%', padding: '8px 12px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)', fontSize: 14, boxSizing: 'border-box' }} />
      </div>

      <div>
        <label style={{ display: 'block', fontSize: 12, color: 'var(--muted)', marginBottom: 4 }}>Default Model</label>
        <input type="text" placeholder={settings.default_model || 'e.g. gpt-4o, claude-sonnet-4-20250514'}
          value={model} onChange={e => setModel(e.target.value)}
          style={{ width: '100%', padding: '8px 12px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)', fontSize: 14, boxSizing: 'border-box' }} />
      </div>

      <div style={{ display: 'flex', gap: 8, marginTop: 4 }}>
        <button onClick={handleSave} disabled={update.isPending}
          style={{ flex: 1, padding: '8px 16px', borderRadius: 6, border: 'none', background: 'var(--primary)', color: '#fff', fontWeight: 600, cursor: 'pointer', opacity: update.isPending ? 0.6 : 1 }}>
          {update.isPending ? 'Saving...' : 'Save Settings'}
        </button>
        <button onClick={handleTest} disabled={test.isPending}
          style={{ padding: '8px 16px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)', cursor: 'pointer', opacity: test.isPending ? 0.6 : 1 }}>
          {test.isPending ? 'Testing...' : 'Test'}
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
