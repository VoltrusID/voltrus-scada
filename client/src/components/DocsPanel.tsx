import { useState } from 'react'
import { useDocsTopology, useDocsTagXref, useDocsAutomation } from '../queries/docs'
import type { TopologyData, TagXrefEntry, AutomationData } from '../queries/docs'

const protoColors: Record<string, string> = {
  'Modbus TCP': '#f97316', 'OPC-UA': '#3b82f6', 'Siemens S7': '#8b5cf6',
  'EtherNet/IP': '#06b6d4', 'DNP3': '#22c55e', 'BACnet/IP': '#ec4899',
  'MQTT': '#eab308', 'Simulation': '#6b7280',
}

// ── Main component ──

export function DocsPanel() {
  const [tab, setTab] = useState<'topology' | 'tags' | 'automation'>('topology')

  const topologyQ = useDocsTopology()
  const tagsQ = useDocsTagXref()
  const automationQ = useDocsAutomation()

  const loading = tab === 'topology' ? topologyQ.isLoading
    : tab === 'tags' ? tagsQ.isLoading
    : automationQ.isLoading

  const error = tab === 'topology' ? topologyQ.error?.message
    : tab === 'tags' ? tagsQ.error?.message
    : automationQ.error?.message

  return (
    <div style={{ padding: 16 }}>
      <h2 style={{ fontSize: 18, fontWeight: 600, margin: '0 0 4px' }}>System Documentation</h2>
      <p style={{ color: 'var(--muted)', fontSize: 13, margin: '0 0 16px' }}>Auto-generated documentation for handover, audits, and compliance.</p>

      <div style={{ display: 'flex', gap: 0, borderBottom: '1px solid var(--border)', marginBottom: 16 }}>
        {(['topology', 'tags', 'automation'] as const).map(t => (
          <div key={t} onClick={() => setTab(t)} style={{
            padding: '8px 16px', cursor: 'pointer', fontSize: 13,
            color: tab === t ? 'var(--fg)' : 'var(--muted)',
            borderBottom: tab === t ? '2px solid #f97316' : '2px solid transparent',
          }}>
            {t === 'topology' ? 'Topology' : t === 'tags' ? 'Tag Xref' : 'Automation'}
          </div>
        ))}
        <a href="/api/v1/docs/export?format=html" target="_blank" style={{
          padding: '8px 16px', fontSize: 13, color: '#f97316', cursor: 'pointer', textDecoration: 'none', fontWeight: 600,
        }}>Export HTML ↗</a>
      </div>

      {error && <div style={{ color: '#ef4444', fontSize: 13, marginBottom: 12 }}>{error}</div>}
      {loading && <div style={{ color: 'var(--muted)', fontSize: 13 }}>Loading…</div>}

      {!loading && tab === 'topology' && topologyQ.data && <TopologyTab data={topologyQ.data} />}
      {!loading && tab === 'tags' && tagsQ.data && <TagsTab tags={tagsQ.data.tags || []} total={tagsQ.data.total_tags || 0} />}
      {!loading && tab === 'automation' && automationQ.data && <AutomationTab data={automationQ.data} />}
    </div>
  )
}

// ── Topology Tab ──

function TopologyTab({ data }: { data: TopologyData }) {
  const s = data.summary
  return (
    <>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(160px, 1fr))', gap: 12, marginBottom: 16 }}>
        {[
          { n: s.total_devices, l: 'Devices', c: '#f97316' },
          { n: s.total_tags, l: 'Tags', c: '#f97316' },
          { n: s.online_count, l: 'Online', c: '#16a34a' },
          { n: s.offline_count, l: 'Offline', c: '#ef4444' },
        ].map(x => (
          <div key={x.l} style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, padding: 16, textAlign: 'center' }}>
            <div style={{ fontSize: 28, fontWeight: 700, color: x.c }}>{x.n}</div>
            <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', marginTop: 4 }}>{x.l}</div>
          </div>
        ))}
      </div>

      <h3 style={{ fontSize: 15, fontWeight: 600, margin: '16px 0 8px' }}>Protocols</h3>
      {(s.protocols || []).map(p => (
        <div key={p.protocol} style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '6px 0', borderBottom: '1px solid var(--border)' }}>
          <div style={{ width: 10, height: 10, borderRadius: '50%', background: protoColors[p.protocol] || '#94a3b8' }} />
          <strong>{p.protocol}</strong>
          <span style={{ color: 'var(--muted)' }}>{p.device_count} devices, {p.tag_count} tags</span>
        </div>
      ))}

      <h3 style={{ fontSize: 15, fontWeight: 600, margin: '16px 0 8px' }}>Devices</h3>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(260px, 1fr))', gap: 12 }}>
        {data.devices.map(d => (
          <div key={d.id} style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 8, padding: 12 }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
              <span style={{ fontSize: 14, fontWeight: 600 }}>{d.name}</span>
              <span style={{ fontSize: 11, fontWeight: 600, padding: '2px 8px', borderRadius: 4,
                background: d.online ? 'rgba(22,163,74,.12)' : 'rgba(239,68,68,.12)',
                color: d.online ? '#16a34a' : '#ef4444',
              }}>{d.online ? 'Online' : 'Offline'}</span>
            </div>
            <div style={{ fontSize: 12, color: 'var(--muted)', marginTop: 4, display: 'flex', alignItems: 'center', gap: 4 }}>
              <div style={{ width: 8, height: 8, borderRadius: '50%', background: protoColors[d.protocol] || '#94a3b8' }} />
              {d.protocol}{d.address ? ` · ${d.address}` : ''}
            </div>
            <div style={{ fontSize: 12, color: 'var(--muted)' }}>{d.tag_count} tags{!d.enabled ? ' · Disabled' : ''}</div>
          </div>
        ))}
      </div>
    </>
  )
}

