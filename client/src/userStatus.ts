// Disable/Enable rules of the Users page, mirroring the server's checks in
// src/api/auth/users.rs so the button explains a refusal before asking.

export interface UserRow {
  id: number
  username: string
  role: string
  disabled?: boolean
}

/**
 * Why `me` may not disable `user`, or null when they may. Enabling is always
 * allowed. The server refuses disabling yourself and disabling the last
 * enabled admin.
 */
export function disableRefused(me: string, user: UserRow, users: UserRow[]): string | null {
  if (user.disabled) return null
  if (me && user.username === me) return 'You cannot disable yourself'
  if (user.role === 'admin') {
    const otherAdmins = users.filter(u => u.id !== user.id && u.role === 'admin' && !u.disabled).length
    if (otherAdmins === 0) return `${user.username} is the last enabled admin`
  }
  return null
}

/** The `PUT /api/v1/users/{id}` body that flips the account's state. */
export function toggleDisabledBody(user: UserRow): { disabled: boolean } {
  return { disabled: !user.disabled }
}
