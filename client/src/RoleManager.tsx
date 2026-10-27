import { useState, useEffect } from 'react'
import { useRoles, useCreateRole, useUpdateRole, useDeleteRole } from './queries'
import type { Role } from './queries'
import { btnStyle, inputStyle } from './styles'
import { ALL_PERMISSIONS } from './permissions'

function parsePermissions(raw: string): string[] {
  try {
    const parsed = JSON.parse(raw)
    if (Array.isArray(parsed)) return parsed.filter((p: unknown) => typeof p === 'string')
    return []
  } catch {
    return []
  }
}

export function RoleManager({ onClose, embedded }: { onClose: () => void; embedded?: boolean }) {
  const { data: roles = [], isLoading } = useRoles()
  const createMut = useCreateRole()
  const updateMut = useUpdateRole()
  const deleteMut = useDeleteRole()

  const [showAdd, setShowAdd] = useState(false)
  const [editRole, setEditRole] = useState<Role | null>(null)
  const [confirmDelete, setConfirmDelete] = useState<Role | null>(null)

  useEffect(() => {
    if (embedded) return
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, embedded])

  const handleCreate = (data: { name: string; permissions: string }) => {
    createMut.mutate(data, { onSuccess: () => setShowAdd(false) })
  }

  const handleUpdate = (id: number, data: { name: string; permissions: string }) => {
    updateMut.mutate({ id, ...data }, { onSuccess: () => setEditRole(null) })
  }

  const handleDelete = (id: number) => {
    deleteMut.mutate(id, { onSuccess: () => setConfirmDelete(null) })
  }

  const thStyle: React.CSSProperties = {
    textAlign: 'left', padding: '8px 12px', fontSize: 11, color: 'var(--muted)',
    textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600,
    borderBottom: '1px solid var(--border)',
  }
  const tdStyle: React.CSSProperties = {
    padding: '8px 12px', fontSize: 13, borderBottom: '1px solid var(--border)',
  }

  const content = (
    <>
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        padding: '16px 24px', borderBottom: '1px solid var(--border)', flexShrink: 0,
      }}>
        <h2 style={{ fontSize: 18, fontWeight: 700 }}>Role Management</h2>
        {!embedded && <button onClick={onClose} style={{ ...btnStyle, padding: '4px 10px' }} aria-label="Close role manager">&times;</button>}
      </div>

        {/* Content */}
        <div style={{ flex: 1, overflow: 'auto', padding: '20px 24px', display: 'flex', flexDirection: 'column', gap: 16 }}>
          {/* Section header */}
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', flexShrink: 0 }}>
            <h3 style={{
              fontSize: 13, fontWeight: 700, textTransform: 'uppercase', letterSpacing: '.05em',
              color: 'var(--fg)', paddingLeft: 12,
              borderLeft: '4px solid var(--cyan)',
            }}>
              Roles ({roles.length})
            </h3>
            <button
              style={{ ...btnStyle, borderColor: 'var(--cyan)', color: 'var(--cyan)' }}
              onClick={() => setShowAdd(true)}
            >
              + Add Role
            </button>
          </div>

          {/* Add role inline form */}
          {showAdd && (
            <RoleForm
              onSave={handleCreate}
              onClose={() => setShowAdd(false)}
              loading={createMut.isPending}
              error={createMut.error?.message}
              submitLabel="Create"
            />
          )}

          {/* Table */}
          <div style={{
            flex: 1, overflow: 'auto',
            background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10,
          }}>
            {isLoading ? (
              <div style={{ color: 'var(--muted)', textAlign: 'center', padding: 40 }}>Loading...</div>
            ) : roles.length === 0 ? (
              <div style={{ color: 'var(--muted)', textAlign: 'center', padding: 40 }}>No roles defined.</div>
            ) : (
              <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                <thead>
                  <tr>
                    <th style={thStyle}>Name</th>
                    <th style={thStyle}>Permissions</th>
                    <th style={{ ...thStyle, width: 140 }}>Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {roles.map(role => {
                    const perms = parsePermissions(role.permissions)
                    const isEditing = editRole?.id === role.id

                    return isEditing ? (
                      <tr key={role.id}>
                        <td colSpan={3} style={{ padding: 0, borderBottom: '1px solid var(--border)' }}>
                          <RoleForm
                            initialName={role.name}
                            initialPermissions={role.permissions}
                            onSave={(data) => handleUpdate(role.id, data)}
                            onClose={() => setEditRole(null)}
                            loading={updateMut.isPending}
                            error={updateMut.error?.message}
                            submitLabel="Save"
                          />
                        </td>
                      </tr>
                    ) : (
                      <tr key={role.id}>
                        <td style={tdStyle}>
                          <span style={{ fontWeight: 600 }}>{role.name}</span>
                        </td>
                        <td style={tdStyle}>
                          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
                            {perms.length === 0 ? (
                              <span style={{ fontSize: 12, color: 'var(--muted)', fontStyle: 'italic' }}>No permissions</span>
                            ) : (
                              perms.map(p => (
                                <span
                                  key={p}
                                  style={{
                                    display: 'inline-block', padding: '2px 8px', borderRadius: 4,
                                    fontSize: 11, fontWeight: 600,
                                    fontFamily: 'var(--font-mono)',
                                    background: 'rgba(34,211,238,.08)', color: 'var(--cyan)',
                                    border: '1px solid rgba(34,211,238,.15)',
                                  }}
                                >
                                  {p}
                                </span>
                              ))
                            )}
                          </div>
                        </td>
                        <td style={tdStyle}>
                          <div style={{ display: 'flex', gap: 6 }}>
                            <button
                              style={{ ...btnStyle, fontSize: 11, padding: '4px 10px' }}
                              onClick={() => setEditRole(role)}
                              aria-label={`Edit role ${role.name}`}
                            >
                              Edit
                            </button>
                            <button
                              style={{ ...btnStyle, fontSize: 11, padding: '4px 10px', borderColor: 'var(--crit)', color: 'var(--crit)' }}
                              onClick={() => setConfirmDelete(role)}
                              aria-label={`Delete role ${role.name}`}
                            >
                              Delete
                            </button>
                          </div>
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            )}
          </div>

          {/* Available permissions reference */}
          <div style={{ flexShrink: 0 }}>
            <h4 style={{
              fontSize: 11, fontWeight: 600, textTransform: 'uppercase', letterSpacing: '.05em',
              color: 'var(--muted)', marginBottom: 8, paddingLeft: 12,
              borderLeft: '4px solid var(--border)',
            }}>
              Available Permissions
            </h4>
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, paddingLeft: 12 }}>
              {ALL_PERMISSIONS.map(p => (
                <span
                  key={p}
                  style={{
                    display: 'inline-block', padding: '2px 8px', borderRadius: 4,
                    fontSize: 11, fontFamily: 'var(--font-mono)',
                    background: 'var(--card)', border: '1px solid var(--border)',
                    color: 'var(--muted)',
                  }}
                >
                  {p}
                </span>
              ))}
            </div>
          </div>
        </div>

        {/* Delete confirmation */}
        {confirmDelete && (
          <div style={{
            position: 'fixed', inset: 0, zIndex: 60,
            background: 'rgba(0,0,0,.6)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <div style={{
              background: 'var(--card)', border: '1px solid var(--border)',
              borderRadius: 12, padding: 24, maxWidth: 400, width: '90%',
            }}>
              <h3 style={{ fontSize: 16, fontWeight: 700, marginBottom: 8 }}>Delete Role</h3>
              <p style={{ fontSize: 13, color: 'var(--muted)', marginBottom: 16 }}>
                Delete role <strong style={{ color: 'var(--fg)' }}>{confirmDelete.name}</strong>? This cannot be undone.
                Users assigned this role will lose its permissions.
              </p>
              {deleteMut.error && (
                <p style={{ fontSize: 12, color: 'var(--crit)', marginBottom: 8 }}>{deleteMut.error.message}</p>
              )}
              <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
                <button style={btnStyle} onClick={() => setConfirmDelete(null)}>Cancel</button>
                <button
                  style={{ ...btnStyle, borderColor: 'var(--crit)', color: 'var(--crit)' }}
                  onClick={() => handleDelete(confirmDelete.id)}
                  disabled={deleteMut.isPending}
                >
                  {deleteMut.isPending ? 'Deleting...' : 'Delete'}
                </button>
              </div>
            </div>
          </div>
        )}
      </>
  )

  if (embedded) {
    return <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>{content}</div>
  }

  return (
    <div style={{
      position: 'fixed', inset: 0, zIndex: 50,
      background: 'rgba(0,0,0,.6)',
      display: 'flex', alignItems: 'center', justifyContent: 'center',
      fontFamily: 'var(--font-ui)',
    }}>
      <div style={{
        background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 12,
        width: '90%', maxWidth: 780, maxHeight: '90vh',
        display: 'flex', flexDirection: 'column', overflow: 'hidden',
      }}>
        {content}
      </div>
    </div>
  )
}

