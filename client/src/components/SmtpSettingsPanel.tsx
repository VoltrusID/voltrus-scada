import { useState, useEffect } from 'react'
import { useSmtpConfig, useUpdateSmtpConfig, useTestSmtpConnection } from '../queries/smtp'
import { inputStyle, labelStyle, labelTextStyle, btnStyle } from '../styles'

const sectionTitle: React.CSSProperties = {
  fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase',
  letterSpacing: '.06em', fontWeight: 600, marginTop: 12,
  marginBottom: 4, paddingBottom: 4, borderBottom: '1px solid var(--border)',
}

const PASSWORD_MASK = '********'

export function SmtpSettingsPanel() {
  const { data, isLoading } = useSmtpConfig()
  const updateMutation = useUpdateSmtpConfig()
  const testMutation = useTestSmtpConnection()

  const [host, setHost] = useState('')
  const [port, setPort] = useState(587)
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [fromAddress, setFromAddress] = useState('')
  const [fromName, setFromName] = useState('')
  const [encryption, setEncryption] = useState<'starttls' | 'implicittls' | 'none'>('starttls')
  const [testRecipient, setTestRecipient] = useState('')

  useEffect(() => {
    if (data?.configured) {
      setHost(data.host ?? '')
      setPort(data.port ?? 587)
      setUsername(data.username ?? '')
      setPassword(data.password ?? PASSWORD_MASK)
      setFromAddress(data.from_address ?? '')
      setFromName(data.from_name ?? '')
      setEncryption(data.encryption ?? 'starttls')
    }
  }, [data])

  if (isLoading) return <div style={{ color: 'var(--muted)', padding: 20 }}>Loading SMTP configuration...</div>

  const configured = data?.configured === true

  const handleSave = () => {
    updateMutation.mutate({
      host,
      port,
      username,
      password: password || undefined,
      from_address: fromAddress || undefined,
      from_name: fromName || undefined,
      encryption,
    })
  }

  const handleTest = () => {
    if (!testRecipient.trim()) return
    testMutation.mutate(testRecipient)
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8, maxWidth: 480 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
        <span style={{ fontSize: 11, fontWeight: 600, textTransform: 'uppercase', letterSpacing: '.06em', color: 'var(--muted)' }}>Status</span>
        <span style={{
          fontSize: 11, fontWeight: 600, padding: '2px 8px', borderRadius: 4,
          background: configured ? 'rgba(34,197,94,0.15)' : 'rgba(249,115,22,0.15)',
          color: configured ? 'var(--ok)' : 'var(--warn)',
        }}>
          {configured ? 'Configured' : 'Not Configured'}
        </span>
      </div>

      <div style={sectionTitle}>Server</div>
      <div style={{ display: 'grid', gridTemplateColumns: '2fr 1fr', gap: 8 }}>
        <label style={labelStyle}>
          <span style={labelTextStyle}>Host</span>
          <input style={inputStyle} placeholder="smtp.example.com" value={host} onChange={e => setHost(e.target.value)} />
        </label>
        <label style={labelStyle}>
          <span style={labelTextStyle}>Port</span>
          <input style={{ ...inputStyle, fontFamily: 'var(--font-mono)' }} type="number" value={port} onChange={e => setPort(Number(e.target.value))} />
        </label>
      </div>

      <div style={sectionTitle}>Authentication</div>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
        <label style={labelStyle}>
          <span style={labelTextStyle}>Username</span>
          <input style={inputStyle} placeholder="your@email.com" value={username} onChange={e => setUsername(e.target.value)} />
        </label>
        <label style={labelStyle}>
          <span style={labelTextStyle}>Password</span>
          <input style={inputStyle} type="password" placeholder="••••••••" value={password} onChange={e => setPassword(e.target.value)} />
          {configured && <span style={{ fontSize: 10, color: 'var(--muted)', marginTop: 2 }}>Leave unchanged to keep current password</span>}
        </label>
      </div>

      <div style={sectionTitle}>Sender</div>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
        <label style={labelStyle}>
          <span style={labelTextStyle}>From Address</span>
          <input style={inputStyle} placeholder="voltrus@example.com" value={fromAddress} onChange={e => setFromAddress(e.target.value)} />
        </label>
        <label style={labelStyle}>
          <span style={labelTextStyle}>From Name</span>
          <input style={inputStyle} placeholder="Voltrus" value={fromName} onChange={e => setFromName(e.target.value)} />
        </label>
      </div>

      <div style={sectionTitle}>Encryption</div>
      <div style={{ display: 'flex', gap: 4 }}>
        {(['starttls', 'implicittls', 'none'] as const).map(opt => (
          <button key={opt} onClick={() => setEncryption(opt)} style={{
            ...btnStyle, flex: 1, justifyContent: 'center',
            borderColor: encryption === opt ? 'var(--cyan)' : 'var(--border)',
            color: encryption === opt ? 'var(--cyan)' : 'var(--muted)',
          }}>
            {opt === 'starttls' ? 'STARTTLS' : opt === 'implicittls' ? 'Implicit TLS' : 'None'}
          </button>
        ))}
      </div>

      <div style={{ display: 'flex', gap: 8, marginTop: 16 }}>
        <button onClick={handleSave} disabled={updateMutation.isPending} style={{
          ...btnStyle, background: 'var(--cyan)', color: '#000', borderColor: 'var(--cyan)',
          fontWeight: 700, opacity: updateMutation.isPending ? 0.6 : 1,
        }}>
          {updateMutation.isPending ? 'Saving...' : 'Save Configuration'}
        </button>
      </div>

      {updateMutation.isSuccess && <span style={{ fontSize: 12, color: 'var(--ok)' }}>Configuration saved</span>}
      {updateMutation.isError && <span style={{ fontSize: 12, color: 'var(--crit)' }}>{updateMutation.error.message}</span>}

      <div style={sectionTitle}>Test Connection</div>
      <div style={{ display: 'flex', gap: 8 }}>
        <input style={{ ...inputStyle, flex: 1 }} placeholder="recipient@example.com" value={testRecipient} onChange={e => setTestRecipient(e.target.value)} />
        <button onClick={handleTest} disabled={testMutation.isPending || !testRecipient.trim()} style={{
          ...btnStyle, opacity: testMutation.isPending || !testRecipient.trim() ? 0.5 : 1,
        }}>
          {testMutation.isPending ? 'Sending...' : 'Send Test'}
        </button>
      </div>
      {testMutation.isSuccess && <span style={{ fontSize: 12, color: 'var(--ok)' }}>{testMutation.data?.message ?? 'Test email sent'}</span>}
      {testMutation.isError && <span style={{ fontSize: 12, color: 'var(--crit)' }}>{testMutation.error.message}</span>}
    </div>
  )
}
