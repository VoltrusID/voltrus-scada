import { useState } from 'react'
import { useFeedTemplates, useFeedTemplate, useWizardCreate } from '../queries/wizard'
import type { FeedDetail, WizardResult } from '../queries/wizard'
import { QuickStartGuide } from '../QuickStartGuide'

const industries = [
  { id: 'Water/Wastewater', name: 'Water/Wastewater' },
  { id: 'Manufacturing', name: 'Manufacturing / Material Handling' },
  { id: 'Oil & Gas', name: 'Oil & Gas / Industrial' },
  { id: 'Chemical', name: 'Chemical / Pharmaceutical' },
  { id: 'Building Automation', name: 'Building Automation / HVAC' },
  { id: 'Energy', name: 'Energy / Power' },
]

const protocols = [
  { id: 'modbus', name: 'Modbus TCP', defaultPort: 502 },
  { id: 'opcua', name: 'OPC-UA', defaultPort: 4840 },
  { id: 's7', name: 'Siemens S7', defaultPort: 102 },
  { id: 'ethernet_ip', name: 'EtherNet/IP', defaultPort: 44818 },
  { id: 'dnp3', name: 'DNP3', defaultPort: 20000 },
  { id: 'bacnet', name: 'BACnet/IP', defaultPort: 47808 },
  { id: 'mqtt', name: 'MQTT', defaultPort: 1883 },
  { id: 'simulation', name: 'Simulation', defaultPort: 0 },
]

type Step = 'industry' | 'template' | 'device' | 'scan' | 'review' | 'result'