/* ── Inline role form (used for both add and edit) ──────────────── */

function RoleForm({ initialName, initialPermissions, onSave, onClose, loading, error, submitLabel }: {
  initialName?: string
  initialPermissions?: string
  onSave: (d: { name: string; permissions: string }) => void
  onClose: () => void
  loading?: boolean
  error?: string
  submitLabel: string
}) {
  const [name, setName] = useState(initialName ?? '')
  const [permissions, setPermissions] = useState(initialPermissions ?? '[]')

  const canSubmit = name.trim() && permissions.trim()

  return (
    <div style={{
      padding: '16px 20px',
      background: 'var(--card)',
      borderBottom: '1px solid var(--border)',
      display: 'flex', flexDirection: 'column', gap: 12,
    }}>
      <div style={{ display: 'flex', gap: 12, alignItems: 'flex-start', flexWrap: 'wrap' }}>
        <div style={{ flex: '1 1 180px', minWidth: 180 }}>
          <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <span style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.04em' }}>Name</span>
            <input
              placeholder="e.g. operator, supervisor"
              value={name}
              onChange={e => setName(e.target.value)}
              style={inputStyle}
              autoFocus
            />
          </label>
        </div>
        <div style={{ flex: '3 1 320px', minWidth: 280 }}>
          <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <span style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.04em' }}>
              Permissions (JSON array)
            </span>
            <textarea
              placeholder={'["screens:read", "devices:read", "alarms:read"]'}
              value={permissions}
              onChange={e => setPermissions(e.target.value)}
              style={{
                ...inputStyle,
                minHeight: 56, resize: 'vertical',
                fontFamily: 'var(--font-mono)', fontSize: 12,
              }}
            />
          </label>
        </div>
      </div>
      {error && <p style={{ fontSize: 12, color: 'var(--crit)' }}>{error}</p>}
      <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
        <button style={btnStyle} onClick={onClose} disabled={loading}>Cancel</button>
        <button
          style={{ ...btnStyle, borderColor: 'var(--cyan)', color: 'var(--cyan)', fontWeight: 600 }}
          onClick={() => onSave({ name: name.trim(), permissions })}
          disabled={!canSubmit || loading}
        >
          {loading ? `${submitLabel}...` : submitLabel}
        </button>
      </div>
    </div>
  )
}
