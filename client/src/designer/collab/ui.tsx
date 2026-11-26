// Presence and comments in the Designer (docs/collaboration.md): avatars in
// the title bar, other people's cursors and selections on the stage, comment
// pins and the Comments tab.
import React, { useMemo, useState } from 'react'
import type { Doc } from '../model'
import type { DesignSessionInfo } from '../../api/websocket'
import type { DesignComment } from '../../queries/designComments'
import { threadsOf } from '../../queries/designComments'
import { initials, relativeTime } from '../historyModel'
import { Badge, D, SmallBtn, mono } from '../ui'
import type { CollabStatus, Peer } from './provider'

const chip: React.CSSProperties = {
  display: 'inline-flex', alignItems: 'center', gap: 5, height: 20, padding: '0 8px', borderRadius: 10, fontSize: D.small, whiteSpace: 'nowrap',
}

export function UserDot({ user, color, size = 22, ring, title, onClick, testId }: {
  user: string; color: string; size?: number; ring?: boolean; title?: string; onClick?: () => void; testId?: string
}) {
  const style: React.CSSProperties = {
    width: size, height: size, borderRadius: size / 2, display: 'inline-grid', placeItems: 'center', flexShrink: 0,
    fontSize: size * 0.42, fontWeight: 700, color: '#fff', background: color, border: 'none', padding: 0,
    boxShadow: ring ? `0 0 0 2px var(--bg), 0 0 0 4px ${color}` : '0 0 0 2px var(--bg)', cursor: onClick ? 'pointer' : 'default',
  }
  return onClick
    ? <button type="button" title={title ?? user} aria-label={title ?? user} onClick={onClick} data-testid={testId} data-user={user} style={style}>{initials(user)}</button>
    : <span title={title ?? user} data-testid={testId} data-user={user} style={style}>{initials(user)}</span>
}

/** Status chip and avatars of the people in the session. Click an avatar to follow them. */
export function PresenceBar({ status, peers, me, pending, following, onFollow }: {
  status: CollabStatus; peers: Peer[]; me: { user: string; color: string }; pending: boolean
  following: number | null; onFollow: (clientId: number | null) => void
}) {
  // One avatar per person (several tabs of one person show once).
  const people = useMemo(() => {
    const seen = new Map<string, Peer>()
    for (const p of peers) if (!seen.has(p.user)) seen.set(p.user, p)
    return [...seen.values()]
  }, [peers])
  const tone = status === 'live' ? 'var(--ok)' : status === 'off' ? 'var(--text-muted)' : 'var(--warn)'
  const text = status === 'live' ? `Live · ${people.length + 1} editing` : status === 'connecting' ? 'Connecting…' : status === 'offline' ? (pending ? 'Offline · edits kept, reconnecting' : 'Offline · reconnecting') : 'Single-user'
  return (
    <div data-testid="designer-presence" data-status={status} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
      <span data-testid="designer-collab-status" style={{ ...chip, border: `1px solid color-mix(in srgb, ${tone} 45%, transparent)`, color: tone }}>
        <span style={{ width: 6, height: 6, borderRadius: 3, background: tone }} />{text}
      </span>
      <div style={{ display: 'flex', gap: 2, alignItems: 'center' }}>
        {people.map(p => (
          <UserDot key={p.clientId} user={p.user} color={p.color} ring={following === p.clientId} testId="designer-peer-avatar"
            title={following === p.clientId ? `Following ${p.user}: click to stop` : `${p.user} is editing: click to follow their view`}
            onClick={() => onFollow(following === p.clientId ? null : p.clientId)} />
        ))}
        {me.user && <UserDot user={me.user} color={me.color} size={18} title={`You (${me.user})`} testId="designer-me-avatar" />}
      </div>
    </div>
  )
}

