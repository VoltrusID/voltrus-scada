// Comments pinned to a screen's components (docs/collaboration.md), and who
// edits which screen. Shapes follow the server's design comments routes.
import { useQuery, type QueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'
import type { DesignSessionInfo } from '../api/websocket'

export interface DesignComment {
  id: number
  screen_id: string
  parent_id: number | null
  object_id: string | null
  object_type: string | null
  x: number
  y: number
  body: string
  author: string
  mentions: string[]
  resolved: boolean
  resolved_by: string | null
  resolved_at_ms: number | null
  created_at_ms: number
  updated_at_ms: number
}

export interface NewComment { body: string; object_id?: string | null; object_type?: string | null; x: number; y: number; parent_id?: number | null }

const sid = (screenId: string) => encodeURIComponent(screenId === 'all' ? '__overview' : screenId)
const base = (screenId: string) => `/api/v1/screens/${sid(screenId)}/comments`
const JSON_HEADERS = { 'Content-Type': 'application/json', 'X-Voltrus-Client': 'designer' }

export const commentKeys = {
  screen: (screenId: string) => [...queryKeys.design(screenId), 'comments'] as const,
  sessions: ['design', 'sessions'] as const,
}

export const fetchComments = (screenId: string) =>
  fetchJson<{ comments: DesignComment[] }>(`${base(screenId)}?resolved=1`).then(r => r.comments ?? [])

export async function postComment(screenId: string, c: NewComment): Promise<DesignComment> {
  const r = await fetchOk(base(screenId), { method: 'POST', headers: JSON_HEADERS, body: JSON.stringify(c) })
  return r.json()
}

export async function patchComment(screenId: string, id: number, p: { body?: string; resolved?: boolean }): Promise<DesignComment> {
  const r = await fetchOk(`${base(screenId)}/${id}`, { method: 'PATCH', headers: JSON_HEADERS, body: JSON.stringify(p) })
  return r.json()
}

export async function deleteComment(screenId: string, id: number): Promise<void> {
  await fetchOk(`${base(screenId)}/${id}`, { method: 'DELETE', headers: JSON_HEADERS })
}

export const fetchSessions = () =>
  fetchJson<{ sessions: DesignSessionInfo[] }>('/api/v1/design/sessions').then(r => r.sessions ?? [])

export function useComments(screenId: string, enabled = true) {
  return useQuery({ queryKey: commentKeys.screen(screenId), queryFn: () => fetchComments(screenId), enabled, retry: false })
}

export function useDesignSessions(enabled = true) {
  return useQuery({ queryKey: commentKeys.sessions, queryFn: fetchSessions, enabled, retry: false, refetchInterval: 30_000 })
}

/** Apply a live comment event to the cached list. */
export function applyCommentEvent(qc: QueryClient, screenId: string, event: string, comment: DesignComment) {
  qc.setQueryData<DesignComment[]>(commentKeys.screen(screenId), list => {
    const rest = (list ?? []).filter(c => c.id !== comment.id)
    return event === 'deleted' ? rest.filter(c => c.parent_id !== comment.id) : [...rest, comment]
  })
}

/** Threads: roots newest first, each with its replies oldest first. */
export function threadsOf(comments: DesignComment[]): { root: DesignComment; replies: DesignComment[] }[] {
  const roots = comments.filter(c => c.parent_id == null).sort((a, b) => b.created_at_ms - a.created_at_ms)
  return roots.map(root => ({ root, replies: comments.filter(c => c.parent_id === root.id).sort((a, b) => a.created_at_ms - b.created_at_ms) }))
}

/** `@name` tokens of a body that name one of `users`. */
export function mentionsIn(body: string, users: string[]): string[] {
  const known = new Set(users)
  const out = new Set<string>()
  for (const m of body.matchAll(/(^|[^\w@])@([\w.-]+)/g)) if (known.has(m[2])) out.add(m[2])
  return [...out]
}
