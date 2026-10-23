import { inputStyle, monoInputStyle, labelStyle, labelTextStyle, btnStyle } from './styles';
import { useAlarmCooldown, useUpdateAlarmCooldown } from './queries';
import { ThemeToggle } from './components/ThemeToggle';
import { LanguageSelector } from './components/LanguageSelector';

const aggOptions = [
  { value: 'sampling', label: 'Sampling', desc: 'Pick evenly-spaced points. Fastest, but skips data between intervals.' },
  { value: 'avg', label: 'Average', desc: 'Average all 1s values within each interval bucket. Smooth curve, hides peaks.' },
  { value: 'minmax', label: 'Min/Max/Avg', desc: 'Shows average line with min/max band. Best overview — no data is lost.' },
] as const;

export type HistoryAutoAgg = typeof aggOptions[number]['value'];

export function DashboardSettings({ settings, onChange, onClose, alarmMuted, onToggleMute, kioskMode, onToggleKiosk }: { settings: { gap: number; refreshInterval: number; historyAutoAgg: HistoryAutoAgg }; onChange: (s: any) => void; onClose: () => void; alarmMuted: boolean; onToggleMute: () => void; kioskMode: boolean; onToggleKiosk: () => void }) {
  const { data: cooldownSecs = 300 } = useAlarmCooldown()
  const updateCooldown = useUpdateAlarmCooldown()

  const sectionTitle: React.CSSProperties = {
    fontSize: 11,
    color: 'var(--muted)',
    textTransform: 'uppercase',
    letterSpacing: '.06em',
    fontWeight: 600,
    marginTop: 12,
    marginBottom: 4,
    paddingBottom: 4,
    borderBottom: '1px solid var(--border)',
  }

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.6)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 100 }} onClick={onClose}>
      <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 12, padding: 20, minWidth: 340, maxWidth: 440, maxHeight: '90vh', overflowY: 'auto' }} onClick={e => e.stopPropagation()}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16 }}>
          <span style={{ fontSize: 15, fontWeight: 600 }}>Settings</span>
          <button onClick={onClose} aria-label="Close" style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', padding: 4, display: 'flex', alignItems: 'center', justifyContent: 'center', borderRadius: 4, transition: 'color 0.15s' }} onMouseEnter={e => e.currentTarget.style.color = 'var(--fg)'} onMouseLeave={e => e.currentTarget.style.color = 'var(--muted)'}>
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
          </button>
        </div>

        {/* ── Appearance ── */}
        <div style={sectionTitle}>Appearance</div>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '8px 0' }}>
          <span style={{ fontSize: 13, color: 'var(--fg)' }}>Theme</span>
          <ThemeToggle size={20} />
        </div>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '8px 0' }}>
          <span style={{ fontSize: 13, color: 'var(--fg)' }}>Language</span>
          <LanguageSelector />
        </div>

        {/* ── Dashboard ── */}
        <div style={sectionTitle}>Dashboard</div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <label style={labelStyle}><span style={labelTextStyle}>Grid Gap (px)</span><input type="range" min="4" max="24" value={settings.gap} onChange={e => onChange({ ...settings, gap: parseInt(e.target.value) })} style={{ width: '100%' }} /><span style={{ fontSize: 11, fontFamily: 'var(--font-mono)' }}>{settings.gap}px</span></label>
          <label style={labelStyle}><span style={labelTextStyle}>Refresh Interval (ms)</span><input type="number" step="100" min="100" value={settings.refreshInterval} onChange={e => onChange({ ...settings, refreshInterval: parseInt(e.target.value) || 1000 })} style={monoInputStyle} /></label>
          <div style={labelStyle}>
            <span style={labelTextStyle}>History Auto Mode</span>
            <div style={{ display: 'flex', gap: 4, marginTop: 4 }}>
              {aggOptions.map(opt => (
                <button key={opt.value} onClick={() => onChange({ ...settings, historyAutoAgg: opt.value })} style={{
                  ...btnStyle, flex: 1, padding: '5px 6px', fontSize: 11, fontWeight: settings.historyAutoAgg === opt.value ? 600 : 400, justifyContent: 'center',
                  borderColor: settings.historyAutoAgg === opt.value ? 'var(--warn)' : 'var(--border)',
                  color: settings.historyAutoAgg === opt.value ? 'var(--warn)' : 'var(--fg)',
                  background: settings.historyAutoAgg === opt.value ? 'rgba(234,179,8,.08)' : 'transparent',
                }}>
                  {opt.label}
                </button>
              ))}
            </div>
            <div style={{ fontSize: 11, color: 'var(--fg-muted)', marginTop: 6, lineHeight: 1.4 }}>
              {aggOptions.find(o => o.value === settings.historyAutoAgg)?.desc}
            </div>
          </div>
        </div>

        {/* ── Alarms ── */}
        <div style={sectionTitle}>Alarms</div>
        <label style={labelStyle}>
          <span style={labelTextStyle}>Cooldown (seconds)</span>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8, alignItems: 'center', marginTop: 4 }}>
            <input type="number" step="30" min="0" max="86400" value={cooldownSecs} onChange={e => {
              const val = Math.max(0, Math.min(86400, parseInt(e.target.value) || 0))
              updateCooldown.mutate(val)
            }} style={monoInputStyle} />
            <span style={{ fontSize: 11, color: 'var(--muted)', fontFamily: 'var(--font-mono)' }}>0 = disabled</span>
          </div>
          <div style={{ fontSize: 11, color: 'var(--muted)', marginTop: 4, lineHeight: 1.4 }}>
            After acknowledging an alarm, wait this long before re-triggering.
          </div>
        </label>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '8px 0' }}>
          <span style={{ fontSize: 13, color: 'var(--fg)' }}>Alarm Sound</span>
          <button onClick={onToggleMute} style={{
            padding: '5px 10px', borderRadius: 6, border: `1px solid ${alarmMuted ? 'var(--warn)' : 'var(--border)'}`,
            background: alarmMuted ? 'rgba(234,179,8,.1)' : 'transparent', color: alarmMuted ? 'var(--warn)' : 'var(--fg)',
            cursor: 'pointer', fontSize: 12, fontWeight: 500, display: 'flex', alignItems: 'center', gap: 5,
          }}>
            {alarmMuted ? (
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9"/><path d="M13.73 21a2 2 0 0 1-3.46 0"/><line x1="1" y1="1" x2="23" y2="23" stroke="currentColor" strokeWidth="2"/></svg>
            ) : (
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9"/><path d="M13.73 21a2 2 0 0 1-3.46 0"/></svg>
            )}
            {alarmMuted ? 'Muted' : 'Active'}
          </button>
        </div>

        {/* ── Operator Console ── */}
        <div style={sectionTitle}>Operator Console</div>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '8px 0' }}>
          <div>
            <span style={{ fontSize: 13, color: 'var(--fg)' }}>Kiosk Mode</span>
            <div style={{ fontSize: 11, color: 'var(--muted)', marginTop: 2, lineHeight: 1.4 }}>
              Block right-click, Alt-Tab, and browser shortcuts
            </div>
          </div>
          <button onClick={onToggleKiosk} style={{
            padding: '5px 10px', borderRadius: 6, border: `1px solid ${kioskMode ? 'var(--accent)' : 'var(--border)'}`,
            background: kioskMode ? 'rgba(0,230,118,.1)' : 'transparent', color: kioskMode ? 'var(--accent)' : 'var(--fg)',
            cursor: 'pointer', fontSize: 12, fontWeight: 500, display: 'flex', alignItems: 'center', gap: 5,
          }}>
            {kioskMode ? 'Active' : 'Off'}
          </button>
        </div>
      </div>
    </div>
  );
}
