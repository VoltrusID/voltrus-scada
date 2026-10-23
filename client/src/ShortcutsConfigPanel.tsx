import { useState, useEffect, useRef } from 'react';
import { normalizeKeyEvent, displayShortcut } from './styles';

type NavShortcuts = Record<string, string>;

const NAV_ITEMS = [
  { target: 'view:cards', label: 'Grids' },
  { target: 'view:canvas', label: 'Canvas' },
  { target: 'view:twin', label: '3D Twin' },
  { target: 'view:scada', label: '2D SCADA' },
  { target: 'mode:live', label: 'Live Mode' },
  { target: 'mode:history', label: 'History Mode' },
  { target: 'action:alarms', label: 'Notifications' },
];

function ShortcutRow({ value, onChange }: { value: string; onChange: (v: string | null) => void }) {
  const [recording, setRecording] = useState(false);

  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === 'Escape') {
        onChange(null);
        setRecording(false);
        return;
      }
      if (['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) return;
      onChange(normalizeKeyEvent(e));
      setRecording(false);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [recording, onChange]);

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
      {value ? (
        <kbd style={{
          fontFamily: "var(--font-mono)", fontSize: 11, fontWeight: 600,
          padding: '2px 6px', borderRadius: 3,
          background: 'var(--bg)', border: '1px solid var(--border)', color: 'var(--cyan)',
        }}>{displayShortcut(value)}</kbd>
      ) : (
        <span style={{ fontSize: 11, color: 'var(--muted)' }}>-</span>
      )}
      <button
        onClick={() => setRecording(true)}
        style={{
          padding: '2px 8px', borderRadius: 3, fontSize: 10, cursor: 'pointer',
          border: recording ? '1px solid var(--cyan)' : '1px solid var(--border)',
          background: recording ? 'rgba(34,197,94,.1)' : 'var(--card)',
          color: recording ? 'var(--cyan)' : 'var(--fg)',
        }}
      >{recording ? 'Press...' : 'Set'}</button>
      {value && !recording && (
        <button
          onClick={() => onChange(null)}
          style={{
            padding: '2px 6px', borderRadius: 3, fontSize: 10, cursor: 'pointer',
            border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)',
          }}
        >x</button>
      )}
    </div>
  );
}

export function ShortcutsConfigPanel({ effectiveNavShortcuts, navShortcuts, saveNavShortcutsMut, onClose }: {
  effectiveNavShortcuts: NavShortcuts;
  navShortcuts: NavShortcuts;
  saveNavShortcutsMut: ReturnType<typeof import('./queries').useSaveNavShortcuts>;
  onClose: () => void;
}) {
  const panelRef = useRef<HTMLDivElement>(null);
  const [local, setLocal] = useState<NavShortcuts>(() => {
    const m: NavShortcuts = {};
    NAV_ITEMS.forEach(({ target }) => {
      m[target] = navShortcuts[target] || effectiveNavShortcuts[target] || '';
    });
    return m;
  });

  useEffect(() => {
    const onClick = (e: MouseEvent) => {
      if (panelRef.current && !panelRef.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener('mousedown', onClick);
    return () => document.removeEventListener('mousedown', onClick);
  }, [onClose]);

  const save = async (updated: NavShortcuts) => {
    const merged = { ...navShortcuts };
    Object.entries(updated).forEach(([k, v]) => {
      if (v) merged[k] = v; else delete merged[k];
    });
    await saveNavShortcutsMut.mutateAsync(merged);
  };

  return (
    <div ref={panelRef} style={{
      position: 'absolute', top: '100%', right: 0, marginTop: 4,
      background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 8,
      width: 240, padding: 12, zIndex: 70,
      boxShadow: '0 8px 24px rgba(0,0,0,.3)',
    }}>
      <div style={{ fontSize: 12, fontWeight: 600, marginBottom: 8, color: 'var(--fg)' }}>Navigation Shortcuts</div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        {NAV_ITEMS.map(({ target, label }) => (
          <div key={target} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8 }}>
            <span style={{ fontSize: 11, color: 'var(--fg)', minWidth: 70 }}>{label}</span>
            <ShortcutRow
              value={local[target] || ''}
              onChange={(v) => {
                const updated = { ...local, [target]: v || '' };
                setLocal(updated);
                save(updated);
              }}
            />
          </div>
        ))}
      </div>
    </div>
  );
}