/** Initials of the people editing a screen, for the Project pane. */
export function ScreenEditors({ screen, sessions, me }: { screen: string; sessions: DesignSessionInfo[] | null; me: string }) {
  const users = sessions?.find(s => s.screen === screen)?.users.filter(u => u.user !== me) ?? []
  if (!users.length) return null
  return (
    <span data-testid={`designer-screen-editors-${screen}`} title={`Editing now: ${users.map(u => u.user).join(', ')}`} style={{ display: 'inline-flex', gap: 2, marginLeft: 6, verticalAlign: 'middle' }}>
      {users.slice(0, 4).map(u => <UserDot key={u.user} user={u.user} color={u.color} size={13} />)}
      {users.length > 4 && <span style={{ ...mono, color: 'var(--text-muted)' }}>+{users.length - 4}</span>}
    </span>
  )
}

/** Other people's cursors (with names) and selections, in screen pixels over the stage. */
export function PeersOverlay({ peers, doc, sx }: { peers: Peer[]; doc: Doc; sx: (v: number) => number }) {
  const byId = useMemo(() => new Map(doc.widgets.map(w => [w.id, w])), [doc.widgets])
  return (
    <div data-testid="designer-peers" style={{ position: 'absolute', inset: 0, pointerEvents: 'none', zIndex: 5 }}>
      <svg style={{ position: 'absolute', left: 0, top: 0, overflow: 'visible' }} width={1} height={1}>
        {peers.flatMap(p => (p.selection ?? []).map(id => {
          const w = byId.get(id)
          return w ? <rect key={`${p.clientId}:${id}`} data-testid="designer-peer-selection" x={sx(w.x) - 2.5} y={sx(w.y) - 2.5} width={sx(w.width) + 5} height={sx(w.height) + 5}
            fill="none" stroke={p.color} strokeWidth={1.5} rx={2} /> : null
        }))}
      </svg>
      {peers.filter(p => p.cursor).map(p => (
        <div key={p.clientId} data-testid="designer-peer-cursor" data-user={p.user} style={{
          position: 'absolute', left: sx(p.cursor!.x), top: sx(p.cursor!.y), transition: 'left 80ms linear, top 80ms linear',
        }}>
          <svg width="14" height="18" viewBox="0 0 14 18" style={{ display: 'block' }}>
            <path d="M1 1l11 9.5-5 .6 3 5.4-2.2 1.2-3-5.5L1 15z" fill={p.color} stroke="#fff" strokeWidth="1" />
          </svg>
          <span style={{ position: 'absolute', left: 12, top: 14, padding: '1px 6px', borderRadius: 3, fontSize: 11, fontWeight: 600, whiteSpace: 'nowrap', color: '#fff', background: p.color }}>{p.user}</span>
        </div>
      ))}
    </div>
  )
}

/** Where a comment sits: on its component (top right corner), else at its point. */
export function pinAt(c: DesignComment, doc: Doc): { x: number; y: number } {
  const w = c.object_id && c.object_type !== 'connection' ? doc.widgets.find(x => x.id === c.object_id) : undefined
  return w ? { x: w.x + w.width, y: w.y } : { x: c.x, y: c.y }
}

export function CommentPins({ comments, doc, sx, active, showResolved, onOpen }: {
  comments: DesignComment[]; doc: Doc; sx: (v: number) => number; active: number | null; showResolved: boolean; onOpen: (id: number) => void
}) {
  const threads = threadsOf(comments).filter(t => showResolved || !t.root.resolved)
  return (
    <div style={{ position: 'absolute', inset: 0, pointerEvents: 'none', zIndex: 6 }}>
      {threads.map(t => {
        const at = pinAt(t.root, doc)
        return (
          <button key={t.root.id} type="button" data-testid="designer-comment-pin" data-comment={t.root.id} onClick={() => onOpen(t.root.id)}
            title={`${t.root.author}: ${t.root.body}`} style={{
              position: 'absolute', left: sx(at.x) - 4, top: sx(at.y) - 20, minWidth: 22, height: 22, padding: '0 6px', pointerEvents: 'auto',
              borderRadius: '11px 11px 11px 2px', border: `1.5px solid ${active === t.root.id ? D.accent : 'var(--bg)'}`, cursor: 'pointer',
              background: t.root.resolved ? 'var(--text-muted)' : 'var(--warn)', color: '#111', fontSize: 11, fontWeight: 700,
              boxShadow: '0 2px 6px rgba(0,0,0,.35)', opacity: t.root.resolved ? 0.6 : 1,
            }}>{t.replies.length ? t.replies.length + 1 : initials(t.root.author)}</button>
        )
      })}
    </div>
  )
}

