// A shared account (`shared_accounts` in the server config, e.g. a public
// demo's `guest`) is one login many people use. The server refuses changes to
// its password and preferences, so the client keeps theme and language
// choices in this browser only instead of saving them.

let shared = false

export function setSharedAccount(value: boolean): void {
  shared = value
}

export function isSharedAccount(): boolean {
  return shared
}

/** Longest hint the server accepts (`login_hint` in the config). */
export const LOGIN_HINT_MAX_CHARS = 200

/**
 * The sign-in hint from `GET /api/v1/auth/providers`, or null. Shown as plain
 * text; anything that is not a non-blank string is ignored.
 */
export function loginHintFrom(body: unknown): string | null {
  if (!body || typeof body !== 'object') return null
  const hint = (body as { login_hint?: unknown }).login_hint
  if (typeof hint !== 'string') return null
  const trimmed = hint.trim()
  if (!trimmed) return null
  return [...trimmed].slice(0, LOGIN_HINT_MAX_CHARS).join('')
}
