import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { fetchJson } from '../queries'
import { queryKeys } from '../queries/keys'

interface HealthIssue {
  severity: 'critical' | 'warning' | 'info' | 'pass'
  category: string
  title: string
  description: string
  suggestion: string
  affected_items: string[]
  count: number
}

interface HealthSummary {
  total_devices: number; total_tags: number; total_alarms: number
  total_flows: number; total_screens: number
  critical_issues: number; warning_issues: number; info_issues: number
  tags_without_alarms: number; tags_without_units: number
  unnamed_devices: number; devices_without_tags: number
  duplicate_tag_names: number; nonstandard_tag_names: number
  missing_dashboards: number; disabled_devices: number
}

interface HealthReport {
  overall_score: number
  issues: HealthIssue[]
  summary: HealthSummary
}

const severityColor: Record<string, string> = {
  critical: '#ef4444', warning: '#f59e0b', info: '#3b82f6', pass: '#16a34a',
}
const severityBg: Record<string, string> = {
  critical: 'rgba(239,68,68,.12)', warning: 'rgba(245,158,11,.12)', info: 'rgba(59,130,246,.12)', pass: 'rgba(22,163,74,.12)',
}

export function HealthCheckPanel() {
  const [expandedIssues, setExpandedIssues] = useState<Set<number>>(new Set())

  const { data: report, isLoading: loading, error: queryError, refetch: runCheck } = useQuery({
    queryKey: queryKeys.wizardHealthCheck,
    queryFn: () => fetchJson<HealthReport>('/api/v1/wizard/health-check'),
  })
  const error = queryError?.message || ''

  const toggleIssue = (idx: number) => {
    const next = new Set(expandedIssues)
    if (next.has(idx)) next.delete(idx); else next.add(idx)
    setExpandedIssues(next)
  }

  const scoreColor = (report?.overall_score ?? 100) >= 80 ? '#16a34a'
    : (report?.overall_score ?? 0) >= 50 ? '#f59e0b' : '#ef4444'

  return (
    <div style={{ padding: 16 }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 20 }}>
        <div>
          <h2 style={{ fontSize: 18, fontWeight: 600, margin: 0 }}>Project Health Check</h2>
          <p style={{ color: 'var(--muted)', fontSize: 13, margin: '4px 0 0' }}>Audit your configuration for common issues and best practices.</p>
        </div>
        <button onClick={() => runCheck()} disabled={loading} style={{
          padding: '8px 16px', borderRadius: 6, border: 'none', background: '#f97316', color: '#fff',
          cursor: loading ? 'not-allowed' : 'pointer', fontSize: 13, fontWeight: 600,
        }}>{loading ? 'Running…' : 'Run Health Check'}</button>
      </div>

      {error && <div style={{ color: '#ef4444', fontSize: 13, marginBottom: 12 }}>{error}</div>}

      {report && (
        <>
          {/* Score */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 24, marginBottom: 24, padding: 20, background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10 }}>
            <div style={{ width: 80, height: 80, borderRadius: '50%', border: `4px solid ${scoreColor}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
              <span style={{ fontSize: 28, fontWeight: 800, color: scoreColor }}>{report.overall_score}</span>
            </div>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 14, fontWeight: 600 }}>
                {report.overall_score >= 80 ? 'Project is healthy' : report.overall_score >= 50 ? 'Room for improvement' : 'Needs attention'}
              </div>
              <div style={{ fontSize: 12, color: 'var(--muted)', marginTop: 4 }}>
                {report.summary.total_devices} devices · {report.summary.total_tags} tags · {report.summary.total_alarms} alarms · {report.summary.total_flows} flows
              </div>
            </div>
            <div style={{ display: 'flex', gap: 16 }}>
              <div style={{ textAlign: 'center' }}><div style={{ fontSize: 20, fontWeight: 700, color: '#ef4444' }}>{report.summary.critical_issues}</div><div style={{ fontSize: 11, color: 'var(--muted)' }}>Critical</div></div>
              <div style={{ textAlign: 'center' }}><div style={{ fontSize: 20, fontWeight: 700, color: '#f59e0b' }}>{report.summary.warning_issues}</div><div style={{ fontSize: 11, color: 'var(--muted)' }}>Warnings</div></div>
              <div style={{ textAlign: 'center' }}><div style={{ fontSize: 20, fontWeight: 700, color: '#3b82f6' }}>{report.summary.info_issues}</div><div style={{ fontSize: 11, color: 'var(--muted)' }}>Info</div></div>
            </div>
          </div>

          {/* Summary stats */}
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(180px, 1fr))', gap: 10, marginBottom: 24 }}>
            {[
              ['Tags without Alarms', report.summary.tags_without_alarms, report.summary.tags_without_alarms > 0 ? 'warning' : 'pass'],
              ['Tags without Units', report.summary.tags_without_units, report.summary.tags_without_units > 0 ? 'info' : 'pass'],
              ['Unnamed Devices', report.summary.unnamed_devices, report.summary.unnamed_devices > 0 ? 'warning' : 'pass'],
              ['Empty Devices', report.summary.devices_without_tags, report.summary.devices_without_tags > 0 ? 'warning' : 'pass'],
              ['Duplicate Names', report.summary.duplicate_tag_names, report.summary.duplicate_tag_names > 0 ? 'warning' : 'pass'],
              ['Non-standard Names', report.summary.nonstandard_tag_names, report.summary.nonstandard_tag_names > 0 ? 'info' : 'pass'],
            ].map(([label, count, sev]) => (
              <div key={label as string} style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 8, padding: 12 }}>
                <div style={{ fontSize: 22, fontWeight: 700, color: severityColor[sev as string] }}>{count as number}</div>
                <div style={{ fontSize: 11, color: 'var(--muted)', marginTop: 2 }}>{label as string}</div>
              </div>
            ))}
          </div>

          {/* Issues list */}
          {report.issues.length === 0 ? (
            <div style={{ padding: 32, textAlign: 'center', color: '#16a34a', fontSize: 14, fontWeight: 600 }}>
              No issues found — your project is in great shape!
            </div>
          ) : (
            <div>
              <h3 style={{ fontSize: 15, fontWeight: 600, marginBottom: 12 }}>Issues Found</h3>
              {report.issues.map((issue, idx) => (
                <div key={idx} style={{ marginBottom: 8 }}>
                  <div onClick={() => toggleIssue(idx)} style={{
                    background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 8, padding: 12,
                    cursor: 'pointer', display: 'flex', alignItems: 'center', gap: 12,
                  }}>
                    <div style={{
                      width: 10, height: 10, borderRadius: '50%',
                      background: severityColor[issue.severity],
                      flexShrink: 0,
                    }} />
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ fontSize: 13, fontWeight: 600 }}>{issue.title}</div>
                      <div style={{ fontSize: 12, color: 'var(--muted)', marginTop: 2, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                        {issue.description}
                      </div>
                    </div>
                    <div style={{ display: 'flex', gap: 12, alignItems: 'center', flexShrink: 0 }}>
                      <span style={{ fontSize: 11, padding: '2px 8px', borderRadius: 4, background: severityBg[issue.severity], color: severityColor[issue.severity], fontWeight: 600, textTransform: 'uppercase' }}>{issue.severity}</span>
                      <span style={{ fontSize: 11, color: 'var(--muted)' }}>{issue.count} affected</span>
                      <span style={{ fontSize: 14, color: 'var(--muted)', transform: expandedIssues.has(idx) ? 'rotate(180deg)' : '' }}>▾</span>
                    </div>
                  </div>

                  {expandedIssues.has(idx) && (
                    <div style={{ background: 'var(--bg)', border: '1px solid var(--border)', borderTop: 'none', borderRadius: '0 0 8px 8px', padding: '12px 16px 16px' }}>
                      <div style={{ fontSize: 12, color: 'var(--fg)', lineHeight: 1.5, marginBottom: 12 }}>{issue.description}</div>
                      <div style={{ fontSize: 12, fontWeight: 600, color: '#f97316', marginBottom: 4 }}>Suggestion</div>
                      <div style={{ fontSize: 12, color: 'var(--muted)', lineHeight: 1.5 }}>{issue.suggestion}</div>

                      {issue.affected_items.length > 0 && (
                        <>
                          <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--fg)', marginTop: 12, marginBottom: 4 }}>Affected Items</div>
                          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
                            {issue.affected_items.slice(0, 15).map(item => (
                              <code key={item} style={{ background: 'var(--card)', padding: '2px 6px', borderRadius: 3, fontSize: 11 }}>{item}</code>
                            ))}
                            {issue.affected_items.length > 15 && (
                              <span style={{ fontSize: 11, color: 'var(--muted)' }}>…and {issue.affected_items.length - 15} more</span>
                            )}
                          </div>
                        </>
                      )}
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </>
      )}
    </div>
  )
}
