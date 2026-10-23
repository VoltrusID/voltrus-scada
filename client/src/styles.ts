export const stColor = (s: string) => s === 'critical' ? 'var(--crit)' : s === 'warning' ? 'var(--warn)' : s === 'offline' ? 'var(--muted)' : 'var(--ok)';
export function fmtTime(ms: number) {
  const s = ms / 1000 | 0;
  return [s / 3600 | 0, (s % 3600) / 60 | 0, s % 60].map(v => String(v).padStart(2, '0')).join(':');
}
export function fmtTs(ts: number) {
  return new Date(ts).toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' });
}
export function parseConfig(cfg: string): Record<string, any> { try { return JSON.parse(cfg) } catch { return {} } }

export function safeMinMax(arr: number[]): [number, number] {
  if (!arr.length) return [0, 0];
  let min = Infinity, max = -Infinity;
  for (const v of arr) { if (v < min) min = v; if (v > max) max = v; }
  return [min, max];
}

const _colorCtx = typeof document !== 'undefined' ? document.createElement('canvas').getContext('2d') : null;
export function toHexColor(color: string): string {
  if (!_colorCtx) return color;
  _colorCtx.fillStyle = '#000000';
  _colorCtx.fillStyle = color;
  return _colorCtx.fillStyle;
}

export function normalizeKeyEvent(e: KeyboardEvent): string {
  const parts: string[] = [];
  if (e.ctrlKey || e.metaKey) parts.push('Mod');
  if (e.altKey) parts.push('Alt');
  if (e.shiftKey && e.key.length > 1) parts.push('Shift');
  parts.push(e.key.length === 1 ? e.key.toUpperCase() : e.key);
  return parts.join('+');
}

export function displayShortcut(shortcut: string): string {
  const isMac = navigator.platform.toUpperCase().includes('MAC');
  return shortcut
    .replace(/\bMod\b/g, isMac ? '\u2318' : 'Ctrl')
    .replace(/\bAlt\b/g, isMac ? '\u2325' : 'Alt')
    .replace(/\bShift\b/g, isMac ? '\u21E7' : 'Shift');
}

export const inputStyle: React.CSSProperties = {
  background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 6, padding: '6px 8px',
  color: 'var(--fg)', fontSize: 13, fontFamily: 'inherit', width: '100%', boxSizing: 'border-box'
};
export const monoInputStyle: React.CSSProperties = { ...inputStyle, fontFamily: 'var(--font-mono)' };
export const labelStyle: React.CSSProperties = { display: 'flex', flexDirection: 'column', gap: 4 };
export const labelTextStyle: React.CSSProperties = { fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' as const };
export const tabStyle = (active: boolean): React.CSSProperties => ({
  padding: '10px 18px', fontSize: 12, fontWeight: 600, cursor: 'pointer', border: 'none',
  borderBottom: active ? '2px solid var(--cyan)' : '2px solid transparent',
  color: active ? 'var(--cyan)' : 'var(--muted)', background: 'transparent', textTransform: 'uppercase' as const,
  letterSpacing: '.05em',
  transition: 'color 0.15s, border-color 0.15s',
});
export const btnStyle: React.CSSProperties = {
  fontFamily: 'var(--font-ui)', fontSize: 12, fontWeight: 600, padding: '6px 12px', borderRadius: 6,
  border: '1px solid var(--border)', background: 'var(--card)', color: 'var(--fg)', cursor: 'pointer',
  display: 'flex', alignItems: 'center', gap: 5, transition: 'border-color 0.15s, color 0.15s',
};

export const touchTarget: React.CSSProperties = { minHeight: 44, minWidth: 44 };

export const touchBtnStyle: React.CSSProperties = {
  ...btnStyle,
  minHeight: 44,
  minWidth: 44,
  padding: '10px 18px',
};