export function ProjectWizard({ onClose }: { onClose?: () => void } = {}) {
  const [mode, setMode] = useState<'quickstart' | 'wizard'>('quickstart')
  const [step, setStep] = useState<Step>('industry')

  // Step 1: Industry
  const [industry, setIndustry] = useState('')

  // Step 2: Template
  const [selectedTemplate, setSelectedTemplate] = useState<string | null>(null)
  const [templateSource, setTemplateSource] = useState<'vertical' | 'feed'>('vertical')

  // Step 3: Device
  const [protocol, setProtocol] = useState('modbus')
  const [address, setAddress] = useState('192.168.1.100')
  const [port, setPort] = useState(502)
  const [unitId, setUnitId] = useState(1)
  const [deviceId, setDeviceId] = useState('')
  const [instanceCount, setInstanceCount] = useState(4)
  const [autoScan, setAutoScan] = useState(true)
  const [genDashboard, setGenDashboard] = useState(true)
  const [genAlarms, setGenAlarms] = useState(true)

  // Step 4: Scan config
  const [regArea, setRegArea] = useState('holding')
  const [startAddr, setStartAddr] = useState(0)
  const [endAddr, setEndAddr] = useState(499)

  // Result
  const [result, setResult] = useState<WizardResult | null>(null)

  // Queries (only fetch when on template step)
  const feedTemplatesQ = useFeedTemplates()
  const feedDetailQ = useFeedTemplate(step === 'template' && templateSource === 'feed' ? selectedTemplate : null)

  // Mutation
  const createM = useWizardCreate()

  const templates = feedTemplatesQ.data?.templates || []
  const feedDetail = feedDetailQ.data ?? null

  const loadFeedDetail = (id: string) => {
    setSelectedTemplate(id)
  }

  const execute = () => {
    createM.mutate(
      {
        template_id: templateSource === 'vertical' ? selectedTemplate : null,
        template_source: templateSource,
        industry,
        device: {
          id: deviceId || null,
          protocol,
          address,
          port,
          unit_id: unitId,
          auto_scan: autoScan && protocol === 'modbus',
          scan: autoScan ? {
            register_area: regArea,
            start_address: startAddr,
            end_address: endAddr,
            scan_delay_ms: 10,
          } : null,
        },
        instance_counts: { n: instanceCount },
        generate_dashboard: genDashboard,
        generate_alarms: genAlarms,
      },
      {
        onSuccess: (data) => {
          setResult(data)
          setStep('result')
        },
      },
    )
  }

  const proto = protocols.find(p => p.id === protocol)!

  const btn = (label: string, primary?: boolean) => ({
    padding: '8px 20px', borderRadius: 6, border: 'none', cursor: 'pointer', fontSize: 13, fontWeight: 600,
    background: primary ? '#f97316' : 'var(--border)', color: primary ? '#fff' : 'var(--fg)',
  } as React.CSSProperties)

  const cardBase = { background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, padding: 20 } as React.CSSProperties
  const stepper = { width: 32, height: 32, borderRadius: '50%', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 14, fontWeight: 700 } as React.CSSProperties
  const stepperActive = { ...stepper, background: '#f97316', color: '#fff' }
  const stepperDone = { ...stepper, background: '#16a34a', color: '#fff' }
  const stepperNext = { ...stepper, background: 'var(--border)', color: 'var(--muted)' }

  const stepIndex = ['industry', 'template', 'device', 'scan', 'review', 'result'].indexOf(step)
  const stepLabels = ['Industry', 'Template', 'Device', 'Scan', 'Review', 'Done']

  const loading = feedTemplatesQ.isLoading || feedDetailQ.isLoading || createM.isPending
  const error = feedTemplatesQ.error?.message || feedDetailQ.error?.message || createM.error?.message

  const tabBtn = (label: string, active: boolean): React.CSSProperties => ({
    padding: '8px 16px',
    borderRadius: 6,
    border: 'none',
    cursor: 'pointer',
    fontSize: 13,
    fontWeight: active ? 600 : 400,
    background: active ? '#f97316' : 'var(--border)',
    color: active ? '#fff' : 'var(--fg)',
  })

  return (
    <div style={{ padding: 16, maxWidth: 900, margin: '0 auto' }}>
      {/* Mode Tabs */}
      <div style={{ display: 'flex', gap: 8, marginBottom: 20 }}>
        <button onClick={() => setMode('quickstart')} style={tabBtn('Quick Start', mode === 'quickstart')}>
          Quick Start
        </button>
        <button onClick={() => setMode('wizard')} style={tabBtn('Full Wizard', mode === 'wizard')}>
          Full Wizard
        </button>
      </div>

      {mode === 'quickstart' ? (
        <QuickStartGuide onComplete={onClose ?? (() => window.location.reload())} />
      ) : (
      <>
      <h2 style={{ fontSize: 18, fontWeight: 600, margin: '0 0 4px' }}>Project Creation Wizard</h2>
      <p style={{ color: 'var(--muted)', fontSize: 13, margin: '0 0 20px' }}>
        Set up a new project in minutes. Select your industry and equipment type, configure your devices, and Voltrus generates everything.
      </p>

      {/* Stepper */}
      {step !== 'result' && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 24 }}>
          {stepLabels.slice(0, 5).map((label, i) => (
            <div key={label} style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <div style={i < stepIndex ? stepperDone : i === stepIndex ? stepperActive : stepperNext}>
                {i < stepIndex ? '✓' : i + 1}
              </div>
              <span style={{ fontSize: 12, color: i === stepIndex ? 'var(--fg)' : 'var(--muted)' }}>{label}</span>
              {i < 4 && <div style={{ width: 24, height: 1, background: 'var(--border)' }} />}
            </div>
          ))}
        </div>
      )}

      {error && <div style={{ color: '#ef4444', fontSize: 13, marginBottom: 12 }}>{error}</div>}
      {loading && <div style={{ color: 'var(--muted)', fontSize: 13, marginBottom: 12 }}>Loading…</div>}

      {/* Step 1: Industry */}
      {step === 'industry' && (
        <>
          <div style={{ ...cardBase, marginBottom: 16 }}>
            <div style={{ fontSize: 14, fontWeight: 600, marginBottom: 12 }}>What industry are you in?</div>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(240px, 1fr))', gap: 10 }}>
              {industries.map(ind => (
                <div key={ind.id} onClick={() => { setIndustry(ind.id); setStep('template') }} style={{
                  ...cardBase, cursor: 'pointer', borderColor: industry === ind.id ? '#f97316' : undefined, padding: 16,
                }}>
                  <div style={{ fontSize: 14, fontWeight: 600 }}>{ind.name}</div>
                </div>
              ))}
            </div>
          </div>
        </>
      )}

      {/* Step 2: Template */}
      {step === 'template' && (
        <>
          <div style={{ display: 'flex', gap: 8, marginBottom: 16 }}>
            <button onClick={() => setTemplateSource('vertical')} style={btn('Vertical Templates (v0.37)', templateSource === 'vertical')}>
              Vertical Templates
            </button>
            <button onClick={() => setTemplateSource('feed')} style={btn('FEED Templates', templateSource === 'feed')}>
              FEED Templates
            </button>
            <button onClick={() => { setSelectedTemplate(null); setStep('device') }} style={btn('Skip — Manual Setup')}>
              Skip
            </button>
          </div>
          {templateSource === 'feed' && (
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))', gap: 12 }}>
              {templates.map(t => (
                <div key={t.id} onClick={() => { setSelectedTemplate(t.id); loadFeedDetail(t.id) }} style={{
                  ...cardBase, cursor: 'pointer', borderColor: selectedTemplate === t.id ? '#f97316' : undefined,
                }}>
                  <div style={{ fontSize: 14, fontWeight: 600 }}>{t.name}</div>
                  <div style={{ fontSize: 12, color: 'var(--muted)', marginTop: 4 }}>{t.description}</div>
                  <div style={{ display: 'flex', gap: 8, marginTop: 8, flexWrap: 'wrap' }}>
                    <span style={{ fontSize: 11, padding: '2px 8px', borderRadius: 4, background: 'rgba(249,115,22,.12)', color: '#f97316' }}>~{t.estimated_tag_count} tags</span>
                    {t.equipment_units.slice(0, 3).map(u => (
                      <span key={u} style={{ fontSize: 11, padding: '2px 8px', borderRadius: 4, background: 'rgba(59,130,246,.12)', color: '#3b82f6' }}>{u}</span>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          )}

          {templateSource === 'vertical' && (
            <div style={{ ...cardBase }}>
              <p style={{ fontSize: 13, color: 'var(--muted)' }}>
                Vertical templates from v0.37 will be used. These are pre-built with ISA-5.1 tags, alarms, and dashboards. The wizard will instantiate them with your device configuration.
              </p>
              <input value={selectedTemplate || ''} onChange={e => setSelectedTemplate(e.target.value)} placeholder="Template ID (e.g. wastewater-treatment)" style={{
                padding: '6px 10px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--card)', color: 'var(--fg)', fontSize: 13, width: 300, marginTop: 8,
              }} />
            </div>
          )}

          {feedDetail && (
            <div style={{ ...cardBase, marginTop: 16 }}>
              <div style={{ fontSize: 16, fontWeight: 600 }}>{feedDetail.name}</div>
              <div style={{ fontSize: 13, color: 'var(--muted)', marginTop: 4 }}>{feedDetail.description}</div>
              <div style={{ display: 'flex', gap: 16, marginTop: 12 }}>
                <div><strong>{feedDetail.io_list.length}</strong> I/O signals</div>
                <div><strong>{feedDetail.alarm_strategy.length}</strong> alarm strategies</div>
                <div><strong>{feedDetail.equipment_units.length}</strong> equipment units</div>
              </div>
              <div style={{ fontSize: 12, color: '#f97316', marginTop: 8, fontWeight: 600 }}>
                FEED templates are reference designs. Use the I/O list to guide manual configuration, or switch to a Vertical Template for one-click setup.
              </div>
            </div>
          )}

          <div style={{ marginTop: 16, display: 'flex', gap: 8 }}>
            <button onClick={() => setStep('industry')} style={btn('← Back')}>Back</button>
            <button onClick={() => setStep('device')} style={btn('Next: Configure Device →', true)}>Next</button>
          </div>
        </>
      )}

      {/* Step 3: Device */}
      {step === 'device' && (
        <>
          <div style={{ ...cardBase }}>
            <div style={{ fontSize: 14, fontWeight: 600, marginBottom: 16 }}>Device Configuration</div>
            <div style={{ display: 'grid', gridTemplateColumns: '140px 1fr', gap: '10px 16px', alignItems: 'center' }}>
              <span style={{ fontSize: 12, color: 'var(--muted)', textAlign: 'right' }}>Protocol</span>
              <select value={protocol} onChange={e => { setProtocol(e.target.value); const p = protocols.find(p => p.id === e.target.value); if (p) setPort(p.defaultPort) }} style={{ padding: '6px 10px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--card)', color: 'var(--fg)', fontSize: 13 }}>
                {protocols.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
              </select>
              <span style={{ fontSize: 12, color: 'var(--muted)', textAlign: 'right' }}>Address</span>
              <input value={address} onChange={e => setAddress(e.target.value)} style={{ padding: '6px 10px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--card)', color: 'var(--fg)', fontSize: 13 }} />
              <span style={{ fontSize: 12, color: 'var(--muted)', textAlign: 'right' }}>Port</span>
              <input type="number" value={port} onChange={e => setPort(parseInt(e.target.value) || 0)} style={{ padding: '6px 10px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--card)', color: 'var(--fg)', fontSize: 13, width: 120 }} />
              {protocol === 'modbus' && (<>
                <span style={{ fontSize: 12, color: 'var(--muted)', textAlign: 'right' }}>Unit ID</span>
                <input type="number" value={unitId} onChange={e => setUnitId(parseInt(e.target.value) || 1)} min={0} max={255} style={{ padding: '6px 10px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--card)', color: 'var(--fg)', fontSize: 13, width: 80 }} />
              </>)}
            </div>

            <div style={{ marginTop: 16, display: 'flex', gap: 12 }}>
              <label style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 13 }}>
                <input type="checkbox" checked={autoScan} onChange={e => setAutoScan(e.target.checked)} />
                Auto-scan registers (Modbus only)
              </label>
              <label style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 13 }}>
                <input type="checkbox" checked={genDashboard} onChange={e => setGenDashboard(e.target.checked)} />
                Generate dashboard
              </label>
              <label style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 13 }}>
                <input type="checkbox" checked={genAlarms} onChange={e => setGenAlarms(e.target.checked)} />
                Generate alarms
              </label>
            </div>
          </div>

          <div style={{ marginTop: 16, display: 'flex', gap: 8 }}>
            <button onClick={() => setStep('template')} style={btn('← Back')}>Back</button>
            {autoScan && protocol === 'modbus'
              ? <button onClick={() => setStep('scan')} style={btn('Next: Scan Settings →', true)}>Next</button>
              : <button onClick={() => setStep('review')} style={btn('Next: Review →', true)}>Next</button>
            }
          </div>
        </>
      )}

      {/* Step 4: Scan Config */}
      {step === 'scan' && (
        <>
          <div style={{ ...cardBase }}>
            <div style={{ fontSize: 14, fontWeight: 600, marginBottom: 16 }}>Auto-Scan Configuration</div>
            <div style={{ display: 'grid', gridTemplateColumns: '160px 1fr', gap: '10px 16px', alignItems: 'center' }}>
              <span style={{ fontSize: 12, color: 'var(--muted)', textAlign: 'right' }}>Register Area</span>
              <select value={regArea} onChange={e => setRegArea(e.target.value)} style={{ padding: '6px 10px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--card)', color: 'var(--fg)', fontSize: 13, width: 160 }}>
                <option value="holding">Holding Registers (3x/4x)</option>
                <option value="input">Input Registers (3x)</option>
                <option value="coil">Coils (0x)</option>
                <option value="discrete">Discrete Inputs (1x)</option>
              </select>
              <span style={{ fontSize: 12, color: 'var(--muted)', textAlign: 'right' }}>Start Address</span>
              <input type="number" value={startAddr} onChange={e => setStartAddr(parseInt(e.target.value) || 0)} min={0} style={{ padding: '6px 10px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--card)', color: 'var(--fg)', fontSize: 13, width: 100 }} />
              <span style={{ fontSize: 12, color: 'var(--muted)', textAlign: 'right' }}>End Address</span>
              <input type="number" value={endAddr} onChange={e => setEndAddr(parseInt(e.target.value) || 0)} min={0} style={{ padding: '6px 10px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--card)', color: 'var(--fg)', fontSize: 13, width: 100 }} />
            </div>
            <div style={{ fontSize: 12, color: 'var(--muted)', marginTop: 12 }}>
              Will scan {endAddr - startAddr + 1} registers on {address}:{port} (unit {unitId})
            </div>
          </div>
          <div style={{ marginTop: 16, display: 'flex', gap: 8 }}>
            <button onClick={() => setStep('device')} style={btn('← Back')}>Back</button>
            <button onClick={() => setStep('review')} style={btn('Next: Review →', true)}>Next</button>
          </div>
        </>
      )}

      {/* Step 5: Review */}
      {step === 'review' && (
        <>
          <div style={{ ...cardBase }}>
            <div style={{ fontSize: 14, fontWeight: 600, marginBottom: 16 }}>Review Configuration</div>
            <table style={{ width: '100%', fontSize: 13, borderCollapse: 'collapse' }}>
              <tbody>
                {[
                  ['Industry', industry],
                  ['Template', selectedTemplate ? `${templateSource === 'feed' ? 'FEED: ' : ''}${selectedTemplate}` : 'None (manual setup)'],
                  ['Protocol', proto.name],
                  ['Address', `${address}:${port}`],
                  ...(protocol === 'modbus' ? [['Unit ID', String(unitId)]] as [string,string][] : []),
                  ['Auto-Scan', autoScan ? (protocol === 'modbus' ? `Yes (${regArea} ${startAddr}-${endAddr})` : 'Not available for this protocol') : 'No'],
                  ['Dashboard', genDashboard ? 'Yes' : 'No'],
                  ['Alarms', genAlarms ? 'Yes' : 'No'],
                  ['Equipment Instances', String(instanceCount)],
                ].map(([label, value]) => (
                  <tr key={label} style={{ borderBottom: '1px solid var(--border)' }}>
                    <td style={{ padding: '6px 12px', color: 'var(--muted)', width: 180 }}>{label}</td>
                    <td style={{ padding: '6px 12px', fontWeight: 500 }}>{value}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div style={{ marginTop: 16, display: 'flex', gap: 8 }}>
            <button onClick={() => autoScan ? setStep('scan') : setStep('device')} style={btn('← Back')}>Back</button>
            <button onClick={execute} disabled={createM.isPending} style={btn(createM.isPending ? 'Creating…' : 'Create Project', true)}>Create Project</button>
          </div>
        </>
      )}

      {/* Step 6: Result */}
      {step === 'result' && result && (
        <>
          <div style={{ ...cardBase, borderColor: result.success ? '#16a34a' : '#ef4444' }}>
            <div style={{ fontSize: 24, textAlign: 'center', marginBottom: 8 }}>
              {result.success ? '' : '!'}
            </div>
            <div style={{ fontSize: 16, fontWeight: 600, textAlign: 'center', color: result.success ? '#16a34a' : '#ef4444' }}>
              {result.success ? 'Project Created!' : 'Wizard Completed with Warnings'}
            </div>

            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(140px, 1fr))', gap: 12, marginTop: 20 }}>
              {[
                ['Tags Created', result.tags_created, '#f97316'],
                ['Alarms Created', result.alarms_created, '#3b82f6'],
                ['Flows Created', result.flows_created, '#8b5cf6'],
                ['Dashboards', result.screens_created, '#06b6d4'],
                ['Tags Discovered', result.tags_discovered, '#22c55e'],
              ].map(([label, value, color]) => (
                <div key={label as string} style={{ background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 8, padding: 12, textAlign: 'center' }}>
                  <div style={{ fontSize: 24, fontWeight: 700, color: color as string }}>{value as number}</div>
                  <div style={{ fontSize: 11, color: 'var(--muted)', marginTop: 4 }}>{label as string}</div>
                </div>
              ))}
            </div>

            <div style={{ ...cardBase, marginTop: 16, background: 'var(--bg)' }}>
              <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 8 }}>Device: <code>{result.device_id}</code></div>
            </div>

            {result.warnings.length > 0 && (
              <div style={{ marginTop: 16 }}>
                <div style={{ fontSize: 13, fontWeight: 600, color: '#f59e0b', marginBottom: 8 }}>Warnings</div>
                {result.warnings.map((w, i) => (
                  <div key={i} style={{ fontSize: 12, color: 'var(--muted)', padding: '4px 0' }}>• {w}</div>
                ))}
              </div>
            )}

            {result.next_steps.length > 0 && (
              <div style={{ marginTop: 16 }}>
                <div style={{ fontSize: 13, fontWeight: 600, color: '#3b82f6', marginBottom: 8 }}>Next Steps</div>
                {result.next_steps.map((s, i) => (
                  <div key={i} style={{ fontSize: 12, color: 'var(--muted)', padding: '4px 0' }}>→ {s}</div>
                ))}
              </div>
            )}
          </div>

          <div style={{ marginTop: 16, display: 'flex', gap: 8 }}>
            <button onClick={() => { setStep('industry'); setResult(null); setSelectedTemplate(null) }} style={btn('Create Another')}>Create Another</button>
            <button onClick={() => window.location.reload()} style={btn('Go to Dashboard →', true)}>Go to Dashboard</button>
          </div>
        </>
      )}
      </>
      )}
    </div>
  )
}
