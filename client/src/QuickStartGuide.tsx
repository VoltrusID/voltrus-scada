import { useState } from 'react'
import { useWizardCreate } from './queries/wizard'
import type { WizardResult } from './queries/wizard'

type QuickStartStep = 1 | 2 | 3

interface QuickStartGuideProps {
  onComplete: () => void
}

export function QuickStartGuide({ onComplete }: QuickStartGuideProps) {
  const [step, setStep] = useState<QuickStartStep>(1)
  const [ip, setIp] = useState('192.168.1.100')
  const [port, setPort] = useState(502)
  const [unitId, setUnitId] = useState(1)
  const [result, setResult] = useState<WizardResult | null>(null)

  const createM = useWizardCreate()

  const loading = createM.isPending
  const error = createM.error?.message

  const isValidIp = /^\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(ip)
      && ip.split('.').every((octet) => { const n = parseInt(octet, 10); return n >= 0 && n <= 255 })

  const handleConnect = () => {
    createM.mutate(
      {
        template_id: null,
        template_source: 'vertical',
        industry: '',
        device: {
          id: null,
          protocol: 'modbus',
          address: ip,
          port,
          unit_id: unitId,
          auto_scan: true,
          scan: {
            register_area: 'holding',
            start_address: 0,
            end_address: 499,
            scan_delay_ms: 10,
          },
        },
        instance_counts: { n: 4 },
        generate_dashboard: true,
        generate_alarms: true,
      },
      {
        onSuccess: (data) => {
          setResult(data)
          setStep(2)
        },
      },
    )
  }

  const inputStyle: React.CSSProperties = {
    borderRadius: 6,
    border: '1px solid var(--border)',
    background: 'var(--bg)',
    color: 'var(--fg)',
    padding: '8px 12px',
    fontSize: 13,
    width: '100%',
    boxSizing: 'border-box',
  }

  const cardStyle: React.CSSProperties = {
    background: 'var(--card)',
    border: '1px solid var(--border)',
    borderRadius: 10,
    padding: 16,
  }

  const primaryBtn: React.CSSProperties = {
    background: '#f97316',
    color: '#fff',
    borderRadius: 6,
    border: 'none',
    fontWeight: 600,
    padding: '10px 20px',
    cursor: 'pointer',
    fontSize: 14,
  }

  const successBtn: React.CSSProperties = {
    background: '#16a34a',
    color: '#fff',
    borderRadius: 6,
    border: 'none',
    fontWeight: 600,
    padding: '10px 20px',
    cursor: 'pointer',
    fontSize: 14,
  }

  const labelStyle: React.CSSProperties = {
    fontSize: 12,
    color: 'var(--muted)',
    marginBottom: 4,
    display: 'block',
  }

  // Progress bar
  const progressBar = (
    <div style={{ display: 'flex', gap: 4, marginBottom: 24 }}>
      {[1, 2, 3].map((s) => (
        <div
          key={s}
          style={{
            flex: 1,
            height: 6,
            borderRadius: 3,
            background: s <= step ? '#f97316' : 'var(--border)',
            transition: 'background 0.2s',
          }}
        />
      ))}
    </div>
  )

  // Step 1: Connect
  if (step === 1) {
    return (
      <div style={{ padding: 16, maxWidth: 480, margin: '0 auto' }}>
        <h2 style={{ fontSize: 18, fontWeight: 600, margin: '0 0 4px' }}>Quick Start</h2>
        <p style={{ color: 'var(--muted)', fontSize: 13, margin: '0 0 20px' }}>
          Connect to your Modbus PLC in under a minute. Voltrus will auto-discover tags and build a dashboard for you.
        </p>

        {progressBar}

        <div style={{ ...cardStyle, marginBottom: 16 }}>
          <div style={{ fontSize: 14, fontWeight: 600, marginBottom: 16 }}>Connect to your PLC</div>

          <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
            <div>
              <label style={labelStyle}>IP Address</label>
              <input
                value={ip}
                onChange={(e) => setIp(e.target.value)}
                placeholder="192.168.1.100"
                style={inputStyle}
              />
            </div>

            <div style={{ display: 'flex', gap: 12 }}>
              <div style={{ flex: 1 }}>
                <label style={labelStyle}>Port</label>
                <input
                  type="number"
                  value={port}
                  onChange={(e) => setPort(parseInt(e.target.value) || 502)}
                  style={inputStyle}
                />
              </div>
              <div style={{ flex: 1 }}>
                <label style={labelStyle}>Unit ID</label>
                <input
                  type="number"
                  value={unitId}
                  onChange={(e) => setUnitId(parseInt(e.target.value) || 1)}
                  min={0}
                  max={255}
                  style={inputStyle}
                />
              </div>
            </div>
          </div>
        </div>

        {error && (
          <div style={{ color: '#ef4444', fontSize: 13, marginBottom: 12 }}>{error}</div>
        )}

        <button
          onClick={handleConnect}
          disabled={loading || !isValidIp}
          style={{
            ...primaryBtn,
            width: '100%',
            opacity: loading || !isValidIp ? 0.6 : 1,
            cursor: loading ? 'wait' : isValidIp ? 'pointer' : 'not-allowed',
          }}
        >
          {loading ? 'Connecting...' : 'Connect & Auto-Discover'}
        </button>
      </div>
    )
  }

  // Step 2: Discover
  if (step === 2 && result) {
    return (
      <div style={{ padding: 16, maxWidth: 480, margin: '0 auto' }}>
        <h2 style={{ fontSize: 18, fontWeight: 600, margin: '0 0 4px' }}>Quick Start</h2>
        <p style={{ color: 'var(--muted)', fontSize: 13, margin: '0 0 20px' }}>
          {result.success ? 'Discovery complete!' : 'Discovery completed with warnings.'}
        </p>

        {progressBar}

        <div style={{ ...cardStyle, marginBottom: 16, borderColor: result.success ? '#16a34a' : '#f59e0b' }}>
          <div style={{ fontSize: 14, fontWeight: 600, marginBottom: 16 }}>Discovery Results</div>

          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12, marginBottom: 16 }}>
            <div style={{ background: 'var(--bg)', borderRadius: 8, padding: 12, textAlign: 'center', border: '1px solid var(--border)' }}>
              <div style={{ fontSize: 28, fontWeight: 700, color: '#22c55e' }}>{result.tags_discovered}</div>
              <div style={{ fontSize: 12, color: 'var(--muted)', marginTop: 4 }}>Tags Discovered</div>
            </div>
            <div style={{ background: 'var(--bg)', borderRadius: 8, padding: 12, textAlign: 'center', border: '1px solid var(--border)' }}>
              <div style={{ fontSize: 28, fontWeight: 700, color: '#f97316' }}>{result.tags_created}</div>
              <div style={{ fontSize: 12, color: 'var(--muted)', marginTop: 4 }}>Tags Created</div>
            </div>
          </div>

          {result.alarms_created > 0 && (
            <div style={{ fontSize: 13, color: 'var(--muted)', marginBottom: 4 }}>
              <strong>{result.alarms_created}</strong> alarms generated
            </div>
          )}
          {result.screens_created > 0 && (
            <div style={{ fontSize: 13, color: 'var(--muted)', marginBottom: 4 }}>
              <strong>{result.screens_created}</strong> dashboard screen{result.screens_created !== 1 ? 's' : ''} created
            </div>
          )}
          {result.flows_created > 0 && (
            <div style={{ fontSize: 13, color: 'var(--muted)', marginBottom: 4 }}>
              <strong>{result.flows_created}</strong> flow{result.flows_created !== 1 ? 's' : ''} configured
            </div>
          )}

          {result.warnings.length > 0 && (
            <div style={{ marginTop: 12 }}>
              <div style={{ fontSize: 13, fontWeight: 600, color: '#f59e0b', marginBottom: 6 }}>Warnings</div>
              {result.warnings.map((w, i) => (
                <div key={i} style={{ fontSize: 12, color: 'var(--muted)', padding: '2px 0' }}>
                  {'\u2022'} {w}
                </div>
              ))}
            </div>
          )}
        </div>

        <button
          onClick={() => setStep(3)}
          style={{ ...successBtn, width: '100%' }}
        >
          Go to Dashboard
        </button>
      </div>
    )
  }

  // Step 3: Done
  return (
    <div style={{ padding: 16, maxWidth: 480, margin: '0 auto' }}>
      <h2 style={{ fontSize: 18, fontWeight: 600, margin: '0 0 4px' }}>Quick Start</h2>
      <p style={{ color: 'var(--muted)', fontSize: 13, margin: '0 0 20px' }}>
        You're all set!
      </p>

      {progressBar}

      <div style={{ ...cardStyle, textAlign: 'center', padding: '32px 16' }}>
        <div style={{ fontSize: 48, color: '#16a34a', marginBottom: 12 }}>&#10003;</div>
        <div style={{ fontSize: 20, fontWeight: 700, marginBottom: 8, color: '#16a34a' }}>
          You're Up and Running!
        </div>
        <div style={{ fontSize: 13, color: 'var(--muted)' }}>
          Your PLC is connected, tags are discovered, and your dashboard is ready.
        </div>
      </div>

      <button
        onClick={onComplete}
        style={{ ...successBtn, width: '100%', marginTop: 16 }}
      >
        Done
      </button>
    </div>
  )
}
