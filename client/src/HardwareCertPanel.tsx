import React, { useState } from 'react'
import { useQuery, useMutation } from '@tanstack/react-query'
import { fetchJson, fetchOk } from './queries'

interface PlatformInfo {
  hostname: string
  os: string
  arch: string
  cpu_model: string
  cpu_cores: number
  ram_total_mb: number
  disk_total_gb: number
  disk_available_gb: number
  board_type: string
  storage_type: string
  is_raspberry_pi: boolean
  is_industrial_pc: boolean
  kernel_version: string
}

interface TierAssessment {
  compatible: boolean
  max_tags: number
  max_poll_rate: number
  compression_recommended: boolean
  limitations: string[]
}

interface StorageAssessment {
  storage_type: string
  write_optimization_critical: boolean
  estimated_endurance_years: number
  batch_writes_active: boolean
  write_amplification: number
  storage_recommendations: string[]
}

interface HwCertReport {
  platform: PlatformInfo
  compatibility: {
    engineering: TierAssessment
    starter: TierAssessment
    professional: TierAssessment
    enterprise: TierAssessment
  }
  storage_assessment: StorageAssessment
  certified: boolean
  certified_tiers: string[]
  warnings: string[]
  recommendations: string[]
}

interface BenchmarkDetail {
  name: string
  operations: number
  duration_ms: number
  throughput: number
  passed: boolean
}

interface BenchmarkResults {
  tag_write_throughput: number
  batch_insert_throughput: number
  compression_throughput: number
  query_throughput: number
  peak_memory_mb: number
  duration_secs: number
  recommended_max_tags: number
  recommended_poll_rate: number
  write_amplification: number
  details: BenchmarkDetail[]
}

interface CompatEntry {
  platform: string
  cpu: string
  ram_mb: number
  storage: string
  supported_tiers: string[]
  max_tags: number
  max_poll_rate: number
  notes: string
}

const fetchHwReport = async (): Promise<HwCertReport> => {
  return fetchJson<HwCertReport>('/api/v1/hw/report')
}

const fetchCompatMatrix = async (): Promise<{ matrix: CompatEntry[] }> => {
  return fetchJson<{ matrix: CompatEntry[] }>('/api/v1/hw/compatibility-matrix')
}

