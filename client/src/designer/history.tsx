import React, { useMemo, useState } from 'react'
import { useScreenHistory, type BlameEntry, type DesignChange } from '../queries/designHistory'
import {
  avatarHue, blameLine, changedFields, groupBatches, initials, kindSummary, objectLabel, operationLabel, relativeTime,
  sourceLabel, type Batch,
} from './historyModel'
import { Badge, D, Icon, SmallBtn, mono } from './ui'

/** The object the History pane follows: the selected component or connection. */
export interface HistoryObject { id: string; type: 'widget' | 'connection'; label: string }

const KIND: Record<string, { icon: string; color: string; label: string }> = {
  add: { icon: 'plus', color: '#22c55e', label: 'Added' },
  update: { icon: 'edit', color: '#f59e0b', label: 'Changed' },
  remove: { icon: 'minus', color: '#ef4444', label: 'Removed' },
}

export function Avatar({ name, size = 22 }: { name: string; size?: number }) {
  const hue = avatarHue(name)
  return (
    <span aria-hidden="true" title={name} style={{
      width: size, height: size, borderRadius: size / 2, flexShrink: 0, display: 'inline-grid', placeItems: 'center',
      fontSize: size * 0.42, fontWeight: 700, color: `hsl(${hue} 70% 82%)`, background: `hsl(${hue} 45% 32%)`,
    }}>{initials(name)}</span>
  )
}

/**
 * The change log of the screen, grouped by save: who, from where, when, and
 * what each save changed. Clicking a batch previews the screen as it was
 * after it; its objects can be put back one by one, or the whole screen.
 */
export function HistoryList({ screen, object, wholeScreen, onWholeScreen, previewSeq, onPreview, onRevertObject, onRevertScreen, canRevert }: {
  screen: string
  object: HistoryObject | null
  wholeScreen: boolean
  onWholeScreen: (whole: boolean) => void
  previewSeq: number | null
  onPreview: (b: Batch | null) => void
  /** Put one object back as it was after the batch. */
  onRevertObject: (c: DesignChange, b: Batch) => void
  onRevertScreen: (b: Batch) => void
  canRevert: boolean
}) {
  const [user, setUser] = useState('')
  const [knownUsers, setKnownUsers] = useState<string[]>([])
  const following = !wholeScreen && object ? object : null
  const q = useScreenHistory(screen, { object: following?.id ?? null, objectType: following?.type ?? null, user: user || null })
  const rows = useMemo(() => q.data?.pages.flatMap(p => p.changes) ?? [], [q.data])
  const batches = useMemo(() => groupBatches(rows), [rows])
  // The user filter lists everyone seen so far, so picking one does not shrink the list to them.
  const pageUsers = useMemo(() => q.data?.pages.flatMap(p => p.users ?? []) ?? [], [q.data])
  React.useEffect(() => {
    if (pageUsers.some(u => !knownUsers.includes(u))) setKnownUsers(k => [...new Set([...k, ...pageUsers])].sort())
  }, [pageUsers, knownUsers])
  const [open, setOpen] = useState<string | null>(null)
  const now = Date.now()

  return (
    <div data-testid="designer-history" style={{ display: 'flex', flexDirection: 'column', minHeight: 0, height: '100%' }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4, padding: 6, borderBottom: `1px solid ${D.line}` }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: D.small }}>
          <span style={{ color: 'var(--text-muted)' }}>Showing</span>
          {following ? (
            <span data-testid="designer-history-scope" style={{ fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {following.type === 'connection' ? 'connection ' : ''}{following.label}
            </span>
          ) : (
            <span data-testid="designer-history-scope" style={{ fontWeight: 600 }}>whole screen</span>
          )}
          <div style={{ flex: 1 }} />
          <label style={{ display: 'inline-flex', alignItems: 'center', gap: 4, color: 'var(--text-dim)', cursor: object ? 'pointer' : 'default', opacity: object ? 1 : 0.5 }}
            title={object ? 'Show the changes of every component, not only the selected one' : 'Select a component to see only its changes'}>
            <input type="checkbox" checked={wholeScreen || !object} disabled={!object} onChange={e => onWholeScreen(e.target.checked)} data-testid="designer-history-whole" />
            Whole screen
          </label>
        </div>
        <select value={user} onChange={e => setUser(e.target.value)} aria-label="Filter by user" data-testid="designer-history-user"
          style={{ height: 24, padding: '0 4px', fontSize: D.small, background: 'var(--input-bg)', color: 'var(--fg)', border: `1px solid ${D.line}`, borderRadius: 3 }}>
          <option value="">Everyone</option>
          {knownUsers.map(u => <option key={u} value={u}>{u}</option>)}
        </select>
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>
        {q.isLoading && <div style={{ padding: 10, color: 'var(--text-muted)' }}>Loading history…</div>}
        {q.isError && <div style={{ padding: 10, color: 'var(--crit)' }}>History not loaded: {(q.error as Error)?.message}</div>}
        {q.isSuccess && !batches.length && (
          <div style={{ padding: 10, color: 'var(--text-muted)', lineHeight: 1.5 }}>
            {following ? `No recorded changes of ${following.label}.` : 'No changes recorded yet. Every save from now on appears here.'}
          </div>
        )}
        {batches.map(b => (
          <BatchRow key={b.batch} b={b} now={now} open={open === b.batch} previewing={previewSeq === b.maxSeq} canRevert={canRevert}
            onToggle={() => {
              // Click: open and preview; again: close and back to the draft.
              const closing = open === b.batch && previewSeq === b.maxSeq
              setOpen(closing ? null : b.batch)
              onPreview(closing ? null : b)
            }}
            onRevertObject={onRevertObject} onRevertScreen={() => onRevertScreen(b)} />
        ))}
        {q.hasNextPage && (
          <div style={{ padding: 8, display: 'flex', justifyContent: 'center' }}>
            <SmallBtn onClick={() => { void q.fetchNextPage() }} disabled={q.isFetchingNextPage} testId="designer-history-more">
              {q.isFetchingNextPage ? 'Loading…' : 'Load older changes'}
            </SmallBtn>
          </div>
        )}
      </div>
    </div>
  )
}

