import { useState, useEffect, useMemo } from 'react'
import { useAnnotations, useCreateAnnotation, useUpdateAnnotation, useDeleteAnnotation } from './queries'
import type { Annotation } from './queries'
import { btnStyle, inputStyle, labelStyle, labelTextStyle } from './styles'

function formatTimestamp(epoch: number): string {
  return new Date(epoch * 1000).toLocaleString('en-US', {
    year: 'numeric', month: 'short', day: '2-digit',
    hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false,
  })
}

function MentionText({ text }: { text: string }) {
  const parts = useMemo(() => {
    const result: { text: string; isMention: boolean }[] = []
    const regex = /(@\w+)/g
    let lastIndex = 0
    let match: RegExpExecArray | null
    while ((match = regex.exec(text)) !== null) {
      if (match.index > lastIndex) {
        result.push({ text: text.slice(lastIndex, match.index), isMention: false })
      }
      result.push({ text: match[1], isMention: true })
      lastIndex = match.index + match[1].length
    }
    if (lastIndex < text.length) {
      result.push({ text: text.slice(lastIndex), isMention: false })
    }
    return result
  }, [text])

  return (
    <span>
      {parts.map((part, i) =>
        part.isMention
          ? <span key={i} style={{ color: '#60a5fa', fontWeight: 600 }}>{part.text}</span>
          : <span key={i}>{part.text}</span>
      )}
    </span>
  )
}

function PencilIcon({ size = 14 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
      <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
      <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
    </svg>
  )
}

function TrashIcon({ size = 14 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
      <polyline points="3 6 5 6 21 6" />
      <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
    </svg>
  )
}

function PlusIcon({ size = 14 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
      <line x1="12" y1="5" x2="12" y2="19" />
      <line x1="5" y1="12" x2="19" y2="12" />
    </svg>
  )
}

function TagIcon({ size = 16, color = 'var(--cyan)' }: { size?: number; color?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
      <path d="M20.59 13.41l-7.17 7.17a2 2 0 0 1-2.83 0L2 12V2h10l8.59 8.59a2 2 0 0 1 0 2.82z" />
      <line x1="7" y1="7" x2="7.01" y2="7" />
    </svg>
  )
}

interface AnnotationFormState {
  tag_id: string
  annotation_text: string
  data_timestamp: string
}

const emptyForm: AnnotationFormState = { tag_id: '', annotation_text: '', data_timestamp: '' }

interface Props {
  onClose: () => void
  embedded?: boolean
}

