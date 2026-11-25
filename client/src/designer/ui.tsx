import React, { useState } from 'react'

// Dense engineering-tool chrome for the Designer, on the app's theme
// variables (index.css), so it follows dark and light like the rest.

export const D = {
  font: 12,
  small: 11,
  row: 24,
  pane: 'var(--card)',
  chrome: 'color-mix(in srgb, var(--card) 70%, var(--bg))',
  line: 'var(--border)',
  accent: 'var(--cyan)',
  accentBg: 'color-mix(in srgb, var(--cyan) 14%, transparent)',
  selBg: 'color-mix(in srgb, var(--cyan) 18%, transparent)',
  bind: '#5aa9e6',
}

export const input: React.CSSProperties = {
  width: '100%', minWidth: 0, height: 22, padding: '0 6px', borderRadius: 3, fontSize: D.font,
  border: `1px solid ${D.line}`, background: 'var(--input-bg)', color: 'var(--fg)', fontFamily: 'inherit',
}

export const mono: React.CSSProperties = { fontFamily: 'var(--font-mono)', fontSize: D.small }

const PATHS: Record<string, React.ReactNode> = {
  undo: <><path d="M9 14 4 9l5-5" /><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11" /></>,
  redo: <><path d="m15 14 5-5-5-5" /><path d="M20 9H9.5a5.5 5.5 0 0 0 0 11H13" /></>,
  save: <><path d="M5 3h11l3 3v15H5z" /><path d="M8 3v6h8V3M8 21v-7h8v7" /></>,
  select: <path d="M4 3l7 17 2.5-7.5L21 10z" />,
  connect: <><circle cx="5" cy="6" r="2.5" /><circle cx="19" cy="18" r="2.5" /><path d="M7.5 6H12v12h4.5" /></>,
  alignLeft: <><path d="M4 3v18" /><rect x="7" y="6" width="10" height="4" /><rect x="7" y="14" width="6" height="4" /></>,
  alignHCenter: <><path d="M12 3v18" /><rect x="6" y="6" width="12" height="4" /><rect x="8" y="14" width="8" height="4" /></>,
  alignRight: <><path d="M20 3v18" /><rect x="7" y="6" width="10" height="4" /><rect x="11" y="14" width="6" height="4" /></>,
  alignTop: <><path d="M3 4h18" /><rect x="6" y="7" width="4" height="10" /><rect x="14" y="7" width="4" height="6" /></>,
  alignVMiddle: <><path d="M3 12h18" /><rect x="6" y="6" width="4" height="12" /><rect x="14" y="8" width="4" height="8" /></>,
  alignBottom: <><path d="M3 20h18" /><rect x="6" y="7" width="4" height="10" /><rect x="14" y="11" width="4" height="6" /></>,
  distH: <><path d="M3 4v16M21 4v16" /><rect x="9" y="8" width="6" height="8" /></>,
  distV: <><path d="M4 3h16M4 21h16" /><rect x="8" y="9" width="8" height="6" /></>,
  sameW: <><path d="M4 12h16M4 12l3-3M4 12l3 3M20 12l-3-3M20 12l-3 3" /></>,
  sameH: <><path d="M12 4v16M12 4l-3 3M12 4l3 3M12 20l-3-3M12 20l3-3" /></>,
  group: <><rect x="3" y="3" width="8" height="8" /><rect x="13" y="13" width="8" height="8" /><path d="M2 2h20v20H2z" strokeDasharray="2 2" /></>,
  ungroup: <><rect x="3" y="3" width="8" height="8" /><rect x="13" y="13" width="8" height="8" /></>,
  lock: <><rect x="5" y="11" width="14" height="10" rx="1" /><path d="M8 11V7a4 4 0 0 1 8 0v4" /></>,
  unlock: <><rect x="5" y="11" width="14" height="10" rx="1" /><path d="M8 11V7a4 4 0 0 1 7.5-2" /></>,
  front: <><rect x="8" y="8" width="12" height="12" fill="currentColor" /><rect x="3" y="3" width="12" height="12" /></>,
  back: <><rect x="3" y="3" width="12" height="12" fill="currentColor" /><rect x="8" y="8" width="12" height="12" /></>,
  forward: <><path d="M12 19V5M6 11l6-6 6 6" /></>,
  backward: <><path d="M12 5v14M6 13l6 6 6-6" /></>,
  grid: <><rect x="3" y="3" width="18" height="18" /><path d="M3 9h18M3 15h18M9 3v18M15 3v18" /></>,
  snap: <><path d="M6 3v7a6 6 0 0 0 12 0V3" /><path d="M6 7h4M14 7h4" /></>,
  ruler: <><path d="M3 17 17 3l4 4L7 21z" /><path d="M7 13l2 2M10 10l2 2M13 7l2 2" /></>,
  zoomIn: <><circle cx="11" cy="11" r="7" /><path d="M21 21l-4.5-4.5M8 11h6M11 8v6" /></>,
  zoomOut: <><circle cx="11" cy="11" r="7" /><path d="M21 21l-4.5-4.5M8 11h6" /></>,
  play: <path d="M7 4v16l13-8z" />,
  stop: <rect x="6" y="6" width="12" height="12" />,
  close: <path d="M6 6l12 12M18 6 6 18" />,
  link: <><path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7" /><path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7" /></>,
  screen: <><rect x="3" y="4" width="18" height="13" rx="1" /><path d="M8 21h8M12 17v4" /></>,
  folder: <path d="M3 6h7l2 2h9v11H3z" />,
  tag: <><path d="M3 12V3h9l9 9-9 9z" /><circle cx="7.5" cy="7.5" r="1.5" /></>,
  faceplate: <><rect x="4" y="3" width="16" height="18" rx="1" /><path d="M8 8h8M8 12h8M8 16h4" /></>,
  script: <><path d="M8 6 3 12l5 6M16 6l5 6-5 6" /></>,
  bell: <><path d="M6 16V11a6 6 0 0 1 12 0v5l2 2H4z" /><path d="M10 20a2 2 0 0 0 4 0" /></>,
  eye: <><path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12z" /><circle cx="12" cy="12" r="3" /></>,
  eyeOff: <><path d="M3 3l18 18" /><path d="M10.6 5.1A10 10 0 0 1 12 5c6 0 10 7 10 7a17 17 0 0 1-3.2 3.9M6.6 6.6C3.8 8.5 2 12 2 12s4 7 10 7a10 10 0 0 0 5.4-1.6" /></>,
  chevron: <path d="M9 6l6 6-6 6" />,
  layers: <><path d="M12 3 2 8l10 5 10-5z" /><path d="M2 13l10 5 10-5" /></>,
  trash: <><path d="M4 7h16M9 7V4h6v3M6 7l1 14h10l1-14" /></>,
  copy: <><rect x="8" y="8" width="12" height="12" /><path d="M16 8V4H4v12h4" /></>,
}