function BatchRow({ b, now, open, previewing, canRevert, onToggle, onRevertObject, onRevertScreen }: {
  b: Batch; now: number; open: boolean; previewing: boolean; canRevert: boolean
  onToggle: () => void; onRevertObject: (c: DesignChange, b: Batch) => void; onRevertScreen: () => void
}) {
  const what = b.changes.length === 1
    ? `${KIND[b.changes[0].kind]?.label ?? b.changes[0].kind} ${objectLabel(b.changes[0].after ?? b.changes[0].before, b.changes[0].object_id)}`
    : kindSummary(b.changes.map(c => c.kind))
  return (
    <div data-testid="designer-history-batch" data-batch={b.batch} data-seq={b.maxSeq} style={{
      borderBottom: `1px solid color-mix(in srgb, ${D.line} 60%, transparent)`, background: previewing ? D.selBg : undefined,
    }}>
      <button type="button" onClick={onToggle} aria-expanded={open} title={`Preview the screen after change #${b.maxSeq}`} style={{
        display: 'flex', width: '100%', gap: 7, alignItems: 'flex-start', padding: '6px 8px', border: 'none', background: 'transparent',
        color: 'var(--fg)', cursor: 'pointer', textAlign: 'left', fontSize: D.font,
      }}>
        <Avatar name={b.username} />
        <span style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 2 }}>
          <span style={{ display: 'flex', alignItems: 'center', gap: 5, minWidth: 0 }}>
            <b style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{b.username}</b>
            <Badge testId="designer-history-source">{sourceLabel(b.source)}</Badge>
            {b.operation !== 'save' && <Badge color="var(--cyan)">{operationLabel(b.operation)}</Badge>}
            <span style={{ flex: 1 }} />
            <span title={new Date(b.at_ms).toLocaleString()} style={{ ...mono, color: 'var(--text-muted)', whiteSpace: 'nowrap' }}>{relativeTime(b.at_ms, now)}</span>
          </span>
          <span style={{ color: 'var(--text-dim)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{what}</span>
          {b.note && <span style={{ color: 'var(--text-muted)', fontStyle: 'italic', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: open ? 'normal' : 'nowrap' }}>{b.note}</span>}
        </span>
      </button>
      {open && (
        <div style={{ padding: '0 8px 8px 37px', display: 'flex', flexDirection: 'column', gap: 4 }}>
          {b.changes.map(c => {
            const k = KIND[c.kind] ?? KIND.update
            const fields = c.kind === 'update' ? changedFields(c.before, c.after) : []
            return (
              <div key={c.seq} data-testid="designer-history-object" data-object={c.object_id} style={{ display: 'flex', gap: 6, alignItems: 'flex-start' }}>
                <span title={k.label} style={{ color: k.color, marginTop: 2 }}><Icon name={k.icon} size={12} /></span>
                <span style={{ flex: 1, minWidth: 0 }}>
                  <span style={{ fontWeight: 600 }}>{objectLabel(c.after ?? c.before, c.object_id)}</span>
                  {c.object_type === 'connection' && <span style={{ color: 'var(--text-muted)' }}> (connection)</span>}
                  <span style={{ ...mono, color: 'var(--text-muted)' }}> #{c.seq}</span>
                  {c.squashed > 0 && <span style={{ color: 'var(--text-muted)' }} title="Several quick saves merged into one row"> · {c.squashed + 1} saves</span>}
                  {fields.length > 0 && (
                    <span style={{ display: 'block', ...mono, color: 'var(--text-dim)', wordBreak: 'break-word' }}>{fields.join(', ')}</span>
                  )}
                </span>
                {canRevert && c.object_type !== 'screen' && (
                  <button type="button" onClick={() => onRevertObject(c, b)} title={`Put ${objectLabel(c.after ?? c.before, c.object_id)} back as it was after change #${b.maxSeq}`}
                    data-testid={`designer-revert-object-${c.object_id}`} style={{
                      height: 20, padding: '0 6px', fontSize: 10.5, borderRadius: 3, border: `1px solid ${D.line}`, background: 'transparent', color: 'var(--fg)', cursor: 'pointer', whiteSpace: 'nowrap',
                    }}>Revert object to here</button>
                )}
              </div>
            )
          })}
          {canRevert && (
            <div style={{ display: 'flex', gap: 6, marginTop: 4 }}>
              <SmallBtn onClick={onRevertScreen} testId="designer-revert-screen" title={`Put the whole screen back as it was after change #${b.maxSeq}`}>Revert screen to here</SmallBtn>
            </div>
          )}
        </div>
      )}
    </div>
  )
}

/** "Last changed by …" for the selected object, with a link to its history. */
export function BlameInfo({ entry, onHistory }: { entry: BlameEntry | undefined; onHistory: () => void }) {
  return (
    <div data-testid="designer-blame" style={{
      display: 'flex', gap: 6, alignItems: 'flex-start', padding: '6px 8px', borderBottom: `1px solid ${D.line}`, fontSize: D.small, color: 'var(--text-dim)', lineHeight: 1.45,
    }}>
      <Icon name="history" size={12} />
      <span style={{ flex: 1, minWidth: 0 }}>
        {entry ? blameLine(entry) : 'Not saved yet: no recorded changes.'}
      </span>
      {entry && (
        <button type="button" onClick={onHistory} data-testid="designer-blame-history" style={{
          border: 'none', background: 'none', padding: 0, color: D.accent, cursor: 'pointer', fontSize: D.small, textDecoration: 'underline',
        }}>History</button>
      )}
    </div>
  )
}
