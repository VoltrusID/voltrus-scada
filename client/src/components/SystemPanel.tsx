import { useState } from 'react'
import type { SystemInfo } from '../types'
import { DocsPanel } from './DocsPanel'
import { TemplatesPanel } from './TemplatesPanel'
import { ConfigSnapshotPanel } from './ConfigSnapshotPanel'
import { ProjectWizard } from './ProjectWizard'
import { HealthCheckPanel } from './HealthCheckPanel'
import { SmtpSettingsPanel } from './SmtpSettingsPanel'
import { StorageSettings } from './StorageSettings'

type SystemTab = 'overview' | 'docs' | 'templates' | 'config' | 'wizard' | 'health' | 'storage' | 'smtp' | 'tools'

interface Props {
  systemInfo: SystemInfo | null | undefined
}

export function SystemPanel({ systemInfo }: Props) {
  const [tab, setTab] = useState<SystemTab>('overview')

  const tabStyle = (active: boolean) => ({
    padding: '8px 16px', cursor: 'pointer', fontSize: 13,
    color: active ? 'var(--fg)' : 'var(--muted)',
    borderBottom: active ? '2px solid #f97316' : '2px solid transparent',
  } as React.CSSProperties)

  return (
    <div style={{ padding: 8, overflowY: 'auto', height: '100%', boxSizing: 'border-box' }}>
      <div style={{ display: 'flex', gap: 0, borderBottom: '1px solid var(--border)', marginBottom: 16 }}>
        <div style={tabStyle(tab === 'overview')} onClick={() => setTab('overview')}>System</div>
        <div style={tabStyle(tab === 'docs')} onClick={() => setTab('docs')}>Documentation</div>
        <div style={tabStyle(tab === 'templates')} onClick={() => setTab('templates')}>Templates</div>
        <div style={tabStyle(tab === 'wizard')} onClick={() => setTab('wizard')}>Wizard</div>
        <div style={tabStyle(tab === 'health')} onClick={() => setTab('health')}>Health</div>
        <div style={tabStyle(tab === 'storage')} onClick={() => setTab('storage')}>Storage</div>
        <div style={tabStyle(tab === 'config')} onClick={() => setTab('config')}>Config</div>
        <div style={tabStyle(tab === 'smtp')} onClick={() => setTab('smtp')}>SMTP</div>
        <div style={tabStyle(tab === 'tools')} onClick={() => setTab('tools')}>Tools</div>
      </div>

      {tab === 'overview' && <SystemOverview systemInfo={systemInfo} />}
      {tab === 'docs' && <DocsPanel />}
      {tab === 'templates' && <TemplatesPanel />}
      {tab === 'wizard' && <ProjectWizard onClose={() => setTab('overview')} />}
      {tab === 'health' && <HealthCheckPanel />}
      {tab === 'storage' && <StorageSettings />}
      {tab === 'config' && <ConfigSnapshotPanel />}
      {tab === 'smtp' && <SmtpSettingsPanel />}
      {tab === 'tools' && <ToolsPanel />}
    </div>
  )
}

const toolCard: React.CSSProperties = {
  background: 'var(--card)',
  border: '1px solid var(--border)',
  borderRadius: 10,
  padding: 20,
  display: 'flex',
  flexDirection: 'column',
  gap: 12,
}

const launchBtn: React.CSSProperties = {
  background: '#7c3aed',
  color: '#fff',
  border: 'none',
  borderRadius: 6,
  padding: '10px 20px',
  fontWeight: 600,
  fontSize: 13,
  cursor: 'pointer',
}

function openDeepLink(scheme: string, params: Record<string, string>) {
  const sp = new URLSearchParams(params)
  window.location.href = `${scheme}://open?${sp.toString()}`
}

function ToolsPanel() {
  const scadaUrl = window.location.origin

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <div style={{ fontSize: 14, fontWeight: 700, color: 'var(--fg)' }}>External Tools</div>
      <div style={{ fontSize: 12, color: 'var(--muted)', lineHeight: 1.5 }}>
        Launch Voltrus desktop apps with SCADA context. Requires the desktop app to be installed.
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
        <div style={toolCard}>
          <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--fg)' }}>Voltrus PLC</div>
          <div style={{ fontSize: 12, color: 'var(--muted)', lineHeight: 1.5 }}>
            IEC 61131-3 Structured Text editor with softPLC runtime. Write logic, compile, simulate, deploy.
          </div>
          <button style={launchBtn} onClick={() => openDeepLink('voltrus-plc', { scada: scadaUrl })}>
            Open Voltrus PLC
          </button>
        </div>

        <div style={toolCard}>
          <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--fg)' }}>Voltrus HMI</div>
          <div style={{ fontSize: 12, color: 'var(--muted)', lineHeight: 1.5 }}>
            Drag-and-drop HMI builder for industrial PLCs. Design operator dashboards, deploy as standalone HTML.
          </div>
          <button style={{ ...launchBtn, background: '#f97316' }} onClick={() => openDeepLink('voltrus-hmi', { scada: scadaUrl })}>
            Open Voltrus HMI
          </button>
        </div>
      </div>
    </div>
  )
}

