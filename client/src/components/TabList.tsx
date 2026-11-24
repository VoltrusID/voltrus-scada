import { useRef, type ReactNode } from 'react'

export interface TabDef<T extends string> {
  id: T
  label: ReactNode
  disabled?: boolean
}

interface Props<T extends string> {
  tabs: TabDef<T>[]
  value: T
  onChange: (id: T) => void
  /** Names the tab list for screen readers. */
  label: string
  /** Prefix for tab and panel ids; panels use `${idPrefix}-panel-${id}`. */
  idPrefix: string
  accent?: string
}

/**
 * WAI-ARIA tabs: real buttons, one tab stop (the selected tab), arrow keys,
 * Home and End move between tabs. Render the content in a TabPanel with the same prefix.
 */
export function TabList<T extends string>({ tabs, value, onChange, label, idPrefix, accent = '#f97316' }: Props<T>) {
  const refs = useRef<(HTMLButtonElement | null)[]>([])
  const enabled = tabs.map((t, i) => (t.disabled ? -1 : i)).filter(i => i >= 0)

  const onKeyDown = (e: React.KeyboardEvent, index: number) => {
    const pos = enabled.indexOf(index)
    let next: number | undefined
    if (e.key === 'ArrowRight') next = enabled[(pos + 1) % enabled.length]
    else if (e.key === 'ArrowLeft') next = enabled[(pos - 1 + enabled.length) % enabled.length]
    else if (e.key === 'Home') next = enabled[0]
    else if (e.key === 'End') next = enabled[enabled.length - 1]
    if (next === undefined) return
    e.preventDefault()
    refs.current[next]?.focus()
    onChange(tabs[next].id)
  }

  return (
    <div role="tablist" aria-label={label} style={{ display: 'flex', flexWrap: 'wrap', gap: 0, borderBottom: '1px solid var(--border)', marginBottom: 16 }}>
      {tabs.map((t, i) => {
        const active = t.id === value
        return (
          <button
            key={t.id}
            ref={el => { refs.current[i] = el }}
            type="button"
            role="tab"
            id={`${idPrefix}-tab-${t.id}`}
            aria-selected={active}
            aria-controls={`${idPrefix}-panel-${t.id}`}
            aria-disabled={t.disabled || undefined}
            tabIndex={active ? 0 : -1}
            onClick={() => { if (!t.disabled) onChange(t.id) }}
            onKeyDown={e => onKeyDown(e, i)}
            style={{
              padding: '8px 16px', fontSize: 13, background: 'none', border: 'none',
              cursor: t.disabled ? 'default' : 'pointer', opacity: t.disabled ? 0.5 : 1,
              color: active ? 'var(--fg)' : 'var(--muted)', fontFamily: 'inherit',
              borderBottom: active ? `2px solid ${accent}` : '2px solid transparent',
            }}
          >
            {t.label}
          </button>
        )
      })}
    </div>
  )
}

export function TabPanel({ idPrefix, id, children }: { idPrefix: string; id: string; children: ReactNode }) {
  return (
    <div role="tabpanel" id={`${idPrefix}-panel-${id}`} aria-labelledby={`${idPrefix}-tab-${id}`}>
      {children}
    </div>
  )
}
