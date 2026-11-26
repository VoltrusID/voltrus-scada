// The Designer's Mobile pane (docs/designer.md, phase 5): the operator layout
// phones get, generated from the screen, with the engineer's adjustments.
// Hiding, moving and half width are ordinary property edits of
// `config.mobile` (one undo step each, saved and shared like any other edit);
// a component carries the key only while it differs from the generated
// layout.
import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { SensorConfig } from '../types'
import type { WidgetInstance } from '../scada2/widgets/types'
import type { Patch } from './model'
import {
  adjustedCount, generateOperatorLayout, movePatch, resetPatch, setHiddenPatch, setSpanPatch, type MobileItem, type MobileSection,
} from '../scada2/mobile/operatorLayout'
import { OperatorView, SYMBOL_INFO } from '../scada2/mobile/OperatorView'
import { useTrackedCurrentValues, useWsConnected } from '../store/dashboardStore'
import { D, Icon, SmallBtn, mono } from './ui'

export interface MobilePaneProps {
  widgets: WidgetInstance[]
  sensors: SensorConfig[]
  selection: string[]
  onSelect: (ids: string[]) => void
  onPatch: (patches: Record<string, Patch>) => void
  labelOf: (w: WidgetInstance) => string
  readOnly?: boolean
  screenName?: string
}

export function MobilePane({ widgets, sensors, selection, onSelect, onPatch, labelOf, readOnly, screenName }: MobilePaneProps) {
  const { t } = useTranslation()
  const [view, setView] = useState<'arrange' | 'preview'>('arrange')
  const doc = useMemo(() => ({ widgets }), [widgets])
  const layout = useMemo(() => generateOperatorLayout(doc, SYMBOL_INFO), [doc])
  const byId = useMemo(() => new Map(widgets.map(w => [w.id, w])), [widgets])
  const adjusted = adjustedCount(doc)
  const sel = new Set(selection)
  const areaSections = layout.sections.filter(s => s.id != null)

  return (
    <div data-testid="designer-mobile" style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0, fontSize: D.font }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '6px 8px', borderBottom: `1px solid ${D.line}`, flexWrap: 'wrap' }}>
        <div role="radiogroup" aria-label={t('designerMobile.title')} style={{ display: 'inline-flex', border: `1px solid ${D.line}`, borderRadius: 3, overflow: 'hidden' }}>
          {(['arrange', 'preview'] as const).map(v => (
            <button key={v} type="button" role="radio" aria-checked={view === v} data-testid={`designer-mobile-${v}`} onClick={() => setView(v)} style={{
              height: 22, padding: '0 9px', border: 'none', fontSize: D.small, cursor: 'pointer',
              background: view === v ? D.accentBg : 'transparent', color: view === v ? D.accent : 'var(--fg)',
            }}>{t(`designerMobile.${v}`)}</button>
          ))}
        </div>
        <div style={{ flex: 1 }} />
        <span data-testid="designer-mobile-adjusted" style={{ ...mono, color: 'var(--text-muted)' }}>{t('designerMobile.adjusted', { count: adjusted })}</span>
        <SmallBtn testId="designer-mobile-reset" disabled={readOnly || adjusted === 0} title={t('designerMobile.resetHint')} onClick={() => onPatch(resetPatch(doc))}>{t('designerMobile.reset')}</SmallBtn>
      </div>
      {view === 'preview' ? (
        <PhonePreview widgets={widgets} sensors={sensors} screenName={screenName} />
      ) : (
        <div style={{ flex: 1, minHeight: 0, overflowY: 'auto', padding: '4px 0 12px' }}>
          <p style={{ margin: '4px 10px 8px', color: 'var(--text-muted)', lineHeight: 1.45, fontSize: D.small }}>{t('designerMobile.hint')}</p>
          {layout.sections.length === 0 && <p style={{ margin: 10, color: 'var(--text-muted)' }}>{t('designerMobile.empty')}</p>}
          {layout.sections.map(s => (
            <SectionBlock key={s.id ?? '_'} section={s} index={s.id ? areaSections.indexOf(s) : -1} count={areaSections.length}
              byId={byId} sel={sel} labelOf={labelOf} readOnly={readOnly} onSelect={onSelect}
              onPatch={p => onPatch(p)} doc={doc} />
          ))}
        </div>
      )}
    </div>
  )
}