/** Text with `@name` mentions highlighted. */
function Body({ text, users }: { text: string; users: Set<string> }) {
  const parts = text.split(/(@[\w.-]+)/g)
  return <>{parts.map((p, i) => (p.startsWith('@') && users.has(p.slice(1)) ? <b key={i} style={{ color: D.accent }}>{p}</b> : <React.Fragment key={i}>{p}</React.Fragment>))}</>
}

/** A text box that offers @mentions from `users`. Enter sends, Shift+Enter breaks the line. */
export function CommentBox({ users, placeholder, onSend, autoFocus, testId }: {
  users: string[]; placeholder: string; onSend: (body: string) => Promise<void> | void; autoFocus?: boolean; testId?: string
}) {
  const [text, setText] = useState('')
  const [busy, setBusy] = useState(false)
  const m = /(^|\s)@([\w.-]*)$/.exec(text)
  const offers = m ? users.filter(u => u.toLowerCase().startsWith(m[2].toLowerCase())).slice(0, 6) : []
  const send = async () => {
    const body = text.trim()
    if (!body || busy) return
    setBusy(true)
    try { await onSend(body); setText('') } finally { setBusy(false) }
  }
  return (
    <div style={{ position: 'relative', display: 'flex', flexDirection: 'column', gap: 4 }}>
      <textarea value={text} onChange={e => setText(e.target.value)} placeholder={placeholder} autoFocus={autoFocus} data-testid={testId} rows={2}
        onKeyDown={e => {
          if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); if (offers.length && m) setText(text.slice(0, text.length - m[2].length) + offers[0] + ' '); else void send() }
          e.stopPropagation()
        }}
        style={{ resize: 'vertical', minHeight: 40, padding: 6, fontSize: D.font, background: 'var(--input-bg)', color: 'var(--fg)', border: `1px solid ${D.line}`, borderRadius: 3, fontFamily: 'inherit' }} />
      {offers.length > 0 && (
        <div role="listbox" style={{ position: 'absolute', left: 0, bottom: '100%', zIndex: 10, background: D.pane, border: `1px solid ${D.line}`, borderRadius: 3, boxShadow: '0 4px 14px rgba(0,0,0,.35)' }}>
          {offers.map(u => (
            <button key={u} type="button" role="option" aria-selected={false} onMouseDown={e => { e.preventDefault(); setText(text.slice(0, text.length - m![2].length) + u + ' ') }}
              style={{ display: 'block', width: '100%', textAlign: 'left', padding: '3px 8px', border: 'none', background: 'transparent', color: 'var(--fg)', cursor: 'pointer', fontSize: D.small }}>@{u}</button>
          ))}
        </div>
      )}
      <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
        <SmallBtn onClick={() => { void send() }} disabled={!text.trim() || busy} testId={testId ? `${testId}-send` : undefined}>{busy ? 'Sending…' : 'Send'}</SmallBtn>
      </div>
    </div>
  )
}

