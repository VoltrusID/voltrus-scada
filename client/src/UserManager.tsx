import { useState, useEffect } from 'react'
import { useUsers, useCreateUser, useUpdateUser, useDeleteUser, useRoles } from './queries'
import { inputStyle, btnStyle } from './styles'
import type { UserItem } from './types'

const ROLE_COLORS: Record<string, { bg: string; color: string }> = {
  admin: { bg: 'rgba(168,85,247,.15)', color: '#a855f7' },
  demo: { bg: 'rgba(59,130,246,.15)', color: '#3b82f6' },
  operator: { bg: 'rgba(34,197,94,.1)', color: 'var(--green)' },
  viewer: { bg: 'rgba(156,163,175,.1)', color: 'var(--muted)' },
}

export function UserManager({ onClose, embedded }: { onClose: () => void; embedded?: boolean }) {
  const { data: users = [], isLoading } = useUsers(true)
  const { data: roles = [] } = useRoles()
  const createMut = useCreateUser()
  const updateMut = useUpdateUser()
  const deleteMut = useDeleteUser()

  const [showAdd, setShowAdd] = useState(false)
  const [editUser, setEditUser] = useState<UserItem | null>(null)
  const [confirmDelete, setConfirmDelete] = useState<UserItem | null>(null)

  useEffect(() => {
    if (embedded) return
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, embedded])

  const thStyle: React.CSSProperties = { textAlign: 'left', padding: '8px 12px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600, borderBottom: '1px solid var(--border)' }
  const tdStyle: React.CSSProperties = { padding: '8px 12px', fontSize: 13, borderBottom: '1px solid var(--border)' }

  const handleCreate = (data: { username: string; password: string; role: string }) => {
    createMut.mutate(data, { onSuccess: () => setShowAdd(false) })
  }

  const handleUpdate = (id: number, data: { role?: string; password?: string }) => {
    updateMut.mutate({ id, ...data }, { onSuccess: () => setEditUser(null) })
  }

  const handleDelete = (id: number) => {
    deleteMut.mutate(id, { onSuccess: () => setConfirmDelete(null) })
  }

  const tableContent = (
    <>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16, flexShrink: 0 }}>
        <span style={{ color: 'var(--muted)', fontSize: 13 }}>{users.length} users</span>
        <button style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }} onClick={() => setShowAdd(true)} aria-label="Add new user">+ Add User</button>
      </div>
      <div style={{ flex: 1, overflow: 'auto', background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10 }}>
        {isLoading ? (
          <div style={{ color: 'var(--muted)', textAlign: 'center', padding: 40 }}>Loading...</div>
        ) : users.length === 0 ? (
          <div style={{ color: 'var(--muted)', textAlign: 'center', padding: 40 }}>No users found.</div>
        ) : (
        <table style={{ width: '100%', borderCollapse: 'collapse' }}>
          <thead>
            <tr>
              <th style={thStyle}>Username</th>
              <th style={thStyle}>Role</th>
              <th style={{ ...thStyle, width: 180 }}>Actions</th>
            </tr>
          </thead>
          <tbody>
            {users.map(u => (
              <tr key={u.id}>
                <td style={tdStyle}>
                  <span style={{ fontWeight: 600 }}>{u.username}</span>
                </td>
                <td style={tdStyle}>
                  <span style={{
                    display: 'inline-block', padding: '2px 8px', borderRadius: 4, fontSize: 11, fontWeight: 600,
                    ...(ROLE_COLORS[u.role] || { bg: 'rgba(34,197,94,.1)', color: 'var(--green)' }),
                    background: (ROLE_COLORS[u.role] || { bg: 'rgba(34,197,94,.1)' }).bg,
                    color: (ROLE_COLORS[u.role] || { color: 'var(--green)' }).color,
                  }}>
                    {u.role}
                  </span>
                </td>
                <td style={tdStyle}>
                  <div style={{ display: 'flex', gap: 6 }}>
                    <button style={{ ...btnStyle, fontSize: 11, padding: '4px 10px' }} onClick={() => setEditUser(u)} aria-label={`Edit user ${u.username}`}>Edit</button>
                    <button style={{ ...btnStyle, fontSize: 11, padding: '4px 10px', borderColor: 'var(--crit)', color: 'var(--crit)' }} onClick={() => setConfirmDelete(u)} aria-label={`Delete user ${u.username}`}>Delete</button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        )}
      </div>

      {showAdd && <AddUserDialog onSave={handleCreate} onClose={() => setShowAdd(false)} error={createMut.error?.message} roles={roles} />}
      {editUser && <EditUserDialog user={editUser} onSave={handleUpdate} onClose={() => setEditUser(null)} error={updateMut.error?.message} roles={roles} />}
      {confirmDelete && (
        <div style={{ position: 'fixed', inset: 0, zIndex: 60, background: 'rgba(0,0,0,.6)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 12, padding: 24, maxWidth: 400, width: '90%' }}>
            <h3 style={{ fontSize: 16, fontWeight: 700, marginBottom: 8 }}>Delete User</h3>
            <p style={{ fontSize: 13, color: 'var(--muted)', marginBottom: 16 }}>
              Delete user <strong style={{ color: 'var(--fg)' }}>{confirmDelete.username}</strong>? This cannot be undone.
            </p>
            {deleteMut.error && <p style={{ fontSize: 12, color: 'var(--crit)', marginBottom: 8 }}>{deleteMut.error.message}</p>}
            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
              <button style={btnStyle} onClick={() => setConfirmDelete(null)}>Cancel</button>
              <button style={{ ...btnStyle, borderColor: 'var(--crit)', color: 'var(--crit)' }} onClick={() => handleDelete(confirmDelete.id)}>Delete</button>
            </div>
          </div>
        </div>
      )}
    </>
  )

  if (embedded) return <div style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}>{tableContent}</div>

  return (
    <div style={{ position: 'fixed', inset: 0, zIndex: 50, background: 'var(--bg)', display: 'flex', flexDirection: 'column', fontFamily: 'var(--font-ui)' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '16px 24px', borderBottom: '1px solid var(--border)', flexShrink: 0 }}>
        <button onClick={onClose} style={{ ...btnStyle, padding: '4px 8px' }} aria-label="Close user management">&larr; Back</button>
        <h2 style={{ fontSize: 18, fontWeight: 700 }}>User Management</h2>
      </div>
      <div style={{ flex: 1, overflow: 'auto', padding: '16px 24px', display: 'flex', flexDirection: 'column' }}>
        {tableContent}
      </div>
    </div>
  )
}

function AddUserDialog({ onSave, onClose, error, roles }: { onSave: (d: { username: string; password: string; role: string }) => void; onClose: () => void; error?: string; roles: { id: number; name: string }[] }) {
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [role, setRole] = useState(roles.length > 0 ? roles[0].name.toLowerCase() : 'viewer')

  return (
    <div style={{ position: 'fixed', inset: 0, zIndex: 60, background: 'rgba(0,0,0,.6)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 12, padding: 24, maxWidth: 400, width: '90%' }}>
        <h3 style={{ fontSize: 16, fontWeight: 700, marginBottom: 16 }}>Add User</h3>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <input placeholder="Username" value={username} onChange={e => setUsername(e.target.value)} style={inputStyle} autoFocus />
          <input placeholder="Password" type="password" value={password} onChange={e => setPassword(e.target.value)} style={inputStyle} />
          <select value={role} onChange={e => setRole(e.target.value)} style={inputStyle}>
            {roles.map(r => (
              <option key={r.id} value={r.name.toLowerCase()}>{r.name}</option>
            ))}
          </select>
          {error && <p style={{ fontSize: 12, color: 'var(--crit)' }}>{error}</p>}
          <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
            <button style={btnStyle} onClick={onClose}>Cancel</button>
            <button style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }} onClick={() => onSave({ username, password, role })} disabled={!username || !password}>Create</button>
          </div>
        </div>
      </div>
    </div>
  )
}

function EditUserDialog({ user, onSave, onClose, error, roles }: { user: UserItem; onSave: (id: number, d: { role?: string; password?: string }) => void; onClose: () => void; error?: string; roles: { id: number; name: string }[] }) {
  const [role, setRole] = useState(user.role)
  const [password, setPassword] = useState('')

  return (
    <div style={{ position: 'fixed', inset: 0, zIndex: 60, background: 'rgba(0,0,0,.6)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 12, padding: 24, maxWidth: 400, width: '90%' }}>
        <h3 style={{ fontSize: 16, fontWeight: 700, marginBottom: 16 }}>Edit User: {user.username}</h3>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <label style={{ fontSize: 12, color: 'var(--muted)' }}>Role</label>
          <select value={role} onChange={e => setRole(e.target.value)} style={inputStyle}>
            {roles.map(r => (
              <option key={r.id} value={r.name.toLowerCase()}>{r.name}</option>
            ))}
          </select>
          <label style={{ fontSize: 12, color: 'var(--muted)' }}>New Password (leave blank to keep current)</label>
          <input placeholder="New password" type="password" value={password} onChange={e => setPassword(e.target.value)} style={inputStyle} />
          {error && <p style={{ fontSize: 12, color: 'var(--crit)' }}>{error}</p>}
          <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
            <button style={btnStyle} onClick={onClose}>Cancel</button>
            <button style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }} onClick={() => {
              const data: { role?: string; password?: string } = {}
              if (role !== user.role) data.role = role
              if (password) data.password = password
              if (Object.keys(data).length > 0) onSave(user.id, data)
              else onClose()
            }}>Save</button>
          </div>
        </div>
      </div>
    </div>
  )
}