export function AnnotationPanel({ onClose, embedded }: Props) {
  useEffect(() => {
    if (embedded) return
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, embedded])

  const [tagFilter, setTagFilter] = useState('')
  const [fromFilter, setFromFilter] = useState('')
  const [toFilter, setToFilter] = useState('')
  const [showCreate, setShowCreate] = useState(false)
  const [editingId, setEditingId] = useState<number | null>(null)
  const [form, setForm] = useState<AnnotationFormState>(emptyForm)
  const [editForm, setEditForm] = useState<AnnotationFormState>(emptyForm)

  const queryParams = useMemo(() => ({
    tag_id: tagFilter || undefined,
    from: fromFilter ? Math.floor(new Date(fromFilter).getTime() / 1000) : undefined,
    to: toFilter ? Math.floor(new Date(toFilter).getTime() / 1000) : undefined,
  }), [tagFilter, fromFilter, toFilter])

  const { data: annotationsData, isLoading } = useAnnotations(queryParams)
  const annotations = annotationsData?.annotations ?? []

  const createMut = useCreateAnnotation()
  const updateMut = useUpdateAnnotation()
  const deleteMut = useDeleteAnnotation()

  const handleCreate = () => {
    if (!form.tag_id || !form.annotation_text) return
    const ts = form.data_timestamp ? Math.floor(new Date(form.data_timestamp).getTime() / 1000) : Math.floor(Date.now() / 1000)
    createMut.mutate(
      { tag_id: form.tag_id, annotation_text: form.annotation_text, data_timestamp: ts },
      { onSuccess: () => { setForm(emptyForm); setShowCreate(false) } }
    )
  }

  const startEdit = (ann: Annotation) => {
    setEditingId(ann.id)
    setEditForm({
      tag_id: ann.tag_id,
      annotation_text: ann.annotation_text,
      data_timestamp: new Date(ann.data_timestamp * 1000).toISOString().slice(0, 16),
    })
  }

  const handleUpdate = () => {
    if (!editingId || !editForm.annotation_text) return
    const ts = editForm.data_timestamp ? Math.floor(new Date(editForm.data_timestamp).getTime() / 1000) : Math.floor(Date.now() / 1000)
    updateMut.mutate(
      { id: editingId, annotation_text: editForm.annotation_text, data_timestamp: ts },
      { onSuccess: () => { setEditingId(null); setEditForm(emptyForm) } }
    )
  }

  const handleDelete = (id: number) => {
    if (confirm('Delete this annotation?')) {
      deleteMut.mutate(id)
    }
  }

  const tdStyle: React.CSSProperties = {
    padding: '10px 14px', fontSize: 13, borderBottom: '1px solid var(--border)', verticalAlign: 'middle',
  }

  const thStyle: React.CSSProperties = {
    textAlign: 'left', padding: '10px 14px', fontSize: 11, color: 'var(--muted)',
    textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600,
    borderBottom: '1px solid var(--border)', whiteSpace: 'nowrap',
  }

  const content = (
    <>
      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        padding: '18px 24px', borderBottom: '1px solid var(--border)', flexShrink: 0,
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <TagIcon size={20} />
          <span style={{ fontSize: 17, fontWeight: 700 }}>Annotations</span>
          <span style={{
            fontSize: 11, color: 'var(--muted)', background: 'var(--card)',
            border: '1px solid var(--border)', borderRadius: 6, padding: '2px 8px',
            fontFamily: 'var(--font-mono)',
          }}>
            {annotations.length} item{annotations.length !== 1 ? 's' : ''}
          </span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <button onClick={() => setShowCreate(v => !v)} style={{
            ...btnStyle, padding: '5px 10px', borderColor: 'var(--cyan)', color: 'var(--cyan)',
          }} aria-label="Create new annotation">
            <PlusIcon /> New
          </button>
          {!embedded && (
            <button onClick={onClose} aria-label="Close" style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', padding: 4, display: 'flex', alignItems: 'center', justifyContent: 'center', borderRadius: 4, transition: 'color 0.15s', marginLeft: 'auto' }}>
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
            </button>
          )}
        </div>
      </div>

      {/* Create form */}
      {showCreate && (
        <div style={{
          padding: '16px 24px', borderBottom: '1px solid var(--border)',
          background: 'rgba(34,197,94,.04)', flexShrink: 0,
        }}>
          <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 10 }}>New Annotation</div>
          <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', marginBottom: 10 }}>
            <label style={{ ...labelStyle, flex: '1 1 180px' }}>
              <span style={labelTextStyle}>Tag ID</span>
              <input value={form.tag_id} onChange={e => setForm({ ...form, tag_id: e.target.value })} placeholder="e.g. sensor-01" style={inputStyle} />
            </label>
            <label style={{ ...labelStyle, flex: '1 1 220px' }}>
              <span style={labelTextStyle}>Timestamp</span>
              <input type="datetime-local" value={form.data_timestamp} onChange={e => setForm({ ...form, data_timestamp: e.target.value })} style={inputStyle} />
            </label>
          </div>
          <label style={{ ...labelStyle, marginBottom: 10 }}>
            <span style={labelTextStyle}>Text</span>
            <textarea value={form.annotation_text} onChange={e => setForm({ ...form, annotation_text: e.target.value })} placeholder="Annotation text... Use @mention to tag users" rows={2} style={{ ...inputStyle, resize: 'vertical', minHeight: 48 }} />
          </label>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <button onClick={handleCreate} disabled={!form.tag_id || !form.annotation_text || createMut.isPending} style={{
              ...btnStyle, padding: '5px 10px', borderColor: 'var(--cyan)', color: 'var(--cyan)',
              opacity: (!form.tag_id || !form.annotation_text || createMut.isPending) ? 0.5 : 1,
            }}>
              {createMut.isPending ? 'Creating...' : 'Create'}
            </button>
            <button onClick={() => { setShowCreate(false); setForm(emptyForm) }} style={{
              ...btnStyle, padding: '5px 10px', color: 'var(--muted)',
            }}>Cancel</button>
            {createMut.isError && <span style={{ fontSize: 12, color: 'var(--crit)' }}>{createMut.error?.message || 'Failed'}</span>}
          </div>
        </div>
      )}

      {/* Filter bar */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 12, padding: '14px 24px',
        borderBottom: '1px solid var(--border)', flexShrink: 0, flexWrap: 'wrap',
      }}>
        <label style={{ ...labelStyle, flex: '1 1 180px' }}>
          <span style={labelTextStyle}>Filter by Tag</span>
          <input value={tagFilter} onChange={e => setTagFilter(e.target.value)} placeholder="Tag ID..." style={inputStyle} />
        </label>
        <label style={{ ...labelStyle, flex: '1 1 180px' }}>
          <span style={labelTextStyle}>From</span>
          <input type="datetime-local" value={fromFilter} onChange={e => setFromFilter(e.target.value)} style={inputStyle} />
        </label>
        <label style={{ ...labelStyle, flex: '1 1 180px' }}>
          <span style={labelTextStyle}>To</span>
          <input type="datetime-local" value={toFilter} onChange={e => setToFilter(e.target.value)} style={inputStyle} />
        </label>
      </div>

      {/* List */}
      <div style={{ flex: 1, overflow: 'auto' }}>
        {isLoading ? (
          <div style={{ padding: 60, textAlign: 'center', color: 'var(--muted)', fontSize: 14 }}>Loading annotations...</div>
        ) : annotations.length === 0 ? (
          <div style={{ padding: 60, textAlign: 'center' }}>
            <div style={{ color: 'var(--muted)', fontSize: 14, marginBottom: 8 }}>No annotations found</div>
            <div style={{ color: 'var(--muted)', fontSize: 12, opacity: 0.7 }}>
              {tagFilter || fromFilter || toFilter ? 'Try adjusting your filters' : 'Click "New" to create your first annotation'}
            </div>
          </div>
        ) : (
          <table style={{ width: '100%', borderCollapse: 'collapse' }}>
            <thead>
              <tr>
                <th style={thStyle}>Time</th>
                <th style={thStyle}>Tag</th>
                <th style={{ ...thStyle, minWidth: 200 }}>Text</th>
                <th style={thStyle}>Author</th>
                <th style={thStyle}>Actions</th>
              </tr>
            </thead>
            <tbody>
              {annotations.map(ann => {
                const isEditing = editingId === ann.id
                return (
                  <tr key={ann.id} style={{ transition: 'background 0.1s ease' }}
                    onMouseEnter={e => { (e.currentTarget as HTMLElement).style.background = 'rgba(255,255,255,.02)' }}
                    onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = 'transparent' }}>
                    <td style={{ ...tdStyle, fontFamily: 'var(--font-mono)', fontSize: 12, whiteSpace: 'nowrap', color: 'var(--muted)' }}>
                      {formatTimestamp(ann.data_timestamp)}
                    </td>
                    <td style={tdStyle}>
                      <span style={{
                        display: 'inline-block', padding: '2px 8px', borderRadius: 4,
                        fontSize: 11, fontWeight: 600, background: 'rgba(34,197,94,.12)',
                        color: 'var(--ok)', fontFamily: 'var(--font-mono)',
                      }}>{ann.tag_id}</span>
                    </td>
                    <td style={{ ...tdStyle, fontSize: 13, maxWidth: 340 }}>
                      {isEditing ? (
                        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                          <textarea value={editForm.annotation_text} onChange={e => setEditForm({ ...editForm, annotation_text: e.target.value })} rows={2} style={{ ...inputStyle, resize: 'vertical', minHeight: 48 }} />
                          <label style={{ ...labelStyle, flex: '1 1 180px' }}>
                            <span style={labelTextStyle}>Timestamp</span>
                            <input type="datetime-local" value={editForm.data_timestamp} onChange={e => setEditForm({ ...editForm, data_timestamp: e.target.value })} style={inputStyle} />
                          </label>
                          <div style={{ display: 'flex', gap: 8 }}>
                            <button onClick={handleUpdate} disabled={!editForm.annotation_text || updateMut.isPending} style={{
                              ...btnStyle, padding: '5px 10px', fontSize: 12, borderColor: 'var(--cyan)', color: 'var(--cyan)',
                              opacity: (!editForm.annotation_text || updateMut.isPending) ? 0.5 : 1,
                            }}>
                              {updateMut.isPending ? 'Saving...' : 'Save'}
                            </button>
                            <button onClick={() => { setEditingId(null); setEditForm(emptyForm) }} style={{
                              ...btnStyle, padding: '5px 10px', fontSize: 12, color: 'var(--muted)',
                            }}>Cancel</button>
                          </div>
                        </div>
                      ) : (
                        <MentionText text={ann.annotation_text} />
                      )}
                    </td>
                    <td style={{ ...tdStyle, fontSize: 12, color: 'var(--muted)' }}>
                      #{ann.author_id}
                    </td>
                    <td style={tdStyle}>
                      {!isEditing && (
                        <div style={{ display: 'flex', gap: 6 }}>
                          <button onClick={() => startEdit(ann)} aria-label="Edit annotation" style={{
                            ...btnStyle, padding: '4px 8px', fontSize: 12,
                          }}>
                            <PencilIcon size={12} />
                          </button>
                          <button onClick={() => handleDelete(ann.id)} disabled={deleteMut.isPending} aria-label="Delete annotation" style={{
                            ...btnStyle, padding: '4px 8px', fontSize: 12, borderColor: 'var(--crit)', color: 'var(--crit)',
                            opacity: deleteMut.isPending ? 0.5 : 1,
                          }}>
                            <TrashIcon size={12} />
                          </button>
                        </div>
                      )}
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        )}
      </div>
    </>
  )

  if (embedded) {
    return <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>{content}</div>
  }

  return (
    <div role="dialog" aria-label="Annotations Panel" style={{
      position: 'fixed', inset: 0, zIndex: 1000, background: 'rgba(0,0,0,.6)',
      display: 'flex', alignItems: 'center', justifyContent: 'center',
    }} onClick={onClose}>
      <div style={{
        position: 'relative', width: '92%', maxWidth: 1000, maxHeight: '88vh',
        background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 12,
        display: 'flex', flexDirection: 'column', overflow: 'hidden',
      }} onClick={e => e.stopPropagation()}>
        {content}
      </div>
    </div>
  )
}