/** The Comments tab: threads with replies, resolve and reopen, delete own. */
export function CommentsPane({ comments, me, isAdmin, users, active, onActive, labelOf, onReply, onResolve, onDelete, onNew, hint }: {
  comments: DesignComment[]; me: string; isAdmin: boolean; users: string[]
  active: number | null; onActive: (id: number | null) => void
  labelOf: (c: DesignComment) => string
  onReply: (root: DesignComment, body: string) => Promise<void>
  onResolve: (root: DesignComment, resolved: boolean) => void
  onDelete: (c: DesignComment) => void
  /** Start a comment on the selection (or ask to click the stage). */
  onNew: () => void
  hint: string
}) {
  const [showResolved, setShowResolved] = useState(false)
  const userSet = useMemo(() => new Set(users), [users])
  const threads = threadsOf(comments).filter(t => showResolved || !t.root.resolved || t.root.id === active)
  const now = Date.now()
  return (
    <div data-testid="designer-comments" style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: 6, borderBottom: `1px solid ${D.line}`, fontSize: D.small }}>
        <SmallBtn onClick={onNew} testId="designer-comment-new" title={hint}>New comment</SmallBtn>
        <div style={{ flex: 1 }} />
        <label style={{ display: 'inline-flex', gap: 4, alignItems: 'center', color: 'var(--text-dim)' }}>
          <input type="checkbox" checked={showResolved} onChange={e => setShowResolved(e.target.checked)} />Resolved
        </label>
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>
        {threads.length === 0 && <div style={{ padding: 10, color: 'var(--text-muted)', lineHeight: 1.5 }}>No comments. {hint}</div>}
        {threads.map(({ root, replies }) => (
          <div key={root.id} data-testid="designer-comment-thread" data-comment={root.id} onClick={() => onActive(root.id)} style={{
            padding: 8, borderBottom: `1px solid color-mix(in srgb, ${D.line} 60%, transparent)`, background: active === root.id ? D.selBg : undefined, cursor: 'pointer',
            opacity: root.resolved ? 0.7 : 1,
          }}>
            <div style={{ display: 'flex', gap: 6, alignItems: 'center', fontSize: D.small, color: 'var(--text-muted)', marginBottom: 4 }}>
              <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>on <b style={{ color: 'var(--fg)' }}>{labelOf(root)}</b></span>
              {root.resolved && <Badge>resolved{root.resolved_by ? ` by ${root.resolved_by}` : ''}</Badge>}
            </div>
            {[root, ...replies].map(c => (
              <div key={c.id} data-testid="designer-comment" style={{ display: 'flex', gap: 6, alignItems: 'flex-start', marginTop: 4 }}>
                <UserDot user={c.author} color={`hsl(${hueOf(c.author)} 45% 40%)`} size={18} />
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ display: 'flex', gap: 6, alignItems: 'baseline' }}>
                    <b>{c.author}</b>
                    <span style={{ ...mono, color: 'var(--text-muted)' }}>{relativeTime(c.created_at_ms, now)}</span>
                    <div style={{ flex: 1 }} />
                    {(c.author === me || isAdmin) && (
                      <button type="button" title="Delete" onClick={e => { e.stopPropagation(); onDelete(c) }}
                        style={{ border: 'none', background: 'transparent', color: 'var(--text-muted)', cursor: 'pointer', fontSize: 11 }}>Delete</button>
                    )}
                  </div>
                  <div style={{ whiteSpace: 'pre-wrap', wordBreak: 'break-word', lineHeight: 1.45 }}><Body text={c.body} users={userSet} /></div>
                </div>
              </div>
            ))}
            {active === root.id && (
              <div style={{ marginTop: 8, display: 'flex', flexDirection: 'column', gap: 6 }} onClick={e => e.stopPropagation()}>
                {!root.resolved && <CommentBox users={users} placeholder="Reply… (@ to mention)" onSend={b => onReply(root, b)} testId="designer-comment-reply" />}
                <div style={{ display: 'flex', gap: 6 }}>
                  <SmallBtn onClick={() => onResolve(root, !root.resolved)} testId="designer-comment-resolve">{root.resolved ? 'Reopen' : 'Resolve'}</SmallBtn>
                </div>
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  )
}

function hueOf(name: string): number {
  let h = 0
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) >>> 0
  return h % 360
}
