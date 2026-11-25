import React, { useEffect, useMemo } from 'react'
import type { SensorConfig } from '../types'
import { useScreenDiff, type DesignVersion } from '../queries/designHistory'
import {
  OUTLINE_COLORS, diffHunks, diffOutlines, diffStats, diffSummary, lineDiff, objectLabel, splitLines, stateLabel, type Outline,
} from './historyModel'
import { StageView } from './StageView'
import { D, DialogBtn, Icon, ToolBtn, mono } from './ui'

/** A version id among the two sides (the one actions apply to), if any. */
function versionSide(from: string, to: string): string | null {
  const isVersion = (s: string) => s !== 'draft' && s !== 'published' && !s.startsWith('seq:')
  return isVersion(to) ? to : isVersion(from) ? from : null
}

/**
 * Two states of the screen side by side with added (green), removed (red)
 * and changed (amber) outlines, the objects that differ with their changed
 * fields, and a line diff of both states as project YAML (tags by ref).
 */
export function CompareView({ screen, from, to, versions, sensors, canConfigure, publishing, onClose, onRevertTo, onPublishVersion }: {
  screen: string
  from: string
  to: string
  versions: DesignVersion[]
  sensors: SensorConfig[]
  canConfigure: boolean
  publishing: boolean
  onClose: () => void
  onRevertTo: (spec: string, label: string) => void
  onPublishVersion: (versionId: string, label: string) => void
}) {
  const q = useScreenDiff(screen, from, to, true)
  const d = q.data
  const fromLabel = stateLabel(from, versions)
  const toLabel = stateLabel(to, versions)
  const outlines = useMemo(() => (d ? diffOutlines(d.diff) : { from: {}, to: {} }), [d])
  const yamlLines = useMemo(() => (d?.yaml ? lineDiff(splitLines(d.yaml.from), splitLines(d.yaml.to)) : []), [d])
  const hunks = useMemo(() => diffHunks(yamlLines, 3), [yamlLines])
  const stats = diffStats(yamlLines)
  const target = versionSide(from, to)
  const targetLabel = target ? stateLabel(target, versions) : ''
  const targetVersion = target ? versions.find(v => String(v.id) === target || v.name === target) : undefined

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.preventDefault(); onClose() }
      // The Designer's shortcuts do not act on the screen behind.
      e.stopPropagation()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [onClose])

  const objects = useMemo(() => {
    if (!d) return []
    const w = d.diff.widgets, c = d.diff.connections
    type Row = { id: string; kind: Outline; label: string; type: 'widget' | 'connection'; fields: string[] }
    const rows: Row[] = [
      ...w.added.map(r => ({ id: r.equipment_id, kind: 'added' as const, label: objectLabel(r, r.equipment_id), type: 'widget' as const, fields: [] })),
      ...w.removed.map(r => ({ id: r.equipment_id, kind: 'removed' as const, label: objectLabel(r, r.equipment_id), type: 'widget' as const, fields: [] })),
      ...w.changed.map(x => ({ id: x.id, kind: 'changed' as const, label: objectLabel(x.after, x.id), type: 'widget' as const, fields: x.fields })),
      ...c.added.map(r => ({ id: r.connection_id, kind: 'added' as const, label: `${r.from_equipment} → ${r.to_equipment}`, type: 'connection' as const, fields: [] })),
      ...c.removed.map(r => ({ id: r.connection_id, kind: 'removed' as const, label: `${r.from_equipment} → ${r.to_equipment}`, type: 'connection' as const, fields: [] })),
      ...c.changed.map(x => ({ id: x.id, kind: 'changed' as const, label: `${x.after.from_equipment} → ${x.after.to_equipment}`, type: 'connection' as const, fields: x.fields })),
    ]
    return rows
  }, [d])

  return (
    <div role="dialog" aria-modal="true" aria-label={`Compare ${fromLabel} with ${toLabel}`} data-testid="designer-compare" style={{
      position: 'fixed', inset: 0, zIndex: 80, display: 'flex', flexDirection: 'column', background: 'var(--bg)', color: 'var(--fg)', fontSize: D.font,
    }}>
      <div style={{ height: 38, flexShrink: 0, display: 'flex', alignItems: 'center', gap: 10, padding: '0 8px 0 12px', borderBottom: `1px solid ${D.line}`, background: D.pane }}>
        <Icon name="compare" />
        <b>Compare</b>
        <span data-testid="designer-compare-title"><b>{fromLabel}</b> → <b>{toLabel}</b></span>
        <span data-testid="designer-compare-summary" style={{ color: 'var(--text-dim)' }}>{d ? diffSummary(d.diff.summary) : q.isError ? '' : 'Comparing…'}</span>
        <span style={{ display: 'inline-flex', gap: 10, marginLeft: 6, fontSize: D.small, color: 'var(--text-muted)' }}>
          {(['added', 'removed', 'changed'] as Outline[]).map(k => (
            <span key={k} style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
              <span style={{ width: 10, height: 10, border: `2px solid ${OUTLINE_COLORS[k]}`, borderRadius: 2 }} />{k}
            </span>
          ))}
        </span>
        <div style={{ flex: 1 }} />
        {target && canConfigure && (
          <>
            <DialogBtn onClick={() => onRevertTo(target, targetLabel)} testId="designer-compare-revert">Revert screen to {targetLabel}</DialogBtn>
            {publishing && !targetVersion?.current && (
              <DialogBtn primary onClick={() => onPublishVersion(target, targetLabel)} testId="designer-compare-publish">Publish {targetLabel}</DialogBtn>
            )}
          </>
        )}
        <ToolBtn icon="close" title="Close the comparison (Esc)" onClick={onClose} testId="designer-compare-close" />
      </div>
      {q.isError && <div style={{ padding: 16, color: 'var(--crit)' }}>Not compared: {(q.error as Error)?.message}</div>}
      <div style={{ flex: '0 0 46%', minHeight: 0, display: 'flex', gap: 1, background: D.line, borderBottom: `1px solid ${D.line}` }}>
        {d?.rows ? (
          <>
            <StageView rows={d.rows.from} sensors={sensors} outlines={outlines.from} testId="designer-compare-from" label={fromLabel} />
            <StageView rows={d.rows.to} sensors={sensors} outlines={outlines.to} testId="designer-compare-to" label={toLabel} />
          </>
        ) : (
          <div style={{ flex: 1, display: 'grid', placeItems: 'center', background: 'var(--bg)', color: 'var(--text-muted)' }}>{q.isLoading ? 'Loading both states…' : ''}</div>
        )}
      </div>
      <div style={{ flex: 1, minHeight: 0, display: 'flex' }}>
        <div data-testid="designer-compare-objects" style={{ width: 320, flexShrink: 0, overflow: 'auto', borderRight: `1px solid ${D.line}`, background: D.pane }}>
          <div style={{ padding: '6px 10px', fontSize: 10.5, fontWeight: 700, letterSpacing: '.07em', textTransform: 'uppercase', color: 'var(--text-dim)', borderBottom: `1px solid ${D.line}` }}>
            Objects ({objects.length})
          </div>
          {d && !objects.length && <div style={{ padding: 10, color: 'var(--text-muted)' }}>The two states are the same.</div>}
          {objects.map(o => (
            <div key={`${o.type}:${o.id}`} data-testid="designer-compare-object" data-kind={o.kind} data-object={o.id} style={{
              padding: '5px 10px', borderBottom: `1px solid color-mix(in srgb, ${D.line} 60%, transparent)`, display: 'flex', gap: 6,
            }}>
              <span style={{ width: 8, height: 8, marginTop: 4, borderRadius: 2, flexShrink: 0, background: OUTLINE_COLORS[o.kind] }} />
              <span style={{ minWidth: 0 }}>
                <span style={{ fontWeight: 600 }}>{o.label}</span>
                <span style={{ color: 'var(--text-muted)' }}> {o.type === 'connection' ? 'connection ' : ''}{o.kind}</span>
                {o.fields.length > 0 && <span style={{ display: 'block', ...mono, color: 'var(--text-dim)', wordBreak: 'break-word' }}>{o.fields.join(', ')}</span>}
              </span>
            </div>
          ))}
        </div>
        <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }}>
          <div style={{ padding: '6px 10px', display: 'flex', gap: 10, fontSize: 10.5, fontWeight: 700, letterSpacing: '.07em', textTransform: 'uppercase', color: 'var(--text-dim)', borderBottom: `1px solid ${D.line}`, background: D.pane }}>
            <span>Project YAML</span>
            <span style={{ color: OUTLINE_COLORS.added }}>+{stats.added}</span>
            <span style={{ color: OUTLINE_COLORS.removed }}>−{stats.removed}</span>
          </div>
          <div data-testid="designer-yaml-diff" style={{ flex: 1, minHeight: 0, overflow: 'auto', ...mono, fontSize: 11.5, lineHeight: 1.55, userSelect: 'text' }}>
            {d && !hunks.length && <div style={{ padding: 10, color: 'var(--text-muted)', fontFamily: 'inherit' }}>No differences in the YAML.</div>}
            {hunks.map((h, i) => (
              <div key={i}>
                {h.skippedBefore > 0 && (
                  <div style={{ padding: '1px 10px', color: 'var(--text-muted)', background: D.chrome }}>⋯ {h.skippedBefore} unchanged line{h.skippedBefore === 1 ? '' : 's'}</div>
                )}
                {h.lines.map((l, j) => (
                  <div key={j} data-testid={l.op === 'same' ? undefined : `designer-yaml-${l.op}`} style={{
                    display: 'grid', gridTemplateColumns: '44px 44px 16px 1fr', whiteSpace: 'pre',
                    background: l.op === 'add' ? 'color-mix(in srgb, #22c55e 14%, transparent)' : l.op === 'del' ? 'color-mix(in srgb, #ef4444 14%, transparent)' : undefined,
                  }}>
                    <span style={{ textAlign: 'right', paddingRight: 6, color: 'var(--text-muted)' }}>{l.a ?? ''}</span>
                    <span style={{ textAlign: 'right', paddingRight: 6, color: 'var(--text-muted)' }}>{l.b ?? ''}</span>
                    <span style={{ color: l.op === 'add' ? OUTLINE_COLORS.added : l.op === 'del' ? OUTLINE_COLORS.removed : 'var(--text-muted)' }}>{l.op === 'add' ? '+' : l.op === 'del' ? '−' : ' '}</span>
                    <span>{l.text}</span>
                  </div>
                ))}
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}
