import { useState, useMemo } from 'react';
import { useAnalyticsAggregate, useAnalyticsCompare } from './queries/analytics';
import { useModbusDevices } from './queries/modbus';
import { btnStyle } from './styles';
import { comparePeriods } from './analyticsCompare';

const RANGES: { label: string; seconds: number }[] = [
  { label: '1h', seconds: 3600 },
  { label: '6h', seconds: 21600 },
  { label: '24h', seconds: 86400 },
  { label: '7d', seconds: 604800 },
  { label: '30d', seconds: 2592000 },
];

const INTERVALS: Record<string, number> = {
  '1h': 60,
  '6h': 300,
  '24h': 3600,
  '7d': 21600,
  '30d': 86400,
};

function fmt(v: number, decimals = 1) {
  if (Number.isNaN(v) || !Number.isFinite(v)) return '--';
  return v.toFixed(decimals);
}

function fmtTs(ts: number) {
  return new Date(ts * 1000).toLocaleString();
}

function exportCsv(data: [number, { min: number; max: number; avg: number; count: number }][], metric: string) {
  const header = 'timestamp,min,max,avg,count,metric\n';
  const rows = data.map(([ts, m]) =>
    `${ts},${m.min},${m.max},${m.avg},${m.count},${metric}`
  ).join('\n');
  const blob = new Blob([header + rows], { type: 'text/csv' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = `analytics_${metric}_${Date.now()}.csv`;
  a.click();
  URL.revokeObjectURL(url);
}

export function AnalyticsPanel() {
  const { data: devices } = useModbusDevices();
  const [deviceId, setDeviceId] = useState('');
  const [metric, setMetric] = useState('');
  const [rangeIdx, setRangeIdx] = useState(2); // 24h
  const [mode, setMode] = useState<'aggregate' | 'compare'>('aggregate');

  const now = Math.floor(Date.now() / 1000);
  const range = RANGES[rangeIdx];
  const from = now - range.seconds;
  const to = now;
  const interval = INTERVALS[range.label] || 3600;

  // Comparison: the recent half is "current", the older half "previous"
  const { current, previous } = comparePeriods(from, to);

  const { data: aggData, isLoading: aggLoading } = useAnalyticsAggregate(
    deviceId || undefined,
    metric || undefined,
    from,
    to,
    interval,
  );

  const { data: compData, isLoading: compLoading } = useAnalyticsCompare(
    deviceId || undefined,
    metric || undefined,
    current[0],
    current[1],
    previous[0],
    previous[1],
  );

  const metrics = useMemo(() => {
    if (!deviceId) return [];
    const dev = devices?.find(d => d.id === deviceId);
    if (!dev) return [];
    // Common metrics for modbus devices - derived from registers
    return []; // Will be populated from available metrics
  }, [deviceId, devices]);

  // Compute overall stats from aggregated data
  const overallStats = useMemo(() => {
    if (!aggData || aggData.length === 0) return null;
    let min = Infinity, max = -Infinity, sum = 0, count = 0;
    for (const [, m] of aggData) {
      if (m.min < min) min = m.min;
      if (m.max > max) max = m.max;
      sum += m.avg * m.count;
      count += m.count;
    }
    return {
      min,
      max,
      avg: count > 0 ? sum / count : 0,
      count,
      dataPoints: aggData.length,
    };
  }, [aggData]);

  const sparklinePoints = useMemo(() => {
    if (!aggData || aggData.length === 0) return '';
    const values = aggData.map(([, m]) => m.avg);
    if (values.length < 2) return '';
    const mn = Math.min(...values);
    const mx = Math.max(...values);
    const range = mx - mn || 1;
    const w = 200;
    const h = 40;
    const pts = values.map((v, i) => {
      const x = (i / (values.length - 1)) * w;
      const y = h - ((v - mn) / range) * h;
      return `${x},${y}`;
    }).join(' ');
    return `<polyline points="${pts}" fill="none" stroke="var(--cyan)" stroke-width="1.5" />`;
  }, [aggData]);

  return (
    <div style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 16 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
        <h2 style={{ margin: 0 }}>Analytics</h2>

        <select
          value={deviceId}
          onChange={e => { setDeviceId(e.target.value); setMetric(''); }}
          style={selectStyle}
        >
          <option value="">Select device...</option>
          {devices?.map(d => (
            <option key={d.id} value={d.id}>{d.name || d.id}</option>
          ))}
        </select>

        <input
          placeholder="Metric name (e.g. voltage)"
          value={metric}
          onChange={e => setMetric(e.target.value)}
          style={inputStyle}
        />

        <div style={{ display: 'flex', gap: 4 }}>
          {RANGES.map((r, i) => (
            <button
              key={r.label}
              onClick={() => setRangeIdx(i)}
              style={{
                ...btnStyle,
                padding: '4px 12px',
                fontSize: 12,
                background: rangeIdx === i ? 'rgba(34,197,94,.15)' : 'transparent',
                color: rangeIdx === i ? 'var(--cyan)' : 'var(--muted)',
                border: rangeIdx === i ? '1px solid var(--cyan)' : '1px solid var(--border)',
              }}
            >
              {r.label}
            </button>
          ))}
        </div>

        <div style={{ display: 'flex', gap: 4 }}>
          <button
            onClick={() => setMode('aggregate')}
            style={{
              ...btnStyle,
              padding: '4px 12px',
              fontSize: 12,
              background: mode === 'aggregate' ? 'rgba(34,197,94,.15)' : 'transparent',
              color: mode === 'aggregate' ? 'var(--cyan)' : 'var(--muted)',
              border: mode === 'aggregate' ? '1px solid var(--cyan)' : '1px solid var(--border)',
            }}
          >
            Aggregate
          </button>
          <button
            onClick={() => setMode('compare')}
            style={{
              ...btnStyle,
              padding: '4px 12px',
              fontSize: 12,
              background: mode === 'compare' ? 'rgba(34,197,94,.15)' : 'transparent',
              color: mode === 'compare' ? 'var(--cyan)' : 'var(--muted)',
              border: mode === 'compare' ? '1px solid var(--cyan)' : '1px solid var(--border)',
            }}
          >
            Compare
          </button>
        </div>
      </div>

      {!deviceId || !metric ? (
        <p style={{ color: 'var(--muted)', textAlign: 'center', marginTop: 40 }}>
          Select a device and enter a metric name to view analytics.
        </p>
      ) : mode === 'aggregate' ? (
        <>
          {aggLoading && <p style={{ color: 'var(--muted)' }}>Loading...</p>}
          {overallStats && (
            <div style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fill, minmax(140px, 1fr))',
              gap: 12,
            }}>
              <StatCard label="Min" value={fmt(overallStats.min)} />
              <StatCard label="Max" value={fmt(overallStats.max)} />
              <StatCard label="Average" value={fmt(overallStats.avg)} />
              <StatCard label="Samples" value={String(overallStats.count)} />
              <StatCard label="Buckets" value={String(overallStats.dataPoints)} />
            </div>
          )}
          {sparklinePoints && (
            <div style={{
              background: 'var(--card)',
              border: '1px solid var(--border)',
              borderRadius: 8,
              padding: 16,
            }}>
              <svg width="100%" height="40" viewBox={`0 0 200 40`} preserveAspectRatio="none">
                {sparklinePoints}
              </svg>
              <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11, color: 'var(--muted)', marginTop: 4 }}>
                <span>{fmtTs(from)}</span>
                <span>{fmtTs(to)}</span>
              </div>
            </div>
          )}
          {aggData && aggData.length > 0 && (
            <button
              onClick={() => exportCsv(aggData, metric)}
              style={{ ...btnStyle, padding: '6px 16px', fontSize: 12 }}
            >
              Export CSV
            </button>
          )}
        </>
      ) : (
        <>
          {compLoading && <p style={{ color: 'var(--muted)' }}>Loading...</p>}
          {compData && (
            <div style={{
              background: 'var(--card)',
              border: '1px solid var(--border)',
              borderRadius: 8,
              padding: 16,
            }}>
              <h3 style={{ margin: '0 0 16px', fontSize: 14 }}>Period Comparison</h3>
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
                <div>
                  <div style={{ fontSize: 11, color: 'var(--muted)', marginBottom: 8 }}>
                    Current ({fmtTs(compData.current.first_ts)} - {fmtTs(compData.current.last_ts)})
                  </div>
                  <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
                    <div><span style={{ color: 'var(--muted)', fontSize: 11 }}>Avg</span><br />{fmt(compData.current.avg)}</div>
                    <div><span style={{ color: 'var(--muted)', fontSize: 11 }}>Min</span><br />{fmt(compData.current.min)}</div>
                    <div><span style={{ color: 'var(--muted)', fontSize: 11 }}>Max</span><br />{fmt(compData.current.max)}</div>
                    <div><span style={{ color: 'var(--muted)', fontSize: 11 }}>Count</span><br />{compData.current.count}</div>
                  </div>
                </div>
                <div>
                  <div style={{ fontSize: 11, color: 'var(--muted)', marginBottom: 8 }}>
                    Previous ({fmtTs(compData.previous.first_ts)} - {fmtTs(compData.previous.last_ts)})
                  </div>
                  <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
                    <div><span style={{ color: 'var(--muted)', fontSize: 11 }}>Avg</span><br />{fmt(compData.previous.avg)}</div>
                    <div><span style={{ color: 'var(--muted)', fontSize: 11 }}>Min</span><br />{fmt(compData.previous.min)}</div>
                    <div><span style={{ color: 'var(--muted)', fontSize: 11 }}>Max</span><br />{fmt(compData.previous.max)}</div>
                    <div><span style={{ color: 'var(--muted)', fontSize: 11 }}>Count</span><br />{compData.previous.count}</div>
                  </div>
                </div>
              </div>
              <div style={{
                marginTop: 16, padding: '8px 12px', borderRadius: 6,
                background: compData.change_percent >= 0 ? 'rgba(34,197,94,.1)' : 'rgba(239,68,68,.1)',
                textAlign: 'center', fontSize: 14, fontWeight: 600,
                color: compData.change_percent >= 0 ? 'var(--ok)' : 'var(--crit)',
              }}>
                {compData.change_percent >= 0 ? '+' : ''}{fmt(compData.change_percent)}% change
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}

function StatCard({ label, value }: { label: string; value: string }) {
  return (
    <div style={{
      background: 'var(--card)',
      border: '1px solid var(--border)',
      borderRadius: 8,
      padding: '12px 16px',
    }}>
      <div style={{ fontSize: 11, color: 'var(--muted)', marginBottom: 4 }}>{label}</div>
      <div style={{ fontSize: 18, fontWeight: 600 }}>{value}</div>
    </div>
  );
}

const selectStyle: React.CSSProperties = {
  background: 'var(--card)',
  border: '1px solid var(--border)',
  borderRadius: 6,
  padding: '6px 12px',
  color: 'var(--fg)',
  fontSize: 13,
};

const inputStyle: React.CSSProperties = {
  background: 'var(--card)',
  border: '1px solid var(--border)',
  borderRadius: 6,
  padding: '6px 12px',
  color: 'var(--fg)',
  fontSize: 13,
  width: 200,
};