export function Icon({ name, size = 14 }: { name: keyof typeof PATHS | string; size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.8}
      strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" style={{ flexShrink: 0 }}>
      {PATHS[name]}
    </svg>
  )
}

export function ToolBtn({ icon, title, onClick, active, disabled, label, testId }: {
  icon: string; title: string; onClick: () => void; active?: boolean; disabled?: boolean; label?: string; testId?: string
}) {
  return (
    <button type="button" onClick={onClick} title={title} aria-label={title} aria-pressed={active} disabled={disabled} data-testid={testId}
      style={{
        height: 24, minWidth: 24, padding: label ? '0 7px' : 0, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', gap: 5,
        borderRadius: 3, border: `1px solid ${active ? D.accent : 'transparent'}`, background: active ? D.accentBg : 'transparent',
        color: active ? D.accent : 'var(--fg)', opacity: disabled ? 0.32 : 1, cursor: disabled ? 'default' : 'pointer', fontSize: D.small,
        whiteSpace: 'nowrap',
      }}>
      <Icon name={icon} />
      {label && <span>{label}</span>}
    </button>
  )
}

export const Sep = () => <div style={{ width: 1, height: 18, background: D.line, margin: '0 4px', flexShrink: 0 }} />

/** A titled pane with a collapsible body. */
export function Pane({ title, icon, children, actions, grow = 1, collapsible = true, testId }: {
  title: string; icon?: string; children: React.ReactNode; actions?: React.ReactNode; grow?: number; collapsible?: boolean; testId?: string
}) {
  const [open, setOpen] = useState(true)
  return (
    <section data-testid={testId} aria-label={title} style={{
      display: 'flex', flexDirection: 'column', minHeight: open ? 0 : undefined, flex: open ? grow : '0 0 auto',
      borderBottom: `1px solid ${D.line}`,
    }}>
      <header style={{
        height: 26, flexShrink: 0, display: 'flex', alignItems: 'center', gap: 6, padding: '0 6px 0 4px',
        background: D.chrome, borderBottom: open ? `1px solid ${D.line}` : undefined, userSelect: 'none',
      }}>
        <button type="button" onClick={() => collapsible && setOpen(o => !o)} aria-expanded={open} style={{
          display: 'flex', alignItems: 'center', gap: 5, flex: 1, minWidth: 0, background: 'none', border: 'none', color: 'var(--text-dim)',
          cursor: collapsible ? 'pointer' : 'default', fontSize: 10.5, fontWeight: 700, letterSpacing: '.07em', textTransform: 'uppercase', padding: 0,
        }}>
          {collapsible && <span style={{ display: 'inline-flex', transform: open ? 'rotate(90deg)' : undefined, transition: 'transform .12s' }}><Icon name="chevron" size={11} /></span>}
          {icon && <Icon name={icon} size={12} />}
          <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{title}</span>
        </button>
        {actions}
      </header>
      {open && <div style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>{children}</div>}
    </section>
  )
}

