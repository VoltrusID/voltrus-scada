import { useState, useEffect } from 'react'
import { useApiKeys, useCreateApiKey, useRevokeApiKey } from './queries'
import { inputStyle, btnStyle } from './styles'
import { fmtTs } from './styles'

interface ApiKeyItem {
  id: number
  name: string
  key_prefix: string
  created_at: number
  created_by: string
  last_used_at: number | null
  expires_at: number | null
  permissions: string
}

interface CreateApiKeyResponse {
  id: number
  name: string
  key: string
  key_prefix: string
  created_at: number
  warning: string
}

export function ApiKeyManager({ onClose, embedded }: { onClose: () => void; embedded?: boolean }) {
  const { data: keys = [], isLoading } = useApiKeys()
  const createMut = useCreateApiKey()
  const revokeMut = useRevokeApiKey()

  const [showAdd, setShowAdd] = useState(false)
  const [confirmRevoke, setConfirmRevoke] = useState<ApiKeyItem | null>(null)
  const [createdKey, setCreatedKey] = useState<CreateApiKeyResponse | null>(null)
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    if (embedded) return
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, embedded])

  const thStyle: React.CSSProperties = { textAlign: 'left', padding: '8px 12px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600, borderBottom: '1px solid var(--border)' }
  const tdStyle: React.CSSProperties = { padding: '8px 12px', fontSize: 13, borderBottom: '1px solid var(--border)' }

  const handleCreate = (data: { name: string; expires_at?: number; permissions?: string }) => {
    createMut.mutate(data, {
      onSuccess: (result) => {
        setShowAdd(false)
        setCreatedKey(result)
      },
    })
  }

  const handleRevoke = (id: number) => {
    revokeMut.mutate(id, { onSuccess: () => setConfirmRevoke(null) })
  }

  const handleCopy = async (key: string) => {
    await navigator.clipboard.writeText(key)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  const tableContent = (
    <>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16, flexShrink: 0 }}>
        <span style={{ color: 'var(--muted)', fontSize: 13 }}>{keys.length} API key{keys.length !== 1 ? 's' : ''}</span>
        <button style={{ ...btnStyle, borderColor: 'var(--cyan)', color: 'var(--cyan)' }} onClick={() => setShowAdd(true)}>+ Create API Key</button>
      </div>

      {createdKey && (
        <div style={{ background: 'rgba(34,197,94,.08)', border: '1px solid var(--cyan)', borderRadius: 10, padding: 20, marginBottom: 16, flexShrink: 0 }}>
          <h3 style={{ fontSize: 15, fontWeight: 700, marginBottom: 12, color: 'var(--ok)' }}>API Key Created</h3>
          <p style={{ fontSize: 12, color: 'var(--warn)', marginBottom: 12, fontWeight: 600 }}>
            This key will not be shown again. Copy it now.
          </p>
          <div style={{ background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 6, padding: 12, fontFamily: 'var(--font-mono)', fontSize: 13, wordBreak: 'break-all', marginBottom: 12, color: 'var(--fg)' }}>
            {createdKey.key}
          </div>
          {createdKey.warning && (
            <p style={{ fontSize: 11, color: 'var(--muted)', marginBottom: 12 }}>{createdKey.warning}</p>
          )}
          <div style={{ display: 'flex', gap: 8 }}>
            <button
              style={{ ...btnStyle, borderColor: 'var(--cyan)', color: 'var(--cyan)' }}
              onClick={() => handleCopy(createdKey.key)}
            >
              {copied ? 'Copied!' : 'Copy Key'}
            </button>
            <button style={btnStyle} onClick={() => setCreatedKey(null)}>Done</button>
          </div>
        </div>
      )}

      <div style={{ flex: 1, overflow: 'auto', background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10 }}>
        {isLoading ? (
          <div style={{ color: 'var(--muted)', textAlign: 'center', padding: 40 }}>Loading...</div>
        ) : keys.length === 0 ? (
          <div style={{ color: 'var(--muted)', textAlign: 'center', padding: 40 }}>No API keys found.</div>
        ) : (
        <table style={{ width: '100%', borderCollapse: 'collapse' }}>
          <thead>
            <tr>
              <th style={thStyle}>Name</th>
              <th style={thStyle}>Key</th>
              <th style={thStyle}>Created</th>
              <th style={thStyle}>Last Used</th>
              <th style={thStyle}>Expires</th>
              <th style={{ ...thStyle, width: 100 }}>Actions</th>
            </tr>
          </thead>
          <tbody>
            {keys.map(k => (
              <tr key={k.id}>
                <td style={tdStyle}>
                  <span style={{ fontWeight: 600 }}>{k.name}</span>
                </td>
                <td style={{ ...tdStyle, fontFamily: 'var(--font-mono)', fontSize: 12, color: 'var(--muted)' }}>
                  {k.key_prefix}...
                </td>
                <td style={{ ...tdStyle, color: 'var(--muted)', fontSize: 12 }}>{fmtTs(k.created_at)}</td>
                <td style={{ ...tdStyle, color: 'var(--muted)', fontSize: 12 }}>{k.last_used_at ? fmtTs(k.last_used_at) : '-'}</td>
                <td style={{ ...tdStyle, color: k.expires_at && k.expires_at < Date.now() ? 'var(--crit)' : 'var(--muted)', fontSize: 12 }}>
                  {k.expires_at ? fmtTs(k.expires_at) : 'Never'}
                </td>
                <td style={tdStyle}>
                  <button
                    style={{ ...btnStyle, fontSize: 11, padding: '4px 10px', borderColor: 'var(--crit)', color: 'var(--crit)' }}
                    onClick={() => setConfirmRevoke(k)}
                  >
                    Revoke
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        )}
      </div>

      {showAdd && (
        <CreateApiKeyDialog
          onSave={handleCreate}
          onClose={() => setShowAdd(false)}
          loading={createMut.isPending}
          error={createMut.error?.message}
        />
      )}

      {confirmRevoke && (
        <div style={{ position: 'fixed', inset: 0, zIndex: 60, background: 'rgba(0,0,0,.6)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 12, padding: 24, maxWidth: 400, width: '90%' }}>
            <h3 style={{ fontSize: 16, fontWeight: 700, marginBottom: 8 }}>Revoke API Key</h3>
            <p style={{ fontSize: 13, color: 'var(--muted)', marginBottom: 16 }}>
              Revoke key <strong style={{ color: 'var(--fg)' }}>{confirmRevoke.name}</strong>? This action cannot be undone. Any services using this key will lose access.
            </p>
            {revokeMut.error && <p style={{ fontSize: 12, color: 'var(--crit)', marginBottom: 8 }}>{revokeMut.error.message}</p>}
            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
              <button style={btnStyle} onClick={() => setConfirmRevoke(null)}>Cancel</button>
              <button style={{ ...btnStyle, borderColor: 'var(--crit)', color: 'var(--crit)' }} onClick={() => handleRevoke(confirmRevoke.id)} disabled={revokeMut.isPending}>Revoke</button>
            </div>
          </div>
        </div>
      )}
    </>
  )

  if (embedded) return <div style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}>{tableContent}</div>

  return (
    <div style={{ position: 'fixed', inset: 0, zIndex: 50, background: 'var(--bg)', display: 'flex', flexDirection: 'column', fontFamily: 'var(--font-ui)' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '16px 24px', borderBottom: '1px solid var(--border)', flexShrink: 0 }}>
        <button onClick={onClose} style={{ ...btnStyle, padding: '4px 8px' }}>&larr; Back</button>
        <h2 style={{ fontSize: 18, fontWeight: 700 }}>API Key Management</h2>
      </div>
      <div style={{ flex: 1, overflow: 'auto', padding: '16px 24px', display: 'flex', flexDirection: 'column' }}>
        {tableContent}
      </div>
    </div>
  )
}

function CreateApiKeyDialog({ onSave, onClose, loading, error }: {
  onSave: (d: { name: string; expires_at?: number; permissions?: string }) => void
  onClose: () => void
  loading?: boolean
  error?: string
}) {
  const [name, setName] = useState('')
  const [expires, setExpires] = useState('')
  const [permissions, setPermissions] = useState('["data:read"]')

  const handleSubmit = () => {
    const data: { name: string; expires_at?: number; permissions?: string } = { name }
    if (expires) {
      const ts = new Date(expires).getTime()
      if (!isNaN(ts)) data.expires_at = ts
    }
    data.permissions = permissions
    onSave(data)
  }

  return (
    <div style={{ position: 'fixed', inset: 0, zIndex: 60, background: 'rgba(0,0,0,.6)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 12, padding: 24, maxWidth: 400, width: '90%' }}>
        <h3 style={{ fontSize: 16, fontWeight: 700, marginBottom: 16 }}>Create API Key</h3>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <span style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Name (required)</span>
            <input placeholder="e.g. Monitoring Script" value={name} onChange={e => setName(e.target.value)} style={inputStyle} autoFocus />
          </label>
          <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <span style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Expiry (optional)</span>
            <input type="date" value={expires} onChange={e => setExpires(e.target.value)} style={inputStyle} />
          </label>
          <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <span style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Permissions (JSON array)</span>
            <input placeholder='["data:read"]' value={permissions} onChange={e => setPermissions(e.target.value)} style={{ ...inputStyle, fontFamily: 'var(--font-mono)', fontSize: 12 }} />
          </label>
          {error && <p style={{ fontSize: 12, color: 'var(--crit)' }}>{error}</p>}
          <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
            <button style={btnStyle} onClick={onClose} disabled={loading}>Cancel</button>
            <button style={{ ...btnStyle, borderColor: 'var(--cyan)', color: 'var(--cyan)' }} onClick={handleSubmit} disabled={!name || loading}>
              {loading ? 'Creating...' : 'Create'}
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
