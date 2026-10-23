// Global in-flight refresh promise to prevent multiple simultaneous refresh attempts
let refreshing: Promise<boolean> | null = null

async function doRefresh(): Promise<boolean> {
  if (refreshing) {
    return refreshing
  }
  refreshing = (async () => {
    try {
      const r = await fetch('/api/v1/auth/refresh', {
        method: 'POST',
        credentials: 'include',
      })
      return r.ok
    } catch {
      return false
    } finally {
      refreshing = null
    }
  })()
  return refreshing
}

async function fetchWithRefresh(
  url: string,
  init?: RequestInit,
): Promise<Response> {
  const r = await fetch(url, { credentials: 'include', ...init })
  if (r.status === 401) {
    const ok = await doRefresh()
    if (ok) {
      return fetch(url, { credentials: 'include', ...init })
    }
    // Don't hard-redirect — throw so React Query handles it gracefully.
    // The App component's auth gate will show the login screen.
    throw new Error('Session expired')
  }
  return r
}

export async function fetchJson<T>(url: string): Promise<T> {
  const r = await fetchWithRefresh(url)
  if (!r.ok) throw new Error(`fetch ${url} failed: ${r.status}`)
  return r.json()
}

export async function fetchOk(url: string, init?: RequestInit): Promise<Response> {
  const r = await fetchWithRefresh(url, init)
  if (!r.ok) {
    const body = await r.json().catch(() => ({}))
    throw new Error(body.error || `Request failed: ${r.status}`)
  }
  return r
}