function SectionBlock({ section, index, count, byId, sel, labelOf, readOnly, onSelect, onPatch, doc }: {
  section: MobileSection; index: number; count: number; byId: Map<string, WidgetInstance>; sel: Set<string>
  labelOf: (w: WidgetInstance) => string; readOnly?: boolean; onSelect: (ids: string[]) => void
  onPatch: (p: Record<string, Patch>) => void; doc: { widgets: WidgetInstance[] }
}) {
  const { t } = useTranslation()
  const id = section.id
  return (
    <div data-testid={`designer-mobile-section-${id ?? 'loose'}`} style={{ margin: '4px 6px 8px', border: `1px solid ${D.line}`, borderRadius: 4, opacity: section.hidden ? 0.55 : 1 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 4, height: 26, padding: '0 4px 0 8px', background: D.chrome, borderBottom: `1px solid ${D.line}` }}>
        <button type="button" onClick={() => id && onSelect([id])} disabled={!id} style={{
          flex: 1, minWidth: 0, textAlign: 'left', border: 'none', background: 'none', padding: 0, cursor: id ? 'pointer' : 'default',
          fontSize: 10.5, fontWeight: 700, letterSpacing: '.07em', textTransform: 'uppercase', color: id && sel.has(id) ? D.accent : 'var(--text-dim)',
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>{section.title || (id ? t('designerMobile.untitledArea') : t('designerMobile.outsideAreas'))}</button>
        {id && (
          <>
            <IconToggle testId={`designer-mobile-hide-${id}`} on={!section.hidden} disabled={readOnly}
              label={section.hidden ? t('designerMobile.showSection') : t('designerMobile.hideSection')}
              onClick={() => onPatch(setHiddenPatch(doc, id, !section.hidden, SYMBOL_INFO))} />
            <MoveBtns id={id} disabled={readOnly} up={index > 0} down={index < count - 1} onMove={d => onPatch(movePatch(doc, id, d, SYMBOL_INFO))} />
          </>
        )}
      </div>
      {section.items.map((it, i) => {
        const w = byId.get(it.id)
        if (!w) return null
        return <ItemRow key={it.id} item={it} w={w} first={i === 0} last={i === section.items.length - 1} selected={sel.has(it.id)}
          label={labelOf(w)} readOnly={readOnly} onSelect={() => onSelect([it.id])}
          onHide={hidden => onPatch(setHiddenPatch(doc, it.id, hidden, SYMBOL_INFO))}
          onSpan={span => onPatch(setSpanPatch(doc, it.id, span))}
          onMove={d => onPatch(movePatch(doc, it.id, d, SYMBOL_INFO))} />
      })}
    </div>
  )
}

function ItemRow({ item, w, first, last, selected, label, readOnly, onSelect, onHide, onSpan, onMove }: {
  item: MobileItem; w: WidgetInstance; first: boolean; last: boolean; selected: boolean; label: string; readOnly?: boolean
  onSelect: () => void; onHide: (hidden: boolean) => void; onSpan: (span: 1 | 2) => void; onMove: (d: -1 | 1) => void
}) {
  const { t } = useTranslation()
  return (
    <div data-testid={`designer-mobile-item-${item.id}`} data-hidden={item.hidden ? 'true' : 'false'} style={{
      display: 'flex', alignItems: 'center', gap: 4, height: D.row + 4, padding: '0 4px 0 8px',
      background: selected ? D.selBg : 'transparent', borderTop: first ? 'none' : `1px solid color-mix(in srgb, ${D.line} 60%, transparent)`,
    }}>
      <button type="button" onClick={onSelect} title={w.id} style={{
        flex: 1, minWidth: 0, display: 'flex', alignItems: 'baseline', gap: 6, border: 'none', background: 'none', padding: 0, cursor: 'pointer', textAlign: 'left',
        color: item.hidden ? 'var(--text-muted)' : 'var(--fg)', textDecoration: item.hidden ? 'line-through' : undefined,
      }}>
        <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{label}</span>
        <span style={{ ...mono, fontSize: 10, color: 'var(--text-muted)', flexShrink: 0, textDecoration: 'none' }}>{t(`designerMobile.kind.${item.kind}`)}</span>
        {item.adjusted && <span title={t('designerMobile.adjustedOne')} style={{ width: 6, height: 6, borderRadius: 3, background: D.accent, flexShrink: 0, alignSelf: 'center' }} />}
      </button>
      <button type="button" data-testid={`designer-mobile-span-${item.id}`} disabled={readOnly || item.hidden}
        aria-label={item.span === 1 ? t('designerMobile.full') : t('designerMobile.half')} title={item.span === 1 ? t('designerMobile.full') : t('designerMobile.half')}
        onClick={() => onSpan(item.span === 1 ? 2 : 1)} style={{
          ...mono, width: 30, height: 20, borderRadius: 3, border: `1px solid ${D.line}`, background: item.span === 1 ? D.accentBg : 'transparent',
          color: item.span === 1 ? D.accent : 'var(--text-dim)', cursor: 'pointer', opacity: readOnly || item.hidden ? 0.4 : 1,
        }}>{item.span === 1 ? '½' : '1'}</button>
      <IconToggle testId={`designer-mobile-hide-${item.id}`} on={!item.hidden} disabled={readOnly}
        label={item.hidden ? t('designerMobile.show') : t('designerMobile.hide')} onClick={() => onHide(!item.hidden)} />
      <MoveBtns id={item.id} disabled={readOnly} up={!first} down={!last} onMove={onMove} />
    </div>
  )
}

function IconToggle({ on, label, onClick, disabled, testId }: { on: boolean; label: string; onClick: () => void; disabled?: boolean; testId: string }) {
  return (
    <button type="button" data-testid={testId} aria-label={label} title={label} aria-pressed={!on} disabled={disabled} onClick={onClick} style={{
      width: 22, height: 20, display: 'grid', placeItems: 'center', border: 'none', borderRadius: 3, background: 'transparent', padding: 0,
      color: on ? 'var(--text-dim)' : 'var(--warn)', cursor: disabled ? 'default' : 'pointer', opacity: disabled ? 0.4 : 1,
    }}><Icon name={on ? 'eye' : 'eyeOff'} size={13} /></button>
  )
}

function MoveBtns({ id, up, down, onMove, disabled }: { id: string; up: boolean; down: boolean; onMove: (d: -1 | 1) => void; disabled?: boolean }) {
  const { t } = useTranslation()
  const btn = (d: -1 | 1, enabled: boolean, label: string) => (
    <button type="button" data-testid={`designer-mobile-${d < 0 ? 'up' : 'down'}-${id}`} aria-label={label} title={label} disabled={disabled || !enabled}
      onClick={() => onMove(d)} style={{
        width: 20, height: 20, display: 'grid', placeItems: 'center', border: 'none', borderRadius: 3, background: 'transparent', padding: 0,
        color: 'var(--text-dim)', cursor: disabled || !enabled ? 'default' : 'pointer', opacity: disabled || !enabled ? 0.3 : 1,
      }}>
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
        <path d={d < 0 ? 'M6 15l6-6 6 6' : 'M6 9l6 6 6-6'} />
      </svg>
    </button>
  )
  return <>{btn(-1, up, t('designerMobile.up'))}{btn(1, down, t('designerMobile.down'))}</>
}

/** The operator view in a phone-sized frame, with live values. */
function PhonePreview({ widgets, sensors, screenName }: { widgets: WidgetInstance[]; sensors: SensorConfig[]; screenName?: string }) {
  const values = useTrackedCurrentValues()
  const live = useWsConnected()
  return (
    <div style={{ flex: 1, minHeight: 0, overflow: 'auto', padding: 10, display: 'flex', justifyContent: 'center', background: D.chrome }}>
      <div data-testid="designer-mobile-phone" style={{
        width: 360, maxWidth: '100%', height: 640, maxHeight: '100%', display: 'flex', flexDirection: 'column', overflow: 'hidden',
        border: `6px solid color-mix(in srgb, var(--fg) 18%, transparent)`, borderRadius: 22, background: 'var(--bg)', flexShrink: 0,
      }}>
        <OperatorView widgets={widgets} sensors={sensors} values={values} live={live} screenName={screenName} />
      </div>
    </div>
  )
}