export const HardwareCertPanel: React.FC<{ onClose: () => void }> = ({ onClose }) => {
  const [tab, setTab] = useState<'report' | 'benchmark' | 'matrix' | 'deploy'>('report')
  const [edgeGuide, setEdgeGuide] = useState<string | null>(null)

  const { data: report, isLoading, error } = useQuery({
    queryKey: ['hw-report'],
    queryFn: fetchHwReport,
  })

  const { data: matrixData } = useQuery({
    queryKey: ['hw-compat-matrix'],
    queryFn: fetchCompatMatrix,
    enabled: tab === 'matrix',
  })

  const benchmarkMutation = useMutation({
    mutationFn: async () => {
      const res = await fetchOk('/api/v1/hw/benchmark?duration=5', { method: 'POST' })
      return res.json() as Promise<BenchmarkResults>
    },
  })

  const loadEdgeGuide = async () => {
    if (edgeGuide) return
    const res = await fetchOk('/api/v1/hw/edge-deploy')
    const text = await res.text()
    setEdgeGuide(text)
  }

  const formatThroughput = (n: number) => {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
    if (n >= 1000) return `${(n / 1000).toFixed(1)}K`
    return n.toFixed(0)
  }

  const tierLabel = (tier: string) => {
    switch (tier) {
      case 'engineering': return 'Engineering'
      case 'starter': return 'Starter'
      case 'professional': return 'Professional'
      case 'enterprise': return 'Enterprise'
      default: return tier
    }
  }

  const storageIcon = (type: string) => {
    switch (type) {
      case 'sd_card': return 'SD Card'
      case 'ssd': return 'SSD'
      case 'nvme': return 'NVMe'
      case 'hdd': return 'HDD'
      case 'emmc': return 'eMMC'
      default: return 'Unknown'
    }
  }

  const boardIcon = (platform: PlatformInfo) => {
    if (platform.is_raspberry_pi) return 'Raspberry Pi'
    if (platform.is_industrial_pc) return 'Industrial PC'
    return 'Desktop'
  }

  return (
    <div style={{
      position: 'fixed', inset: 0, zIndex: 1000,
      background: 'rgba(0,0,0,0.5)', display: 'flex',
      alignItems: 'center', justifyContent: 'center',
    }}>
      <div style={{
        width: '92%', maxWidth: 960, maxHeight: '88vh',
        background: 'var(--card)', borderRadius: 12,
        border: '1px solid var(--border)', display: 'flex',
        flexDirection: 'column', overflow: 'hidden',
        boxShadow: '0 20px 60px rgba(0,0,0,0.3)',
      }}>
        {/* Header */}
        <div style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          padding: '16px 20px', borderBottom: '1px solid var(--border)',
        }}>
          <div>
            <h2 style={{ margin: 0, fontSize: 18, fontWeight: 600 }}>
              Hardware Certification & Edge Deployment
            </h2>
            <span style={{ fontSize: 12, color: 'var(--muted)' }}>
              v0.38.0 — Test, certify, and deploy Voltrus on OT hardware
            </span>
          </div>
          <button onClick={onClose} style={{
            background: 'none', border: 'none', color: 'var(--fg)',
            fontSize: 20, cursor: 'pointer', padding: 4,
          }}>✕</button>
        </div>

        {/* Tabs */}
        <div style={{
          display: 'flex', gap: 0, borderBottom: '1px solid var(--border)',
          background: 'var(--bg-surface)',
        }}>
          {(['report', 'benchmark', 'matrix', 'deploy'] as const).map(t => (
            <button key={t} onClick={() => {
              setTab(t)
              if (t === 'deploy') loadEdgeGuide()
            }} style={{
              flex: 1, padding: '10px 16px', border: 'none', cursor: 'pointer',
              background: tab === t ? 'var(--card)' : 'transparent',
              borderBottom: tab === t ? '2px solid var(--ok)' : '2px solid transparent',
              color: tab === t ? 'var(--fg)' : 'var(--muted)', fontSize: 13, fontWeight: 500,
            }}>
              {t === 'report' && 'Report'}
              {t === 'benchmark' && 'Benchmark'}
              {t === 'matrix' && 'Compatibility'}
              {t === 'deploy' && 'Edge Deploy'}
            </button>
          ))}
        </div>

        {/* Content */}
        <div style={{ flex: 1, overflow: 'auto', padding: 20 }}>
          {isLoading && tab === 'report' && (
            <div style={{ textAlign: 'center', padding: 40, color: 'var(--muted)' }}>
              Detecting hardware...
            </div>
          )}

          {error && (
            <div style={{ color: 'var(--crit)', padding: 20 }}>Error: {error.message}</div>
          )}

          {/* Report Tab */}
          {tab === 'report' && report && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
              {/* Platform Info */}
              <div style={{
                background: 'linear-gradient(135deg, #1a1a2e 0%, #16213e 100%)',
                border: '1px solid #22c55e', borderRadius: 10, padding: 20,
              }}>
                <div style={{ fontSize: 11, color: '#22c55e', textTransform: 'uppercase', letterSpacing: '.08em', marginBottom: 12, fontWeight: 700 }}>
                  {boardIcon(report.platform)} {report.platform.board_type}
                </div>
                <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(180px, 1fr))', gap: 16 }}>
                  <div>
                    <div style={{ fontSize: 20, fontWeight: 700 }}>{report.platform.hostname}</div>
                    <div style={{ fontSize: 12, color: 'var(--muted)' }}>{report.platform.os} ({report.platform.kernel_version})</div>
                  </div>
                  <div>
                    <div style={{ fontSize: 20, fontWeight: 700 }}>{report.platform.cpu_model}</div>
                    <div style={{ fontSize: 12, color: 'var(--muted)' }}>{report.platform.cpu_cores} cores · {report.platform.arch}</div>
                  </div>
                  <div>
                    <div style={{ fontSize: 20, fontWeight: 700 }}>{report.platform.ram_total_mb} MB</div>
                    <div style={{ fontSize: 12, color: 'var(--muted)' }}>RAM</div>
                  </div>
                  <div>
                    <div style={{ fontSize: 20, fontWeight: 700 }}>{storageIcon(report.storage_assessment.storage_type)}</div>
                    <div style={{ fontSize: 12, color: 'var(--muted)' }}>
                      {report.platform.disk_available_gb.toFixed(1)} / {report.platform.disk_total_gb.toFixed(1)} GB
                    </div>
                  </div>
                </div>
              </div>

              {/* Certification Status */}
              <div style={{
                padding: 16, borderRadius: 8,
                background: report.certified ? 'rgba(34,197,94,0.1)' : 'rgba(239,68,68,0.1)',
                border: `1px solid ${report.certified ? '#22c55e' : '#ef4444'}`,
              }}>
                <div style={{ fontSize: 16, fontWeight: 700, color: report.certified ? '#22c55e' : '#ef4444' }}>
                  {report.certified ? '✓ Hardware Certified' : '✗ Not Fully Certified'}
                </div>
                <div style={{ fontSize: 13, color: 'var(--muted)', marginTop: 4 }}>
                  Supported tiers: {report.certified_tiers.map(tierLabel).join(', ')}
                </div>
              </div>

              {/* Tier Compatibility */}
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(200px, 1fr))', gap: 12 }}>
                {Object.entries(report.compatibility).map(([tier, assessment]) => (
                  <div key={tier} style={{
                    background: 'var(--bg-surface)', borderRadius: 8, padding: 14,
                    border: `1px solid ${assessment.compatible ? '#22c55e' : 'var(--border)'}`,
                    opacity: assessment.compatible ? 1 : 0.6,
                  }}>
                    <div style={{ fontSize: 13, fontWeight: 600, color: assessment.compatible ? '#22c55e' : 'var(--muted)' }}>
                      {tierLabel(tier)}
                    </div>
                    {assessment.compatible ? (
                      <div style={{ marginTop: 8, fontSize: 12, color: 'var(--muted)' }}>
                        <div>↑ {assessment.max_tags.toLocaleString()} tags</div>
                        <div>↑ {assessment.max_poll_rate} Hz poll rate</div>
                        {assessment.compression_recommended && <div style={{ color: '#f59e0b' }}>Compression recommended</div>}
                      </div>
                    ) : (
                      <div style={{ marginTop: 8, fontSize: 12, color: 'var(--muted)' }}>
                        {assessment.limitations.map((l, i) => (
                          <div key={i} style={{ color: '#ef4444' }}>⚠ {l}</div>
                        ))}
                      </div>
                    )}
                  </div>
                ))}
              </div>

              {/* Storage */}
              <div style={{ background: 'var(--bg-surface)', borderRadius: 8, padding: 16, border: '1px solid var(--border)' }}>
                <div style={{ fontSize: 14, fontWeight: 600, marginBottom: 8 }}>Storage Assessment</div>
                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 12, fontSize: 13 }}>
                  <div>
                    <div style={{ color: 'var(--muted)' }}>Endurance</div>
                    <div style={{ fontWeight: 600 }}>{report.storage_assessment.estimated_endurance_years.toFixed(1)} years</div>
                  </div>
                  <div>
                    <div style={{ color: 'var(--muted)' }}>Write Optimization</div>
                    <div style={{ fontWeight: 600, color: report.storage_assessment.write_optimization_critical ? '#f59e0b' : 'var(--fg)' }}>
                      {report.storage_assessment.write_optimization_critical ? 'Critical' : 'Standard'}
                    </div>
                  </div>
                  <div>
                    <div style={{ color: 'var(--muted)' }}>Batch Writes</div>
                    <div style={{ fontWeight: 600, color: report.storage_assessment.batch_writes_active ? '#22c55e' : '#ef4444' }}>
                      {report.storage_assessment.batch_writes_active ? 'Active' : 'Disabled'}
                    </div>
                  </div>
                </div>
                {report.storage_assessment.storage_recommendations.length > 0 && (
                  <div style={{ marginTop: 10, fontSize: 12 }}>
                    {report.storage_assessment.storage_recommendations.map((r, i) => (
                      <div key={i} style={{ color: 'var(--muted)', marginBottom: 2 }}>→ {r}</div>
                    ))}
                  </div>
                )}
              </div>

              {/* Warnings & Recommendations */}
              {(report.warnings.length > 0 || report.recommendations.length > 0) && (
                <div style={{ display: 'flex', gap: 12 }}>
                  {report.warnings.length > 0 && (
                    <div style={{ flex: 1, background: 'rgba(245,158,11,0.08)', borderRadius: 8, padding: 14, border: '1px solid rgba(245,158,11,0.3)' }}>
                      <div style={{ fontSize: 13, fontWeight: 600, color: '#f59e0b', marginBottom: 6 }}>Warnings</div>
                      {report.warnings.map((w, i) => (
                        <div key={i} style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 2 }}>⚠ {w}</div>
                      ))}
                    </div>
                  )}
                  {report.recommendations.length > 0 && (
                    <div style={{ flex: 1, background: 'var(--bg-surface)', borderRadius: 8, padding: 14, border: '1px solid var(--border)' }}>
                      <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 6 }}>Recommendations</div>
                      {report.recommendations.map((r, i) => (
                        <div key={i} style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 2 }}>→ {r}</div>
                      ))}
                    </div>
                  )}
                </div>
              )}
            </div>
          )}

          {/* Benchmark Tab */}
          {tab === 'benchmark' && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                <div style={{ fontSize: 14, fontWeight: 600 }}>Performance Benchmark</div>
                <button
                  onClick={() => benchmarkMutation.mutate()}
                  disabled={benchmarkMutation.isPending}
                  style={{
                    padding: '8px 20px', borderRadius: 6, border: 'none',
                    background: benchmarkMutation.isPending ? 'var(--muted)' : '#22c55e',
                    color: benchmarkMutation.isPending ? '#666' : '#000',
                    fontWeight: 600, cursor: benchmarkMutation.isPending ? 'wait' : 'pointer',
                    fontSize: 13,
                  }}
                >
                  {benchmarkMutation.isPending ? 'Running...' : 'Run Benchmark (~20s)'}
                </button>
              </div>

              {benchmarkMutation.isError && (
                <div style={{ color: 'var(--crit)', fontSize: 13 }}>
                  Error: {(benchmarkMutation.error as Error).message}
                </div>
              )}

              {benchmarkMutation.data && (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
                  {/* Summary */}
                  <div style={{
                    background: 'linear-gradient(135deg, #1a1a2e 0%, #16213e 100%)',
                    border: '1px solid #22c55e', borderRadius: 10, padding: 20,
                  }}>
                    <div style={{ fontSize: 11, color: '#22c55e', textTransform: 'uppercase', letterSpacing: '.08em', marginBottom: 12, fontWeight: 700 }}>
                      Benchmark Summary
                    </div>
                    <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(140px, 1fr))', gap: 16 }}>
                      <div>
                        <div style={{ fontSize: 22, fontWeight: 700, color: '#22c55e' }}>
                          {formatThroughput(benchmarkMutation.data.tag_write_throughput)}
                        </div>
                        <div style={{ fontSize: 11, color: 'var(--muted)' }}>Tag writes/s</div>
                      </div>
                      <div>
                        <div style={{ fontSize: 22, fontWeight: 700, color: '#22c55e' }}>
                          {formatThroughput(benchmarkMutation.data.batch_insert_throughput)}
                        </div>
                        <div style={{ fontSize: 11, color: 'var(--muted)' }}>Batch rows/s</div>
                      </div>
                      <div>
                        <div style={{ fontSize: 22, fontWeight: 700, color: '#22c55e' }}>
                          {formatThroughput(benchmarkMutation.data.query_throughput)}
                        </div>
                        <div style={{ fontSize: 11, color: 'var(--muted)' }}>Queries/s</div>
                      </div>
                      <div>
                        <div style={{ fontSize: 22, fontWeight: 700, color: '#22c55e' }}>
                          {formatThroughput(benchmarkMutation.data.compression_throughput)}
                        </div>
                        <div style={{ fontSize: 11, color: 'var(--muted)' }}>Compression/s</div>
                      </div>
                    </div>
                  </div>

                  {/* Recommendations */}
                  <div style={{
                    background: 'var(--bg-surface)', borderRadius: 8, padding: 16,
                    border: '1px solid var(--border)',
                  }}>
                    <div style={{ fontSize: 14, fontWeight: 600, marginBottom: 10 }}>Recommendations</div>
                    <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 12, fontSize: 13 }}>
                      <div>
                        <div style={{ color: 'var(--muted)' }}>Max Tags</div>
                        <div style={{ fontWeight: 700, fontSize: 18 }}>{benchmarkMutation.data.recommended_max_tags.toLocaleString()}</div>
                      </div>
                      <div>
                        <div style={{ color: 'var(--muted)' }}>Max Poll Rate</div>
                        <div style={{ fontWeight: 700, fontSize: 18 }}>{benchmarkMutation.data.recommended_poll_rate} Hz</div>
                      </div>
                      <div>
                        <div style={{ color: 'var(--muted)' }}>Write Amplification</div>
                        <div style={{ fontWeight: 700, fontSize: 18 }}>{benchmarkMutation.data.write_amplification.toFixed(1)}×</div>
                      </div>
                    </div>
                  </div>

                  {/* Individual Benchmarks */}
                  <div style={{ background: 'var(--bg-surface)', borderRadius: 8, padding: 16, border: '1px solid var(--border)' }}>
                    <div style={{ fontSize: 14, fontWeight: 600, marginBottom: 10 }}>Individual Tests</div>
                    <table style={{ width: '100%', fontSize: 13, borderCollapse: 'collapse' }}>
                      <thead>
                        <tr style={{ borderBottom: '1px solid var(--border)' }}>
                          <th style={{ textAlign: 'left', padding: 6, color: 'var(--muted)' }}>Test</th>
                          <th style={{ textAlign: 'right', padding: 6, color: 'var(--muted)' }}>Throughput</th>
                          <th style={{ textAlign: 'right', padding: 6, color: 'var(--muted)' }}>Ops</th>
                          <th style={{ textAlign: 'right', padding: 6, color: 'var(--muted)' }}>Time</th>
                          <th style={{ textAlign: 'center', padding: 6, color: 'var(--muted)' }}>Status</th>
                        </tr>
                      </thead>
                      <tbody>
                        {benchmarkMutation.data.details.map((d, i) => (
                          <tr key={i} style={{ borderBottom: '1px solid var(--border)' }}>
                            <td style={{ padding: 8 }}>{d.name}</td>
                            <td style={{ padding: 8, textAlign: 'right', fontWeight: 600 }}>
                              {formatThroughput(d.throughput)}/s
                            </td>
                            <td style={{ padding: 8, textAlign: 'right', color: 'var(--muted)' }}>
                              {d.operations.toLocaleString()}
                            </td>
                            <td style={{ padding: 8, textAlign: 'right', color: 'var(--muted)' }}>
                              {(d.duration_ms / 1000).toFixed(1)}s
                            </td>
                            <td style={{ padding: 8, textAlign: 'center' }}>
                              <span style={{
                                color: d.passed ? '#22c55e' : '#ef4444',
                                fontWeight: 600,
                              }}>
                                {d.passed ? 'PASS' : 'FAIL'}
                              </span>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </div>
              )}

              {!benchmarkMutation.data && !benchmarkMutation.isPending && (
                <div style={{
                  textAlign: 'center', padding: 40, color: 'var(--muted)',
                  border: '1px dashed var(--border)', borderRadius: 8,
                }}>
                  <div style={{ fontSize: 24, marginBottom: 12, fontFamily: 'var(--font-mono)', color: 'var(--muted)' }}>[Bench]</div>
                  <div>Click "Run Benchmark" to measure hardware performance.</div>
                  <div style={{ fontSize: 12, marginTop: 6 }}>Tests: tag writes, batch inserts, queries, compression (~20 seconds)</div>
                </div>
              )}
            </div>
          )}

          {/* Compatibility Matrix Tab */}
          {tab === 'matrix' && (
            <div>
              <div style={{ fontSize: 14, fontWeight: 600, marginBottom: 12 }}>
                Official Hardware Compatibility Matrix
              </div>
              <div style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 16 }}>
                Tested and certified hardware platforms. Click any row to see details.
              </div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                {matrixData?.matrix.map((entry, i) => (
                  <div key={i} style={{
                    background: 'var(--bg-surface)', borderRadius: 8, padding: 14,
                    border: '1px solid var(--border)',
                  }}>
                    <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'start' }}>
                      <div>
                        <div style={{ fontWeight: 600, fontSize: 14 }}>{entry.platform}</div>
                        <div style={{ fontSize: 12, color: 'var(--muted)', marginTop: 2 }}>{entry.cpu}</div>
                      </div>
                      <div style={{ display: 'flex', gap: 4 }}>
                        {entry.supported_tiers.map(t => (
                          <span key={t} style={{
                            padding: '2px 8px', borderRadius: 4, fontSize: 10,
                            background: 'rgba(34,197,94,0.1)', color: '#22c55e',
                            border: '1px solid rgba(34,197,94,0.3)',
                          }}>
                            {tierLabel(t)}
                          </span>
                        ))}
                      </div>
                    </div>
                    <div style={{ display: 'flex', gap: 16, marginTop: 8, fontSize: 12, color: 'var(--muted)' }}>
                      <span>RAM: {entry.ram_mb >= 1024 ? `${(entry.ram_mb / 1024).toFixed(0)} GB` : `${entry.ram_mb} MB`}</span>
                      <span>Storage: {entry.storage}</span>
                      <span>Max: {entry.max_tags.toLocaleString()} tags @ {entry.max_poll_rate} Hz</span>
                    </div>
                    {entry.notes && (
                      <div style={{ fontSize: 11, color: 'var(--muted)', marginTop: 6, fontStyle: 'italic' }}>
                        {entry.notes}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Edge Deploy Tab */}
          {tab === 'deploy' && (
            <div>
              <div style={{ fontSize: 14, fontWeight: 600, marginBottom: 12 }}>
                Edge Deployment Guide
              </div>
              <div style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 12 }}>
                Platform-specific deployment instructions generated for your hardware.
              </div>
              <pre style={{
                background: '#0a0a0a', color: '#e0e0e0', padding: 16,
                borderRadius: 8, fontSize: 12, lineHeight: 1.6,
                overflow: 'auto', maxHeight: '55vh', whiteSpace: 'pre-wrap',
                border: '1px solid var(--border)',
              }}>
                {edgeGuide || 'Loading deployment guide...'}
              </pre>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
