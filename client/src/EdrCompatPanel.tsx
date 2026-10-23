import React, { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { fetchJson, fetchOk } from './queries'

interface EdrProduct {
  name: string
  vendor: string
  active: boolean
  notes: string | null
  category: string
}

interface EdrExclusion {
  path: string
  exclusion_type: string
  reason: string
  priority: string
}

interface IoOptimizationStatus {
  batch_writes_enabled: boolean
  batch_size: number
  wal_mode_active: boolean
  checkpoint_optimized: boolean
  write_amplification: number
  sequential_write_pct: number
}

interface EdrReport {
  detected_products: EdrProduct[]
  recommended_exclusions: EdrExclusion[]
  io_optimization: IoOptimizationStatus
  compatible: boolean
  warnings: string[]
  recommendations: string[]
}

const fetchEdrReport = async (): Promise<EdrReport> => {
  return fetchJson<EdrReport>('/api/v1/edr/report')
}

const copyToClipboard = (text: string) => {
  navigator.clipboard.writeText(text).catch(() => {})
}

const CATEGORY_MAP: Record<string, string> = {
  antivirus: 'Antivirus',
  edr: 'EDR',
  xdr: 'XDR',
  ot_security: 'OT Security',
}

const PRIORITY_STYLES: Record<string, { bg: string; color: string; border: string }> = {
  critical: { bg: 'rgba(239,68,68,.08)', color: 'var(--crit)', border: 'var(--crit)' },
  recommended: { bg: 'rgba(245,158,11,.08)', color: 'var(--warn)', border: 'var(--warn)' },
  optional: { bg: 'var(--bg)', color: 'var(--muted)', border: 'var(--border)' },
}

const thStyle: React.CSSProperties = {
  textAlign: 'left', padding: '8px 12px', fontSize: 11, color: 'var(--muted)',
  textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600,
  borderBottom: '1px solid var(--border)',
}

const tdStyle: React.CSSProperties = {
  padding: '8px 12px', fontSize: 13, borderBottom: '1px solid var(--border)',
}

function BoolBadge({ value, label }: { value: boolean; label: string }) {
  return (
    <div style={{
      padding: '8px 12px', background: 'var(--bg)', borderRadius: 6,
      display: 'flex', justifyContent: 'space-between', alignItems: 'center',
    }}>
      <span style={{ fontSize: 12, color: 'var(--muted)' }}>{label}</span>
      <span style={{
        fontSize: 12, fontWeight: 600,
        color: value ? 'var(--ok)' : 'var(--warn)',
      }}>
        {value ? 'Enabled' : 'Disabled'}
      </span>
    </div>
  )
}

export const EdrCompatPanel: React.FC<{ onClose: () => void }> = ({ onClose }) => {
  const [tab, setTab] = useState<'report' | 'exclusions' | 'guide'>('report')
  const [ps1Script, setPs1Script] = useState<string | null>(null)
  const [gpoGuide, setGpoGuide] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)

  const { data: report, isLoading, error } = useQuery({
    queryKey: ['edr-report'],
    queryFn: fetchEdrReport,
  })

  // Lazy-load PS1 script and GPO guide
  React.useEffect(() => {
    if (tab === 'exclusions' && !ps1Script) {
      fetchOk('/api/v1/edr/exclusions/ps1')
        .then(r => r.text())
        .then(setPs1Script)
        .catch(() => setPs1Script(''))
    }
    if (tab === 'guide' && !gpoGuide) {
      fetchOk('/api/v1/edr/gpo-guide')
        .then(r => r.text())
        .then(setGpoGuide)
        .catch(() => setGpoGuide(''))
    }
  }, [tab])

  const handleCopy = () => {
    if (!ps1Script) return
    copyToClipboard(ps1Script)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  const tabs = (
    <div style={{ display: 'flex', gap: 0, marginLeft: 'auto' }}>
      {(['report', 'exclusions', 'guide'] as const).map(key => (
        <button
          key={key}
          onClick={() => setTab(key)}
          style={{
            padding: '4px 14px', border: 'none', cursor: 'pointer',
            background: tab === key ? 'var(--card)' : 'transparent',
            color: tab === key ? 'var(--fg)' : 'var(--muted)',
            fontWeight: tab === key ? 600 : 400,
            borderBottom: tab === key ? '2px solid var(--cyan)' : '2px solid transparent',
            fontSize: 12, transition: 'color 0.15s, border-color 0.15s',
          }}
        >
          {key === 'report' ? 'Report' : key === 'exclusions' ? 'Exclusions' : 'Deploy Guide'}
        </button>
      ))}
    </div>
  )

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      {/* Inline tab bar */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 0,
        borderBottom: '1px solid var(--border)', marginBottom: 16, flexShrink: 0,
      }}>
        {(['report', 'exclusions', 'guide'] as const).map(key => (
          <button
            key={key}
            onClick={() => setTab(key)}
            style={{
              padding: '8px 16px', border: 'none', cursor: 'pointer',
              background: tab === key ? 'var(--card)' : 'transparent',
              color: tab === key ? 'var(--fg)' : 'var(--muted)',
              fontWeight: tab === key ? 600 : 400,
              borderBottom: tab === key ? '2px solid var(--cyan)' : '2px solid transparent',
              fontSize: 13, transition: 'color 0.15s, border-color 0.15s',
            }}
          >
            {key === 'report' ? 'Report' : key === 'exclusions' ? 'Exclusions' : 'Deploy Guide'}
          </button>
        ))}
      </div>

      {/* Content */}
      <div style={{ flex: 1, overflow: 'auto', minHeight: 0 }}>
        {isLoading && (
          <div style={{ textAlign: 'center', padding: 48, color: 'var(--muted)', fontSize: 13 }}>
            Scanning system for EDR/antivirus products...
          </div>
        )}

        {error && (
          <div style={{
            padding: 16, borderRadius: 8, fontSize: 13,
            background: 'rgba(239,68,68,.08)', color: 'var(--crit)',
            borderLeft: '3px solid var(--crit)',
          }}>
            {error.message}
          </div>
        )}

        {report && tab === 'report' && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 20 }}>
            {/* Status banner */}
            <div style={{
              padding: '12px 16px', borderRadius: 8,
              background: report.compatible ? 'rgba(34,197,94,.06)' : 'rgba(239,68,68,.06)',
              border: `1px solid ${report.compatible ? 'rgba(34,197,94,.2)' : 'rgba(239,68,68,.2)'}`,
              display: 'flex', alignItems: 'center', gap: 10,
            }}>
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke={report.compatible ? 'var(--ok)' : 'var(--crit)'} strokeWidth="2">
                {report.compatible
                  ? <><path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/><polyline points="22 4 12 14.01 9 11.01"/></>
                  : <><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></>
                }
              </svg>
              <div>
                <div style={{ fontWeight: 600, fontSize: 14, color: report.compatible ? 'var(--ok)' : 'var(--crit)' }}>
                  {report.compatible ? 'System is EDR-compatible' : 'EDR compatibility issues detected'}
                </div>
                <div style={{ fontSize: 12, color: 'var(--muted)', marginTop: 2 }}>
                  {report.detected_products.length} product(s) detected / {report.warnings.length} warning(s)
                </div>
              </div>
            </div>

            {/* Detected products */}
            <div>
              <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 8, color: 'var(--fg)' }}>
                Detected Security Products
              </div>
              {report.detected_products.length === 0 ? (
                <div style={{ padding: 16, background: 'var(--bg)', borderRadius: 6, color: 'var(--muted)', fontSize: 13 }}>
                  No EDR/antivirus products detected on this system.
                </div>
              ) : (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                  {report.detected_products.map((p, i) => (
                    <div key={i} style={{
                      padding: '10px 14px', background: 'var(--bg)', borderRadius: 6,
                      display: 'flex', alignItems: 'center', gap: 12,
                    }}>
                      <span style={{
                        fontSize: 11, fontWeight: 600, minWidth: 72, padding: '2px 8px',
                        borderRadius: 4, textAlign: 'center',
                        background: 'var(--card)', border: '1px solid var(--border)',
                        color: 'var(--muted)',
                      }}>
                        {CATEGORY_MAP[p.category] ?? p.category}
                      </span>
                      <div style={{ flex: 1 }}>
                        <span style={{ fontWeight: 500, fontSize: 13 }}>{p.name}</span>
                        <span style={{ color: 'var(--muted)', fontSize: 12, marginLeft: 6 }}>({p.vendor})</span>
                        {p.notes && <div style={{ fontSize: 12, color: 'var(--muted)', marginTop: 2 }}>{p.notes}</div>}
                      </div>
                      <span style={{
                        padding: '2px 8px', borderRadius: 4, fontSize: 11, fontWeight: 600,
                        background: p.active ? 'rgba(34,197,94,.08)' : 'var(--bg)',
                        color: p.active ? 'var(--ok)' : 'var(--muted)',
                        border: `1px solid ${p.active ? 'rgba(34,197,94,.2)' : 'var(--border)'}`,
                      }}>
                        {p.active ? 'Active' : 'Inactive'}
                      </span>
                    </div>
                  ))}
                </div>
              )}
            </div>

            {/* I/O Optimization */}
            <div>
              <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 8, color: 'var(--fg)' }}>
                I/O Optimization
              </div>
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(200px, 1fr))', gap: 6 }}>
                <BoolBadge label="Batch Writes" value={report.io_optimization.batch_writes_enabled} />
                <BoolBadge label="WAL Mode" value={report.io_optimization.wal_mode_active} />
                <BoolBadge label="Checkpoint Optimized" value={report.io_optimization.checkpoint_optimized} />
                <div style={{
                  padding: '8px 12px', background: 'var(--bg)', borderRadius: 6,
                  display: 'flex', justifyContent: 'space-between', alignItems: 'center',
                }}>
                  <span style={{ fontSize: 12, color: 'var(--muted)' }}>Batch Size</span>
                  <span style={{ fontSize: 12, fontWeight: 600 }}>{report.io_optimization.batch_size}</span>
                </div>
                <div style={{
                  padding: '8px 12px', background: 'var(--bg)', borderRadius: 6,
                  display: 'flex', justifyContent: 'space-between', alignItems: 'center',
                }}>
                  <span style={{ fontSize: 12, color: 'var(--muted)' }}>Write Amplification</span>
                  <span style={{ fontSize: 12, fontWeight: 600 }}>{report.io_optimization.write_amplification.toFixed(1)}x</span>
                </div>
                <div style={{
                  padding: '8px 12px', background: 'var(--bg)', borderRadius: 6,
                  display: 'flex', justifyContent: 'space-between', alignItems: 'center',
                }}>
                  <span style={{ fontSize: 12, color: 'var(--muted)' }}>Sequential Writes</span>
                  <span style={{ fontSize: 12, fontWeight: 600 }}>{(report.io_optimization.sequential_write_pct * 100).toFixed(0)}%</span>
                </div>
              </div>
            </div>

            {/* Warnings */}
            {report.warnings.length > 0 && (
              <div>
                <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 8, color: 'var(--warn)' }}>
                  Warnings
                </div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                  {report.warnings.map((w, i) => (
                    <div key={i} style={{
                      padding: '8px 12px', background: 'rgba(245,158,11,.06)', borderRadius: 6,
                      borderLeft: '3px solid var(--warn)', fontSize: 13,
                    }}>
                      {w}
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Recommendations */}
            {report.recommendations.length > 0 && (
              <div>
                <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 8, color: 'var(--cyan)' }}>
                  Recommendations
                </div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                  {report.recommendations.map((r, i) => (
                    <div key={i} style={{
                      padding: '8px 12px', background: 'var(--bg)', borderRadius: 6,
                      borderLeft: '3px solid var(--cyan)', fontSize: 13,
                    }}>
                      {r}
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
        )}

        {report && tab === 'exclusions' && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
            <div style={{ fontSize: 13, color: 'var(--muted)' }}>
              Add these exclusion paths to your EDR/antivirus policy. Critical exclusions are required for acceptable performance.
            </div>

            <div style={{ overflow: 'auto', background: 'var(--bg)', borderRadius: 8, border: '1px solid var(--border)' }}>
              <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                <thead>
                  <tr>
                    <th style={thStyle}>Priority</th>
                    <th style={thStyle}>Type</th>
                    <th style={thStyle}>Path</th>
                    <th style={thStyle}>Reason</th>
                  </tr>
                </thead>
                <tbody>
                  {report.recommended_exclusions.map((exc, i) => {
                    const s = PRIORITY_STYLES[exc.priority] ?? PRIORITY_STYLES.optional
                    return (
                      <tr key={i}>
                        <td style={tdStyle}>
                          <span style={{
                            padding: '2px 8px', borderRadius: 4, fontSize: 11, fontWeight: 600,
                            background: s.bg, color: s.color, border: `1px solid ${s.border}`,
                          }}>
                            {exc.priority.toUpperCase()}
                          </span>
                        </td>
                        <td style={{ ...tdStyle, fontFamily: "var(--font-mono)", fontSize: 12 }}>{exc.exclusion_type}</td>
                        <td style={{ ...tdStyle, fontFamily: "var(--font-mono)", fontSize: 12 }}>{exc.path}</td>
                        <td style={{ ...tdStyle, color: 'var(--muted)' }}>{exc.reason}</td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            </div>

            {ps1Script && (
              <div>
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 8 }}>
                  <div style={{ fontSize: 13, fontWeight: 600 }}>Windows Defender Script</div>
                  <button
                    onClick={handleCopy}
                    style={{
                      padding: '4px 12px', borderRadius: 6, border: '1px solid var(--border)',
                      background: 'var(--card)', color: copied ? 'var(--ok)' : 'var(--fg)',
                      fontSize: 12, cursor: 'pointer', fontWeight: 500,
                    }}
                  >
                    {copied ? 'Copied' : 'Copy Script'}
                  </button>
                </div>
                <pre style={{
                  background: 'var(--bg)', color: 'var(--fg)', padding: 16, borderRadius: 8,
                  border: '1px solid var(--border)',
                  fontSize: 12, overflow: 'auto', maxHeight: 300, lineHeight: 1.6,
                  fontFamily: "var(--font-mono)",
                }}>
                  {ps1Script}
                </pre>
              </div>
            )}
          </div>
        )}

        {tab === 'guide' && (
          <div>
            <div style={{ fontSize: 13, color: 'var(--muted)', marginBottom: 12 }}>
              OT Network Deployment Guide for IT teams deploying Voltrus to SCADA environments.
            </div>
            {gpoGuide ? (
              <pre style={{
                background: 'var(--bg)', padding: 16, borderRadius: 8,
                border: '1px solid var(--border)',
                fontSize: 12, overflow: 'auto', maxHeight: 'calc(100vh - 280px)', lineHeight: 1.7,
                whiteSpace: 'pre-wrap', fontFamily: 'inherit',
              }}>
                {gpoGuide}
              </pre>
            ) : (
              <div style={{ color: 'var(--muted)', fontSize: 13 }}>Loading deployment guide...</div>
            )}
          </div>
        )}
      </div>
    </div>
  )
}