/** Section header inside the property grid. */
export function GridSection({ title, children, right }: { title: string; children: React.ReactNode; right?: React.ReactNode }) {
  const [open, setOpen] = useState(true)
  return (
    <div>
      <div onClick={() => setOpen(o => !o)} style={{
        height: 22, display: 'flex', alignItems: 'center', gap: 4, padding: '0 6px', cursor: 'pointer', userSelect: 'none',
        background: D.chrome, borderBottom: `1px solid ${D.line}`, fontSize: D.small, fontWeight: 600, color: 'var(--text-dim)',
      }}>
        <span style={{ display: 'inline-flex', transform: open ? 'rotate(90deg)' : undefined }}><Icon name="chevron" size={10} /></span>
        <span style={{ flex: 1 }}>{title}</span>
        {right}
      </div>
      {open && children}
    </div>
  )
}

/** One property row: name | editor | trailing control. */
export function PropRow({ label, children, trailing, bound, title, testId }: {
  label: string; children: React.ReactNode; trailing?: React.ReactNode; bound?: boolean; title?: string; testId?: string
}) {
  return (
    <div data-testid={testId} title={title} style={{
      display: 'grid', gridTemplateColumns: '38% 1fr auto', alignItems: 'center', minHeight: D.row, gap: 4,
      padding: '1px 4px 1px 10px', borderBottom: `1px solid color-mix(in srgb, ${D.line} 55%, transparent)`,
    }}>
      <span style={{ fontSize: D.font, color: bound ? D.bind : 'var(--text-dim)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{label}</span>
      <div style={{ minWidth: 0, display: 'flex', alignItems: 'center' }}>{children}</div>
      <div style={{ display: 'flex' }}>{trailing}</div>
    </div>
  )
}

export function SmallBtn({ children, onClick, title, danger, disabled, testId }: {
  children: React.ReactNode; onClick: () => void; title?: string; danger?: boolean; disabled?: boolean; testId?: string
}) {
  return (
    <button type="button" onClick={onClick} title={title} disabled={disabled} data-testid={testId} style={{
      height: 22, padding: '0 8px', borderRadius: 3, fontSize: D.small, cursor: disabled ? 'default' : 'pointer', whiteSpace: 'nowrap',
      border: `1px solid ${danger ? 'color-mix(in srgb, var(--crit) 45%, transparent)' : D.line}`, background: 'transparent',
      color: danger ? 'var(--crit)' : 'var(--fg)', opacity: disabled ? 0.4 : 1,
    }}>{children}</button>
  )
}

export interface Choice { label: string; onClick: () => void; danger?: boolean; primary?: boolean; testId?: string }

/**
 * A modal question with a few answers (Save / Discard / Cancel). Escape picks
 * `onCancel`; keys do not reach the Designer while it is open.
 */
export function ChoiceDialog({ title, message, choices, onCancel, testId }: {
  title: string; message: React.ReactNode; choices: Choice[]; onCancel: () => void; testId?: string
}) {
  const first = React.useRef<HTMLButtonElement>(null)
  React.useEffect(() => { first.current?.focus() }, [])
  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Capture phase: the Designer's own shortcuts never see a key while the dialog is open.
      e.stopImmediatePropagation()
      if (e.key === 'Escape') { e.preventDefault(); onCancel() }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [onCancel])
  return (
    <div style={{ position: 'fixed', inset: 0, zIndex: 90, display: 'grid', placeItems: 'center', background: 'rgba(0,0,0,.45)' }}>
      <div role="alertdialog" aria-modal="true" aria-labelledby="designer-dialog-title" data-testid={testId} style={{
        width: 420, maxWidth: 'calc(100vw - 32px)', padding: 16, borderRadius: 6, background: 'var(--card)', color: 'var(--fg)',
        border: `1px solid ${D.line}`, boxShadow: '0 12px 40px rgba(0,0,0,.5)', display: 'flex', flexDirection: 'column', gap: 12, fontSize: D.font,
      }}>
        <div id="designer-dialog-title" style={{ fontSize: 14, fontWeight: 700 }}>{title}</div>
        <div style={{ lineHeight: 1.5, color: 'var(--text-dim)' }}>{message}</div>
        <div style={{ display: 'flex', gap: 6, justifyContent: 'flex-end', flexWrap: 'wrap' }}>
          {choices.map((c, i) => (
            <button key={c.label} ref={i === 0 ? first : undefined} type="button" onClick={c.onClick} data-testid={c.testId} style={{
              height: 26, padding: '0 12px', borderRadius: 3, cursor: 'pointer', fontSize: D.font, fontWeight: c.primary ? 600 : 400,
              border: `1px solid ${c.danger ? 'color-mix(in srgb, var(--crit) 45%, transparent)' : c.primary ? D.accent : D.line}`,
              background: c.primary ? D.accentBg : 'transparent', color: c.danger ? 'var(--crit)' : c.primary ? D.accent : 'var(--fg)',
            }}>{c.label}</button>
          ))}
        </div>
      </div>
    </div>
  )
}