// ── Tags Tab ──

function TagsTab({ tags, total }: { tags: TagXrefEntry[]; total: number }) {
  return (
    <>
      <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, padding: 16, marginBottom: 12 }}>
        <strong>{total}</strong> tags across all devices
      </div>
      <div style={{ overflowX: 'auto' }}>
        <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 12 }}>
          <thead>
            <tr>
              {['Key', 'Name', 'Device', 'Protocol', 'Unit', 'Alarm', 'Warn', 'Crit'].map(h => (
                <th key={h} style={{ textAlign: 'left', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', padding: '6px 8px', borderBottom: '1px solid var(--border)' }}>{h}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {tags.slice(0, 200).map(t => (
              <tr key={t.key} style={{ borderBottom: '1px solid var(--border)' }}>
                <td><code style={{ background: 'var(--border)', padding: '1px 4px', borderRadius: 3, fontSize: 11 }}>{t.key}</code></td>
                <td>{t.name}</td>
                <td>{t.device_name}</td>
                <td>{t.protocol}</td>
                <td>{t.unit}</td>
                <td>{t.has_alarm
                  ? <span style={{ fontSize: 11, fontWeight: 600, padding: '2px 6px', borderRadius: 4, background: t.alarm_severity === 'critical' ? 'rgba(239,68,68,.12)' : 'rgba(245,158,11,.12)', color: t.alarm_severity === 'critical' ? '#ef4444' : '#f59e0b' }}>{t.alarm_severity || 'yes'}</span>
                  : '—'}</td>
                <td>{t.warn_val}</td>
                <td>{t.crit_val}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {tags.length > 200 && <div style={{ color: 'var(--muted)', fontSize: 13, padding: 20, textAlign: 'center' }}>Showing first 200 of {total} tags</div>}
      </div>
    </>
  )
}

// ── Automation Tab ──

function AutomationTab({ data }: { data: AutomationData }) {
  const s = data.summary
  return (
    <>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(160px, 1fr))', gap: 12, marginBottom: 16 }}>
        {[
          { n: s.total_scripts, l: 'Scripts', c: '#f97316' },
          { n: s.total_flows, l: 'Flow Rules', c: '#f97316' },
          { n: s.enabled_scripts + s.enabled_flows, l: 'Enabled', c: '#16a34a' },
        ].map(x => (
          <div key={x.l} style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, padding: 16, textAlign: 'center' }}>
            <div style={{ fontSize: 28, fontWeight: 700, color: x.c }}>{x.n}</div>
            <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', marginTop: 4 }}>{x.l}</div>
          </div>
        ))}
      </div>

      {data.flows.length > 0 && (
        <>
          <h3 style={{ fontSize: 15, fontWeight: 600, margin: '16px 0 8px' }}>Flow Rules</h3>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 12 }}>
            <thead>
              <tr>{['Name', 'Trigger', 'Actions', 'Status'].map(h => (
                <th key={h} style={{ textAlign: 'left', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', padding: '6px 8px', borderBottom: '1px solid var(--border)' }}>{h}</th>
              ))}</tr>
            </thead>
            <tbody>
              {data.flows.map(f => (
                <tr key={f.id} style={{ borderBottom: '1px solid var(--border)' }}>
                  <td><strong>{f.name}</strong></td>
                  <td>{f.trigger_description}</td>
                  <td style={{ maxWidth: 300 }}>{f.node_descriptions.join('; ') || '—'}</td>
                  <td><span style={{ fontSize: 11, fontWeight: 600, padding: '2px 8px', borderRadius: 4,
                    background: f.enabled ? 'rgba(22,163,74,.12)' : 'rgba(239,68,68,.12)',
                    color: f.enabled ? '#16a34a' : '#ef4444',
                  }}>{f.enabled ? 'ON' : 'OFF'}</span></td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}

      {data.scripts.length > 0 && (
        <>
          <h3 style={{ fontSize: 15, fontWeight: 600, margin: '16px 0 8px' }}>Scripts</h3>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 12 }}>
            <thead>
              <tr>{['Name', 'Description', 'Timeout', 'Status'].map(h => (
                <th key={h} style={{ textAlign: 'left', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', padding: '6px 8px', borderBottom: '1px solid var(--border)' }}>{h}</th>
              ))}</tr>
            </thead>
            <tbody>
              {data.scripts.map(sc => (
                <tr key={sc.id} style={{ borderBottom: '1px solid var(--border)' }}>
                  <td><strong>{sc.name}</strong></td>
                  <td style={{ maxWidth: 400 }}>{sc.description}</td>
                  <td>{sc.cpu_limit_ms}ms</td>
                  <td>{sc.syntax_error
                    ? <span style={{ fontSize: 11, fontWeight: 600, padding: '2px 8px', borderRadius: 4, background: 'rgba(239,68,68,.12)', color: '#ef4444' }}>Error</span>
                    : <span style={{ fontSize: 11, fontWeight: 600, padding: '2px 8px', borderRadius: 4,
                        background: sc.enabled ? 'rgba(22,163,74,.12)' : 'rgba(239,68,68,.12)',
                        color: sc.enabled ? '#16a34a' : '#ef4444',
                      }}>{sc.enabled ? 'ON' : 'OFF'}</span>
                  }</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </>
  )
}
