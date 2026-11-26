import React, { useEffect, useState } from 'react'
import {
  createVersion, publishScreen, reviewVersion, useScreenDiff, useVersions, type DesignStatus, type DesignVersion,
} from '../queries/designHistory'
import { isSignatureCancelled } from '../signature/logic'
import { toast } from '../components/Toast'
import { diffSummary, nextVersionName, relativeTime } from './historyModel'
import { Avatar } from './history'
import { Badge, D, Dialog, DialogBtn, SmallBtn, input, mono } from './ui'

const STATUS: Record<string, { label: string; color: string }> = {
  saved: { label: 'saved', color: 'var(--text-dim)' },
  pending: { label: 'pending approval', color: 'var(--cyan)' },
  published: { label: 'published', color: 'var(--ok)' },
  approved: { label: 'approved', color: 'var(--ok)' },
  rejected: { label: 'rejected', color: 'var(--crit)' },
}

/**
 * Named versions of the screen: record the saved draft as one, compare any
 * two (or one with the draft or what is published), and for publish requests
 * waiting for approval, approve or reject them.
 */
export function VersionsList({ screen, me, canConfigure, publishing, dirty, ensureSaved, onCompare, onChanged }: {
  screen: string
  me: { username: string; isAdmin: boolean }
  canConfigure: boolean
  /** Publishing is on and the screen has a published version ("vs published" means something). */
  publishing: boolean
  dirty: boolean
  /** Save unsaved edits first; false when that failed or was refused. */
  ensureSaved: () => Promise<boolean>
  onCompare: (from: string, to: string) => void
  /** A version was created, approved or rejected. */
  onChanged: () => void
}) {
  const q = useVersions(screen)
  const versions = q.data ?? []
  const [name, setName] = useState('')
  const [message, setMessage] = useState('')
  const [busy, setBusy] = useState(false)
  const [picked, setPicked] = useState<string[]>([])
  const [rejecting, setRejecting] = useState<DesignVersion | null>(null)

  const create = async () => {
    if (busy) return
    setBusy(true)
    try {
      if (dirty && !(await ensureSaved())) return
      const v = await createVersion(screen, { name, message })
      toast.success(`Version ${v.name} recorded`)
      setName(''); setMessage('')
      onChanged()
    } catch (e: any) {
      toast.error(`Version not recorded: ${e?.message || 'request failed'}`)
    } finally {
      setBusy(false)
    }
  }

  const review = async (v: DesignVersion, approve: boolean, note = '') => {
    try {
      await reviewVersion(screen, v.id, approve, note)
      toast.success(approve ? `${v.name} approved and published` : `${v.name} rejected`)
      onChanged()
    } catch (e: any) {
      if (!isSignatureCancelled(e)) toast.error(`${approve ? 'Not approved' : 'Not rejected'}: ${e?.message || 'request failed'}`)
    }
  }

  const toggle = (id: string) => setPicked(p => (p.includes(id) ? p.filter(x => x !== id) : [...p, id].slice(-2)))
  // Older first: compare shows what changed from the older to the newer.
  const pickedSorted = [...picked].sort((a, b) => Number(a) - Number(b))

  return (
    <div data-testid="designer-versions" style={{ display: 'flex', flexDirection: 'column', minHeight: 0, height: '100%' }}>
      {canConfigure && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4, padding: 6, borderBottom: `1px solid ${D.line}` }}>
          <div style={{ display: 'flex', gap: 4 }}>
            <input value={name} onChange={e => setName(e.target.value)} placeholder={`Name (${nextVersionName(versions)})`} aria-label="Version name"
              data-testid="designer-version-name" maxLength={100} style={{ ...input, flex: 1 }} />
            <SmallBtn onClick={() => { void create() }} disabled={busy} testId="designer-version-create"
              title={dirty ? 'Save the screen, then record it as a version' : 'Record the saved screen as a version'}>
              {busy ? 'Saving…' : dirty ? 'Save & create' : 'Create version'}
            </SmallBtn>
          </div>
          <textarea value={message} onChange={e => setMessage(e.target.value)} placeholder="What changed and why (optional)" aria-label="Version message"
            data-testid="designer-version-message" rows={2} maxLength={2000}
            style={{ ...input, height: 'auto', padding: '4px 6px', resize: 'vertical', lineHeight: 1.4 }} />
        </div>
      )}
      <div style={{ display: 'flex', alignItems: 'center', gap: 4, padding: '4px 6px', borderBottom: `1px solid ${D.line}`, fontSize: D.small, color: 'var(--text-muted)' }}>
        <span style={{ flex: 1 }}>{picked.length === 2 ? 'Two versions picked' : 'Tick two versions to compare them'}</span>
        <SmallBtn onClick={() => onCompare(pickedSorted[0], pickedSorted[1])} disabled={picked.length !== 2} testId="designer-versions-compare">Compare</SmallBtn>
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>
        {q.isLoading && <div style={{ padding: 10, color: 'var(--text-muted)' }}>Loading versions…</div>}
        {q.isSuccess && !versions.length && (
          <div style={{ padding: 10, color: 'var(--text-muted)', lineHeight: 1.5 }}>No versions yet. A version is a named copy of the saved screen you can compare with, go back to, or publish.</div>
        )}
        {versions.map(v => {
          const st = STATUS[v.status] ?? STATUS.saved
          const id = String(v.id)
          const mayReview = v.status === 'pending' && me.isAdmin && v.requested_by !== me.username
          return (
            <div key={v.id} data-testid="designer-version" data-version={v.name} style={{
              display: 'flex', gap: 7, padding: '6px 8px', borderBottom: `1px solid color-mix(in srgb, ${D.line} 60%, transparent)`,
              background: picked.includes(id) ? D.selBg : undefined,
            }}>
              <input type="checkbox" checked={picked.includes(id)} onChange={() => toggle(id)} aria-label={`Pick ${v.name} to compare`}
                data-testid={`designer-version-pick-${v.name}`} style={{ marginTop: 3 }} />
              <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 3 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 5, flexWrap: 'wrap' }}>
                  <b style={{ fontSize: D.font }}>{v.name}</b>
                  {v.current && <Badge color="var(--ok)" testId="designer-version-current" title="What operators see now">current</Badge>}
                  {(v.status !== 'saved' && !(v.current && v.status === 'published')) && <Badge color={st.color} testId="designer-version-status">{st.label}</Badge>}
                  <span style={{ flex: 1 }} />
                  <span style={{ ...mono, color: 'var(--text-muted)' }}>#{v.upto_seq}</span>
                </div>
                {v.message && <div style={{ color: 'var(--text-dim)', lineHeight: 1.4, wordBreak: 'break-word' }}>{v.message}</div>}
                <div style={{ display: 'flex', alignItems: 'center', gap: 5, color: 'var(--text-muted)', fontSize: D.small }}>
                  <Avatar name={v.username} size={16} />
                  <span>{v.username} · {relativeTime(v.at_ms)} · {v.widgets} components</span>
                </div>
                {v.status === 'pending' && v.requested_by && (
                  <div style={{ fontSize: D.small, color: 'var(--cyan)' }}>Publish requested by {v.requested_by}{v.requested_by === me.username ? ' (you): another administrator approves it' : ''}</div>
                )}
                {v.status === 'rejected' && v.reviewed_by && (
                  <div style={{ fontSize: D.small, color: 'var(--crit)' }}>Rejected by {v.reviewed_by}{v.review_note ? `: ${v.review_note}` : ''}</div>
                )}
                <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap', marginTop: 2 }}>
                  <SmallBtn onClick={() => onCompare(id, 'draft')} testId={`designer-version-vs-draft-${v.name}`} title={`What changed from ${v.name} to the draft`}>vs draft</SmallBtn>
                  {publishing && !v.current && (
                    <SmallBtn onClick={() => onCompare('published', id)} testId={`designer-version-vs-published-${v.name}`} title={`What ${v.name} would change for operators`}>vs published</SmallBtn>
                  )}
                  {mayReview && (
                    <>
                      <SmallBtn onClick={() => { void review(v, true) }} testId={`designer-version-approve-${v.name}`}>Approve</SmallBtn>
                      <SmallBtn danger onClick={() => setRejecting(v)} testId={`designer-version-reject-${v.name}`}>Reject</SmallBtn>
                    </>
                  )}
                </div>
              </div>
            </div>
          )
        })}
      </div>
      {rejecting && (
        <RejectDialog v={rejecting} onCancel={() => setRejecting(null)} onReject={note => { const v = rejecting; setRejecting(null); void review(v, false, note) }} />
      )}
    </div>
  )
}

