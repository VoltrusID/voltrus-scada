import { useState } from 'react';
import { inputStyle, labelStyle, labelTextStyle, btnStyle } from './styles';

function toLocalDatetime(ts: number) {
  const d = new Date(ts);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function CustomRangePicker({ onApply, onClose }: { onApply: (range: string) => void; onClose: () => void }) {
  const now = Date.now();
  const [start, setStart] = useState(toLocalDatetime(now - 30 * 60 * 1000));
  const [end, setEnd] = useState(toLocalDatetime(now));

  const apply = () => {
    const startMs = new Date(start).getTime();
    const endMs = new Date(end).getTime();
    if (isNaN(startMs) || isNaN(endMs) || startMs >= endMs) return;
    onApply(`${startMs}-${endMs}`);
    onClose();
  };

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.6)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 60 }} onClick={onClose}>
      <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, width: 360, padding: 20 }} onClick={e => e.stopPropagation()}>
        <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 16 }}>Custom Time Range</div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <label style={labelStyle}>
            <span style={labelTextStyle}>Start</span>
            <input type="datetime-local" value={start} onChange={e => setStart(e.target.value)} style={inputStyle} />
          </label>
          <label style={labelStyle}>
            <span style={labelTextStyle}>End</span>
            <input type="datetime-local" value={end} onChange={e => setEnd(e.target.value)} style={inputStyle} />
          </label>
        </div>
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end', marginTop: 16 }}>
          <button onClick={onClose} style={{ padding: '7px 16px', borderRadius: 6, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: 'pointer', fontSize: 13 }}>Cancel</button>
          <button onClick={apply} style={{ padding: '7px 16px', borderRadius: 6, border: '1px solid var(--warn)', background: 'transparent', color: 'var(--warn)', cursor: 'pointer', fontSize: 13, fontWeight: 600 }}>Apply</button>
        </div>
      </div>
    </div>
  );
}

function formatCustomRange(range: string) {
  const [s, e] = range.split('-').map(Number);
  if (isNaN(s) || isNaN(e)) return range;
  const fmt = (ts: number) => new Date(ts).toLocaleString('en-US', { hour12: false, month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
  return `${fmt(s)} – ${fmt(e)}`;
}

function getIntervalMs(range: string, detail?: 'auto' | 'full'): number {
  const isFull = detail === 'full';
  // Parse duration in seconds
  let durSec: number;
  const parts = range.split('-');
  if (parts.length === 2) {
    const s = Number(parts[0]), e = Number(parts[1]);
    durSec = (!isNaN(s) && !isNaN(e) && e > s) ? (e - s) / 1000 : 1800;
  } else {
    const m: Record<string, number> = { '30m': 1800, '1h': 3600, '6h': 21600, '24h': 86400, '7d': 604800, '30d': 2592000 };
    durSec = m[range] ?? 1800;
  }
  if (isFull) return 1000;
  const presets: Record<string, number> = { '30m': 10000, '1h': 10000, '6h': 120000, '24h': 480000, '7d': 3360000, '30d': 14400000 };
  if (presets[range]) return presets[range];
  return Math.max(1000, Math.min(3600000, durSec * 1000 / 300));
}

function formatPts(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M';
  if (n >= 1000) return (n / 1000).toFixed(n >= 10_000 ? 0 : 1) + 'K';
  return String(n);
}

function fmtInterval(ms: number): string {
  if (ms < 1000) return ms + 'ms';
  if (ms < 60000) return (ms / 1000) + 's';
  if (ms < 3600000) return (ms / 60000) + 'm';
  return (ms / 3600000) + 'h';
}

const aggLabels: Record<string, string> = { sampling: 'Sampling', avg: 'Avg', minmax: 'Min/Max/Avg' };

export function HistoryRangeBar({ range, onChange, detail, onDetailChange, agg }: { range: string; onChange: (r: string) => void; detail?: 'auto' | 'full'; onDetailChange?: (d: 'auto' | 'full') => void; agg?: string }) {
  const [showCustom, setShowCustom] = useState(false);
  const presets = ['30m', '1h', '6h', '24h'] as const;
  const isCustom = !presets.includes(range as any);

  const intervalMs = getIntervalMs(range, detail);
  // Parse duration seconds for point count
  let durSec: number;
  const parts = range.split('-');
  if (parts.length === 2) {
    const s = Number(parts[0]), e = Number(parts[1]);
    durSec = (!isNaN(s) && !isNaN(e) && e > s) ? (e - s) / 1000 : 1800;
  } else {
    const m: Record<string, number> = { '30m': 1800, '1h': 3600, '6h': 21600, '24h': 86400, '7d': 604800, '30d': 2592000 };
    durSec = m[range] ?? 1800;
  }
  const pts = Math.round(durSec * 1000 / intervalMs);
  const isFull = detail === 'full';
  const aggName = aggLabels[agg ?? 'sampling'] ?? 'Sampling';
  const label = isFull
    ? `${formatPts(pts)} pts · 1s res`
    : `${formatPts(pts)} pts · ${aggName} / ${fmtInterval(intervalMs)}`;

  return (
    <div style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
      {presets.map(r => (
        <button key={r} style={{ ...btnStyle, padding: '6px 10px', fontSize: 12, borderColor: range === r ? 'var(--warn)' : 'var(--border)', color: range === r ? 'var(--warn)' : 'var(--fg)' }} onClick={() => onChange(r)}>{r}</button>
      ))}
      <button style={{ ...btnStyle, padding: '6px 10px', fontSize: 12, borderColor: isCustom ? 'var(--warn)' : 'var(--border)', color: isCustom ? 'var(--warn)' : 'var(--fg)' }} onClick={() => setShowCustom(true)}>
        Custom
      </button>
      {isCustom && (
        <span style={{ fontSize: 11, color: 'var(--warn)', fontFamily: "var(--font-mono)", whiteSpace: 'nowrap' }}>{formatCustomRange(range)}</span>
      )}
      <div style={{ marginLeft: 4, borderLeft: '1px solid var(--border)', paddingLeft: 4, display: 'flex', gap: 2 }}>
        <button title="Aggregated · ~300 points" style={{ ...btnStyle, padding: '6px 8px', fontSize: 11, borderColor: detail === 'auto' ? 'var(--warn)' : 'var(--border)', color: detail === 'auto' ? 'var(--warn)' : 'var(--fg-muted)' }} onClick={() => onDetailChange?.('auto')}>Auto</button>
        <button title="Every second · no data loss" style={{ ...btnStyle, padding: '6px 8px', fontSize: 11, borderColor: detail === 'full' ? 'var(--warn)' : 'var(--border)', color: detail === 'full' ? 'var(--warn)' : 'var(--fg-muted)' }} onClick={() => onDetailChange?.('full')}>Full</button>
      </div>
      {showCustom && <CustomRangePicker onApply={onChange} onClose={() => setShowCustom(false)} />}
    </div>
  );
}
