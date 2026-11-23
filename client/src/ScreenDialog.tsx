import { useState, useEffect } from 'react';
import { inputStyle, labelStyle, labelTextStyle, btnStyle, normalizeKeyEvent, displayShortcut } from './styles';
import { Dialog } from './components/Dialog';
import { useTranslation } from 'react-i18next';

export function ConfirmDialog({ title, message, confirmLabel, onConfirm, onClose }: {
  title: string; message: string; confirmLabel?: string; onConfirm: () => void; onClose: () => void;
}) {
  const { t } = useTranslation();
  return (
    <Dialog title={title} onClose={onClose}>
        <div style={{ fontSize: 13, color: 'var(--muted)', marginBottom: 20 }}>{message}</div>
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <button onClick={onClose} style={{ padding: '7px 16px', borderRadius: 6, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: 'pointer', fontSize: 13 }}>{t('common.cancel')}</button>
          <button onClick={() => { onConfirm(); onClose(); }} style={{ padding: '7px 16px', borderRadius: 6, border: '1px solid var(--crit)', background: 'var(--crit)', color: '#fff', cursor: 'pointer', fontSize: 13, fontWeight: 600 }}>{confirmLabel || t('common.delete')}</button>
        </div>
    </Dialog>
  );
}

export function PromptDialog({ title, label, placeholder, initialValue, shortcutValue, okLabel, onSubmit, onClose }: {
  title: string; label: string; placeholder?: string; initialValue?: string;
  shortcutValue?: string;
  okLabel?: string;
  onSubmit: (value: string, shortcut?: string | null) => void; onClose: () => void;
}) {
  const { t } = useTranslation();
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
    <Dialog title={title} onClose={onClose} trapActive={!recording}>
        <label style={labelStyle}>
          <span style={labelTextStyle}>{label}</span>
          <input value={value} onChange={e => setValue(e.target.value)} placeholder={placeholder} style={inputStyle} autoFocus onKeyDown={e => { if (e.key === 'Enter' && value.trim()) submit(); }} />
        </label>
        {shortcutValue !== undefined && (
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 12 }}>
            <span style={{ ...labelTextStyle, minWidth: 55 }}>{t('dialog.shortcut')}</span>
            <div style={{ display: 'flex', alignItems: 'center', gap: 6, flex: 1 }}>
              {capturedShortcut ? (
                <kbd style={{
                  fontFamily: "var(--font-mono)", fontSize: 12, fontWeight: 600,
                  padding: '4px 8px', borderRadius: 4,
                  background: 'var(--bg)', border: '1px solid var(--border)', color: 'var(--cyan)',
                }}>{displayShortcut(capturedShortcut)}</kbd>
              ) : (
                <span style={{ fontSize: 12, color: 'var(--muted)' }}>{t('dialog.none')}</span>
              )}
              <button
                onClick={() => setRecording(true)}
                style={{
                  padding: '4px 10px', borderRadius: 4, fontSize: 11, cursor: 'pointer',
                  border: recording ? '1px solid var(--cyan)' : '1px solid var(--border)',
                  background: recording ? 'rgba(34,197,94,.1)' : 'var(--card)',
                  color: recording ? 'var(--cyan)' : 'var(--fg)',
                }}
              >{recording ? t('dialog.pressKey') : t('dialog.record')}</button>
              {capturedShortcut && !recording && (
                <button
                  onClick={() => setCapturedShortcut(null)}
                  style={{
                    padding: '4px 8px', borderRadius: 4, fontSize: 11, cursor: 'pointer',
                    border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)',
                  }}
                >{t('dialog.clear')}</button>
              )}
            </div>
          </div>
        )}
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end', marginTop: 16 }}>
          <button onClick={onClose} style={{ padding: '7px 16px', borderRadius: 6, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: 'pointer', fontSize: 13 }}>{t('common.cancel')}</button>
          <button onClick={submit} style={{ padding: '7px 16px', borderRadius: 6, border: '1px solid var(--cyan)', background: 'transparent', color: 'var(--cyan)', cursor: 'pointer', fontSize: 13, fontWeight: 600 }}>{okLabel || t('common.ok')}</button>
        </div>
    </Dialog>
  );
}