function RejectDialog({ v, onReject, onCancel }: { v: DesignVersion; onReject: (note: string) => void; onCancel: () => void }) {
  const [note, setNote] = useState('')
  return (
    <Dialog title={`Reject ${v.name}?`} onCancel={onCancel} testId="designer-reject-dialog" actions={<>
      <DialogBtn danger onClick={() => onReject(note)} testId="designer-reject-confirm">Reject</DialogBtn>
      <DialogBtn onClick={onCancel}>Cancel</DialogBtn>
    </>}>
      <div style={{ color: 'var(--text-dim)' }}>{v.requested_by ?? v.username} asked to publish {v.name}. Operators keep seeing what is published now.</div>
      <textarea value={note} onChange={e => setNote(e.target.value)} placeholder="Why (shown to the requester)" rows={3} aria-label="Reason"
        data-testid="designer-reject-note" style={{ ...input, height: 'auto', padding: '4px 6px' }} />
    </Dialog>
  )
}

/**
 * Publish the draft (recorded as a new version) or one version: name,
 * message, and what operators will see change. With approval required it
 * files a request instead. Unsaved edits are saved first (Save & publish).
 */
export function PublishDialog({ screen, status, versions, version, dirty, live = false, save, baseSeq, onClose, onDone }: {
  screen: string
  status: DesignStatus | undefined
  versions: DesignVersion[]
  /** Publish this version instead of the draft. */
  version?: { id: string; label: string }
  dirty: boolean
  /** A live session edits the draft: the check is against the draft the diff showed. */
  live?: boolean
  save: () => Promise<boolean>
  /** The draft's change-log head as loaded (read after any save); not used when live. */
  baseSeq: () => number | null
  onClose: () => void
  onDone: () => void
}) {
  const approval = status?.approval === 'required'
  const [name, setName] = useState('')
  const [message, setMessage] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<{ status: 'published' | 'pending'; version: DesignVersion } | null>(null)
  const diff = useScreenDiff(screen, 'published', version?.id ?? 'draft', false)
  const s = diff.data?.diff.summary
  const current = status?.published?.name ?? null
  const willSave = dirty && !version
  const verb = approval ? 'Request publish' : 'Publish'
  /** The draft's head as the summary shows it. */
  const shownSeq = (() => {
    const to = diff.data?.to
    return to?.kind === 'draft' && typeof to.seq === 'number' ? to.seq : null
  })()
  const waitDiff = live && !version && shownSeq == null && diff.isLoading
  // Live edits go to the draft every couple of seconds: write them now, so the summary has them.
  useEffect(() => { if (live && !version) void save() }, []) // eslint-disable-line react-hooks/exhaustive-deps

  const submit = async () => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      if (willSave && !(await save())) { setError('The screen was not saved, so nothing was published.'); return }
      const r = await publishScreen(screen, version
        ? { version: version.id, message }
        : { name, message, baseSeq: live ? shownSeq : baseSeq() })
      if (r.ok) { setResult({ status: r.status, version: r.version }); onDone(); return }
      if (r.code === 'conflict' && live && !version) {
        void diff.refetch()
        setError('The draft changed since this summary was shown. Check the updated summary and publish again.')
        return
      }
      setError(r.code === 'conflict'
        ? 'The screen was changed by someone else since you loaded it. Close this, check the change, and publish again.'
        : r.message)
    } catch (e: any) {
      if (!isSignatureCancelled(e)) setError(e?.message || 'Request failed')
    } finally {
      setBusy(false)
    }
  }

  if (result) {
    return (
      <Dialog title={result.status === 'pending' ? 'Publish requested' : 'Published'} onCancel={onClose} testId="designer-publish-dialog"
        actions={<DialogBtn primary onClick={onClose} testId="designer-publish-close" autoFocus>Close</DialogBtn>}>
        <div data-testid="designer-publish-result" data-status={result.status} style={{ lineHeight: 1.5, color: 'var(--text-dim)' }}>
          {result.status === 'pending'
            ? <>Request to publish <b style={{ color: 'var(--fg)' }}>{result.version.name}</b> sent. An administrator other than you must approve it; until then operators see {current ? <b style={{ color: 'var(--fg)' }}>{current}</b> : 'the draft'}.</>
            : <><b style={{ color: 'var(--fg)' }}>{result.version.name}</b> is published: operators see it now.</>}
        </div>
      </Dialog>
    )
  }

  return (
    <Dialog title={version ? `${verb} ${version.label}` : `${verb} this screen`} onCancel={onClose} testId="designer-publish-dialog" actions={<>
      <DialogBtn primary onClick={() => { void submit() }} disabled={busy || waitDiff} testId="designer-publish-confirm">
        {busy ? 'Working…' : willSave ? `Save & ${verb.toLowerCase()}` : verb}
      </DialogBtn>
      <DialogBtn onClick={onClose} testId="designer-publish-cancel">Cancel</DialogBtn>
    </>}>
      <div data-testid="designer-publish-summary" style={{ lineHeight: 1.5, color: 'var(--text-dim)' }}>
        {!status?.published
          ? <>First publish. Operators see this screen as it is now; later edits stay in the draft until published.</>
          : <>Operators see <b style={{ color: 'var(--fg)' }}>{current}</b>. {version ? `${version.label} changes` : 'Publishing the draft changes'}: <b style={{ color: 'var(--fg)' }}>{diff.isLoading ? '…' : diffSummary(s)}</b>.</>}
        {approval && <div style={{ marginTop: 4 }}>This site requires approval: an administrator other than you approves the request before operators see it.</div>}
        {willSave && <div style={{ marginTop: 4, color: 'var(--warn)' }}>You have unsaved changes; they are saved first.</div>}
      </div>
      {!version && (
        <label style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
          <span style={{ color: 'var(--text-dim)' }}>Version name</span>
          <input value={name} onChange={e => setName(e.target.value)} placeholder={nextVersionName(versions)} maxLength={100}
            data-testid="designer-publish-name" style={input} />
        </label>
      )}
      <label style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
        <span style={{ color: 'var(--text-dim)' }}>Message</span>
        <textarea value={message} onChange={e => setMessage(e.target.value)} placeholder="What changes for operators, and why" rows={3} maxLength={2000}
          data-testid="designer-publish-message" style={{ ...input, height: 'auto', padding: '4px 6px', lineHeight: 1.4 }} />
      </label>
      {error && <div role="alert" data-testid="designer-publish-error" style={{ color: 'var(--crit)' }}>{error}</div>}
    </Dialog>
  )
}