function SystemOverview({ systemInfo }: { systemInfo: SystemInfo | null | undefined }) {
  if (!systemInfo) {
    return <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: 300, color: 'var(--muted)' }}>Connecting to server...</div>
  }

  const barColor = (pct: number) => pct > 90 ? 'var(--crit)' : pct > 70 ? 'var(--warn)' : 'var(--ok)'

  return (
    <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))', gap: 16 }}>
      <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, padding: 20 }}>
        <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', marginBottom: 8 }}>Host</div>
        <div style={{ fontSize: 18, fontWeight: 700 }}>{systemInfo.hostname}</div>
        <div style={{ fontSize: 13, color: 'var(--muted)' }}>{systemInfo.os} — up {Math.floor(systemInfo.uptime_secs / 86400)}d {Math.floor((systemInfo.uptime_secs % 86400) / 3600)}h {Math.floor((systemInfo.uptime_secs % 3600) / 60)}m</div>
      </div>
      <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, padding: 20 }}>
        <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', marginBottom: 8 }}>CPU Usage</div>
        <div style={{ fontSize: 28, fontWeight: 700, color: systemInfo.cpu_usage > 80 ? 'var(--crit)' : systemInfo.cpu_usage > 50 ? 'var(--warn)' : 'var(--ok)' }}>{systemInfo.cpu_usage.toFixed(1)}%</div>
        <div style={{ marginTop: 8, height: 6, borderRadius: 3, background: 'var(--border)' }}>
          <div style={{ height: '100%', borderRadius: 3, background: barColor(systemInfo.cpu_usage), width: `${Math.min(systemInfo.cpu_usage, 100)}%`, transition: 'width 0.3s' }} />
        </div>
      </div>
      <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, padding: 20 }}>
        <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', marginBottom: 8 }}>Memory</div>
        <div style={{ fontSize: 28, fontWeight: 700 }}>{systemInfo.memory_used_mb.toFixed(0)} <span style={{ fontSize: 14, color: 'var(--muted)' }}>/ {systemInfo.memory_total_mb.toFixed(0)} MB</span></div>
        <div style={{ fontSize: 13, color: 'var(--muted)', marginTop: 4 }}>{systemInfo.memory_percent.toFixed(1)}%</div>
        <div style={{ marginTop: 8, height: 6, borderRadius: 3, background: 'var(--border)' }}>
          <div style={{ height: '100%', borderRadius: 3, background: barColor(systemInfo.memory_percent), width: `${Math.min(systemInfo.memory_percent, 100)}%`, transition: 'width 0.3s' }} />
        </div>
      </div>
      <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, padding: 20 }}>
        <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', marginBottom: 8 }}>Disk</div>
        <div style={{ fontSize: 28, fontWeight: 700 }}>{systemInfo.disk_used_gb.toFixed(1)} <span style={{ fontSize: 14, color: 'var(--muted)' }}>/ {systemInfo.disk_total_gb.toFixed(1)} GB</span></div>
        <div style={{ fontSize: 13, color: 'var(--muted)', marginTop: 4 }}>{systemInfo.disk_percent.toFixed(1)}%</div>
        <div style={{ marginTop: 8, height: 6, borderRadius: 3, background: 'var(--border)' }}>
          <div style={{ height: '100%', borderRadius: 3, background: barColor(systemInfo.disk_percent), width: `${Math.min(systemInfo.disk_percent, 100)}%`, transition: 'width 0.3s' }} />
        </div>
      </div>
      <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, padding: 20 }}>
        <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', marginBottom: 8 }}>Database</div>
        <div style={{ fontSize: 28, fontWeight: 700, color: systemInfo.db_size_mb > 512 ? 'var(--warn)' : 'var(--cyan)' }}>{systemInfo.db_size_mb > 1024 ? `${(systemInfo.db_size_mb / 1024).toFixed(1)} GB` : `${systemInfo.db_size_mb.toFixed(1)} MB`}</div>
        <div style={{ fontSize: 13, color: 'var(--muted)', marginTop: 4 }}>{systemInfo.db_size_mb > 1024 ? `${systemInfo.db_size_mb.toFixed(0)} MB` : 'SQLite file'}</div>
      </div>
      <div style={{ background: 'linear-gradient(135deg, #1a1a2e 0%, #16213e 100%)', border: '1px solid #22c55e', borderRadius: 10, padding: 20, gridColumn: 'span 2' }}>
        <div style={{ fontSize: 11, color: '#22c55e', textTransform: 'uppercase', letterSpacing: '.08em', marginBottom: 12, fontWeight: 700 }}>Rust Service (PID {systemInfo.process.pid})</div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 16 }}>
          <div><div style={{ fontSize: 24, fontWeight: 700, color: '#22c55e' }}>{systemInfo.process.memory_mb.toFixed(1)} MB</div><div style={{ fontSize: 11, color: 'var(--muted)' }}>RSS Memory</div></div>
          <div><div style={{ fontSize: 24, fontWeight: 700, color: '#22c55e' }}>{systemInfo.process.cpu_percent.toFixed(1)}%</div><div style={{ fontSize: 11, color: 'var(--muted)' }}>CPU Usage</div></div>
          <div><div style={{ fontSize: 24, fontWeight: 700, color: '#22c55e' }}>{Math.floor(systemInfo.process.uptime_secs / 86400)}d {Math.floor((systemInfo.process.uptime_secs % 86400) / 3600)}h {Math.floor((systemInfo.process.uptime_secs % 3600) / 60)}m</div><div style={{ fontSize: 11, color: 'var(--muted)' }}>Uptime</div></div>
          <div><div style={{ fontSize: 24, fontWeight: 700, color: '#22c55e' }}>{systemInfo.process.binary_size_mb.toFixed(1)} MB</div><div style={{ fontSize: 11, color: 'var(--muted)' }}>Binary Size</div></div>
        </div>
      </div>
    </div>
  )
}
