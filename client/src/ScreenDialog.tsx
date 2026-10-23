import { useState, useEffect } from 'react';
import { inputStyle, labelStyle, labelTextStyle, btnStyle, normalizeKeyEvent, displayShortcut } from './styles';

export function ConfirmDialog({ title, message, confirmLabel, onConfirm, onClose }: {
  title: string; message: string; confirmLabel?: string; onConfirm: () => void; onClose: () => void;
}) {
  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.6)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 60 }} onClick={onClose}>
      <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, width: 380, padding: 20 }} onClick={e => e.stopPropagation()}>
        <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 12 }}>{title}</div>
        <div style={{ fontSize: 13, color: 'var(--muted)', marginBottom: 20 }}>{message}</div>
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <button onClick={onClose} style={{ padding: '7px 16px', borderRadius: 6, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: 'pointer', fontSize: 13 }}>Cancel</button>
          <button onClick={() => { onConfirm(); onClose(); }} style={{ padding: '7px 16px', borderRadius: 6, border: '1px solid var(--crit)', background: 'var(--crit)', color: '#fff', cursor: 'pointer', fontSize: 13, fontWeight: 600 }}>{confirmLabel || 'Delete'}</button>
        </div>
      </div>
    </div>
  );
}

export function PromptDialog({ title, label, placeholder, initialValue, shortcutValue, okLabel, onSubmit, onClose }: {
  title: string; label: string; placeholder?: string; initialValue?: string;
  shortcutValue?: string;
  okLabel?: string;
  onSubmit: (value: string, shortcut?: string | null) => void; onClose: () => void;
}) {
  const [value, setValue] = useState(initialValue || '');
  const [recording, setRecording] = useState(false);
  const [capturedShortcut, setCapturedShortcut] = useState<string | null>(shortcutValue ?? null);

  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === 'Escape') {
        setCapturedShortcut(null);
        setRecording(false);
        return;
      }
      if (['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) return;
      setCapturedShortcut(normalizeKeyEvent(e));
      setRecording(false);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [recording]);

  const submit = () => {
    if (!value.trim()) return;
    onSubmit(value.trim(), capturedShortcut);
    onClose();
  };

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.6)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 60 }} onClick={onClose}>
      <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, width: 380, padding: 20 }} onClick={e => e.stopPropagation()}>
        <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 16 }}>{title}</div>
        <label style={labelStyle}>
          <span style={labelTextStyle}>{label}</span>
          <input value={value} onChange={e => setValue(e.target.value)} placeholder={placeholder} style={inputStyle} autoFocus onKeyDown={e => { if (e.key === 'Enter' && value.trim()) submit(); }} />
        </label>
        {shortcutValue !== undefined && (
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 12 }}>
            <span style={{ ...labelTextStyle, minWidth: 55 }}>Shortcut</span>
            <div style={{ display: 'flex', alignItems: 'center', gap: 6, flex: 1 }}>
              {capturedShortcut ? (
                <kbd style={{
                  fontFamily: "var(--font-mono)", fontSize: 12, fontWeight: 600,
                  padding: '4px 8px', borderRadius: 4,
                  background: 'var(--bg)', border: '1px solid var(--border)', color: 'var(--cyan)',
                }}>{displayShortcut(capturedShortcut)}</kbd>
              ) : (
                <span style={{ fontSize: 12, color: 'var(--muted)' }}>None</span>
              )}
              <button
                onClick={() => setRecording(true)}
                style={{
                  padding: '4px 10px', borderRadius: 4, fontSize: 11, cursor: 'pointer',
                  border: recording ? '1px solid var(--cyan)' : '1px solid var(--border)',
                  background: recording ? 'rgba(34,197,94,.1)' : 'var(--card)',
                  color: recording ? 'var(--cyan)' : 'var(--fg)',
                }}
              >{recording ? 'Press a key...' : 'Record'}</button>
              {capturedShortcut && !recording && (
                <button
                  onClick={() => setCapturedShortcut(null)}
                  style={{
                    padding: '4px 8px', borderRadius: 4, fontSize: 11, cursor: 'pointer',
                    border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)',
                  }}
                >Clear</button>
              )}
            </div>
          </div>
        )}
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end', marginTop: 16 }}>
          <button onClick={onClose} style={{ padding: '7px 16px', borderRadius: 6, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: 'pointer', fontSize: 13 }}>Cancel</button>
          <button onClick={submit} style={{ padding: '7px 16px', borderRadius: 6, border: '1px solid var(--cyan)', background: 'transparent', color: 'var(--cyan)', cursor: 'pointer', fontSize: 13, fontWeight: 600 }}>{okLabel || 'OK'}</button>
        </div>
      </div>
    </div>
  );
}
