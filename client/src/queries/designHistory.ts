// Version history of a 2D screen (docs/designer.md, phase 3): the change log,
// blame, versions, diff, revert, and draft/publish with an optional approval
// step. Shapes follow src/api/design_history.rs.
import { useInfiniteQuery, useQuery, useQueryClient, type QueryClient } from '@tanstack/react-query'
import type { ScadaConnectionItem, ScadaEquipmentItem } from '../types'
import { queryKeys } from './keys'
import { fetchJson, fetchSigned, fetchWithRefresh } from './fetch'
import type { ChangeSince } from './scada'

export type ChangeKind = 'add' | 'update' | 'remove'
export type ChangeSource = 'user' | 'designer' | 'native_designer' | 'editor' | 'api_key' | 'mcp' | 'copilot' | 'flow' | 'system'

/** One row of the change log: one object changed by one batch. */
export interface DesignChange {
  seq: number
  batch: string
  screen_id: string
  object_type: string
  object_id: string
  kind: ChangeKind
  before: Record<string, any> | null
  after: Record<string, any> | null
  username: string
  source: ChangeSource | string
  operation: string
  note: string
  at_ms: number
  squashed: number
}

/** `floor`: changes at or below it can no longer be previewed or reverted to by seq (retention). */
export interface HistoryPage { changes: DesignChange[]; next_before_seq: number | null; users: string[]; floor?: number }

export interface HistoryFilter { object?: string | null; objectType?: string | null; user?: string | null; batch?: string | null }

export interface BlameEntry {
  object_type: string
  object_id: string
  exists: boolean
  seq: number
  batch: string
  kind: ChangeKind
  username: string
  source: string
  operation: string
  note: string
  at_ms: number
  changes: number
  created_by: string | null
  created_at_ms: number | null
}

export type VersionStatus = 'saved' | 'pending' | 'published' | 'approved' | 'rejected'

export interface DesignVersion {
  id: number
  screen_id: string
  name: string
  message: string
  upto_seq: number
  username: string
  at_ms: number
  status: VersionStatus | string
  requested_by: string | null
  reviewed_by: string | null
  review_note: string | null
  reviewed_at_ms: number | null
  published_at_ms: number | null
  published_by: string | null
  signature: string | null
  current: boolean
  widgets: number
  connections: number
}

export interface DesignStatus {
  screen_id: string
  seq: number
  revision: string
  publishing: boolean
  approval: 'none' | 'required'
  published: null | { version_id: number; name: string | null; upto_seq: number; at_ms: number; by: string; draft_differs: boolean }
  changes_since_published: number
  last_version: DesignVersion | null
  changes_since_version: number
  pending: DesignVersion[]
}

export interface DesignSettings {
  publishing: boolean
  approval: 'none' | 'required'
  squash_window_secs: number
  squash_after_days: number
  keep_days: number
  /** A change that relaxes approval, waiting for a second admin. */
  pending_request?: { settings: Omit<DesignSettings, 'pending_request'>; requested_by: string; at_ms: number } | null
}

export interface LayoutRows { equipment: ScadaEquipmentItem[]; connections: ScadaConnectionItem[] }

export interface StateRows extends LayoutRows { state: Record<string, any> }

export interface DiffSide<T> {
  added: T[]
  removed: T[]
  changed: { id: string; fields: string[]; before: T; after: T }[]
}

export interface ScreenDiff {
  from: Record<string, any>
  to: Record<string, any>
  diff: {
    widgets: DiffSide<ScadaEquipmentItem>
    connections: DiffSide<ScadaConnectionItem>
    summary: { added: number; removed: number; changed: number }
  }
  yaml?: { from: string; to: string }
  rows?: { from: LayoutRows; to: LayoutRows }
}

/** A revert: whole screen (`to` / `to_seq`), one object, or several in one batch. */
export interface RevertBody {
  to?: string
  to_seq?: number
  object?: string
  object_type?: 'widget' | 'connection'
  objects?: { object_id: string; object_type?: 'widget' | 'connection'; to_seq?: number; to?: string }[]
  base_seq?: number | null
  base_revision?: string | null
  message?: string
}

export type RevertResult =
  | { ok: true; batch: string | null; seq: number; revision: string; removed_connections: string[]; ungrouped: string[]; missing_tags: number[] }
  | { ok: false; code: 'dangling_connection'; message: string; connection: string; missing: string[] }
  | { ok: false; code: 'conflict'; message: string; seq: number | null; revision: string | null; changesSince: ChangeSince[] }
  | { ok: false; code: 'not_found' | 'state_gone' | 'error'; message: string }

export type PublishResult =
  | { ok: true; status: 'published' | 'pending'; version: DesignVersion }
  | { ok: false; code: string; message: string; seq?: number | null; changesSince?: ChangeSince[] }

const sid = (screenId: string) => encodeURIComponent(screenId === 'all' ? '__overview' : screenId)
const base = (screenId: string) => `/api/v1/screens/${sid(screenId)}`
const JSON_HEADERS = { 'Content-Type': 'application/json', 'X-Voltrus-Client': 'designer' }

export const designKeys = {
  all: (screenId: string) => queryKeys.design(screenId),
  status: (screenId: string) => [...queryKeys.design(screenId), 'status'] as const,
  history: (screenId: string, f: HistoryFilter) => [...queryKeys.design(screenId), 'history', f.object ?? '', f.objectType ?? '', f.user ?? '', f.batch ?? ''] as const,
  blame: (screenId: string) => [...queryKeys.design(screenId), 'blame'] as const,
  versions: (screenId: string) => [...queryKeys.design(screenId), 'versions'] as const,
  state: (screenId: string, at: string) => [...queryKeys.design(screenId), 'state', at] as const,
  diff: (screenId: string, from: string, to: string, yaml: boolean) => [...queryKeys.design(screenId), 'diff', from, to, yaml] as const,
}

// ── Fetchers ──

export function historyUrl(screenId: string, f: HistoryFilter, beforeSeq?: number | null, limit = 100): string {
  const q = new URLSearchParams()
  if (f.object) q.set('object', f.object)
  if (f.objectType) q.set('object_type', f.objectType)
  if (f.user) q.set('user', f.user)
  if (f.batch) q.set('batch', f.batch)
  if (beforeSeq != null) q.set('before_seq', String(beforeSeq))
  q.set('limit', String(limit))
  return `${base(screenId)}/history?${q}`
}

export const fetchHistory = (screenId: string, f: HistoryFilter, beforeSeq?: number | null, limit?: number) =>
  fetchJson<HistoryPage>(historyUrl(screenId, f, beforeSeq, limit))
export const fetchBlame = (screenId: string) => fetchJson<{ objects: BlameEntry[] }>(`${base(screenId)}/blame`).then(r => r.objects)
export const fetchDesignStatus = (screenId: string) => fetchJson<DesignStatus>(`${base(screenId)}/design-status`)
export const fetchVersions = (screenId: string) => fetchJson<{ versions: DesignVersion[] }>(`${base(screenId)}/versions`).then(r => r.versions)
export const fetchVersion = (screenId: string, v: string | number) =>
  fetchJson<{ version: DesignVersion } & LayoutRows>(`${base(screenId)}/versions/${encodeURIComponent(String(v))}`)
export const fetchState = (screenId: string, at: string) => fetchJson<StateRows>(`${base(screenId)}/state?at=${encodeURIComponent(at)}`)
export const fetchDiff = (screenId: string, from: string, to: string, yaml = true) =>
  fetchJson<ScreenDiff>(`${base(screenId)}/diff?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}${yaml ? '&yaml=1' : '&yaml=0'}`)
export const fetchDesignSettings = () => fetchJson<DesignSettings>('/api/v1/settings/design-history')

async function bodyOf(r: Response): Promise<any> {
  return r.json().catch(() => ({}))
}

/** Record the current draft as a named version. `name_taken` comes back as an error message. */
export async function createVersion(screenId: string, input: { name?: string; message?: string }): Promise<DesignVersion> {
  const r = await fetchWithRefresh(`${base(screenId)}/versions`, {
    method: 'POST', headers: JSON_HEADERS,
    body: JSON.stringify({ ...(input.name?.trim() ? { name: input.name.trim() } : {}), message: input.message?.trim() ?? '' }),
  })
  const b = await bodyOf(r)
  if (!r.ok) throw new Error(b.error || `Request failed: ${r.status}`)
  return b.version
}

/** Revert the screen or some of its objects. Refusals the UI answers (dangling, conflict) are results, not errors. */
export async function revertScreen(screenId: string, body: RevertBody): Promise<RevertResult> {
  const r = await fetchSigned(`${base(screenId)}/revert`, { method: 'POST', headers: JSON_HEADERS, body: JSON.stringify(body) })
  const b = await bodyOf(r)
  if (r.ok) {
    return {
      ok: true, batch: b.batch ?? null, seq: b.seq, revision: b.revision,
      removed_connections: b.removed_connections ?? [], ungrouped: b.ungrouped ?? [], missing_tags: b.missing_tags ?? [],
    }
  }
  const message = b.error || `Request failed: ${r.status}`
  if (r.status === 409 && b.code === 'dangling_connection') {
    return { ok: false, code: 'dangling_connection', message, connection: b.connection, missing: b.missing ?? [] }
  }
  if (r.status === 409 && b.code === 'conflict') {
    return { ok: false, code: 'conflict', message, seq: b.seq ?? null, revision: b.revision ?? null, changesSince: b.changes_since ?? [] }
  }
  // 410: the seq was squashed or pruned away (reload the history).
  return { ok: false, code: r.status === 404 ? 'not_found' : r.status === 410 ? 'state_gone' : 'error', message }
}

/**
 * Publish the draft (recorded as a new version) or a version, or file a
 * publish request when the site requires approval. A 428 opens the signature
 * prompt (fetchSigned); cancelling it throws SignatureCancelledError.
 */
export async function publishScreen(
  screenId: string, input: { version?: string | number; name?: string; message?: string; baseSeq?: number | null },
): Promise<PublishResult> {
  const body: Record<string, unknown> = { message: input.message?.trim() ?? '' }
  if (input.version != null) body.version = String(input.version)
  if (input.name?.trim()) body.name = input.name.trim()
  if (input.baseSeq != null) body.base_seq = input.baseSeq
  const r = await fetchSigned(`${base(screenId)}/publish`, { method: 'POST', headers: JSON_HEADERS, body: JSON.stringify(body) })
  const b = await bodyOf(r)
  if (r.ok) return { ok: true, status: b.status === 'pending' ? 'pending' : 'published', version: b.version }
  return {
    ok: false, code: b.code || String(r.status), message: b.error || `Request failed: ${r.status}`,
    seq: b.seq ?? null, changesSince: b.changes_since,
  }
}

/** Approve (publishes it) or reject a pending publish request. Admin session, not the requester. */
export async function reviewVersion(
  screenId: string, version: string | number, approve: boolean, note = '',
): Promise<{ status: string; version: DesignVersion }> {
  const r = await fetchSigned(`${base(screenId)}/versions/${encodeURIComponent(String(version))}/${approve ? 'approve' : 'reject'}`, {
    method: 'POST', headers: JSON_HEADERS, body: JSON.stringify({ note: note.trim() }),
  })
  const b = await bodyOf(r)
  if (!r.ok) throw new Error(b.error || `Request failed: ${r.status}`)
  return b
}

// ── Hooks ──

export function useDesignSettings(enabled = true) {
  return useQuery({ queryKey: queryKeys.designSettings, queryFn: fetchDesignSettings, enabled, staleTime: 60_000 })
}

export function useDesignStatus(screenId: string, enabled = true) {
  return useQuery({ queryKey: designKeys.status(screenId), queryFn: () => fetchDesignStatus(screenId), enabled, staleTime: 0 })
}

/** The change log, newest first, a page at a time (`fetchNextPage` for older ones). */
export function useScreenHistory(screenId: string, f: HistoryFilter, enabled = true, limit = 100) {
  return useInfiniteQuery({
    queryKey: designKeys.history(screenId, f),
    queryFn: ({ pageParam }) => fetchHistory(screenId, f, pageParam, limit),
    initialPageParam: null as number | null,
    getNextPageParam: last => last.next_before_seq ?? undefined,
    enabled,
  })
}

export function useBlame(screenId: string, enabled = true) {
  return useQuery({ queryKey: designKeys.blame(screenId), queryFn: () => fetchBlame(screenId), enabled })
}

export function useVersions(screenId: string, enabled = true) {
  return useQuery({ queryKey: designKeys.versions(screenId), queryFn: () => fetchVersions(screenId), enabled })
}

/** The rows of a past state (`seq:<n>`, a version, `published`, `draft`); `null` fetches nothing. */
export function useScreenState(screenId: string, at: string | null) {
  return useQuery({
    queryKey: designKeys.state(screenId, at ?? ''),
    queryFn: () => fetchState(screenId, at!),
    enabled: at != null,
    // A seq state can disappear (squashed or pruned: 410), so it is not cached for ever.
    staleTime: at?.startsWith('seq:') ? 30_000 : 0,
  })
}

export function useScreenDiff(screenId: string, from: string | null, to: string | null, yaml = true) {
  return useQuery({
    queryKey: designKeys.diff(screenId, from ?? '', to ?? '', yaml),
    queryFn: () => fetchDiff(screenId, from!, to!, yaml),
    enabled: from != null && to != null,
    staleTime: 0,
  })
}

/** Refetch everything version history knows about a screen (after a save, revert, publish). */
export function invalidateDesign(qc: QueryClient, screenId: string) {
  return qc.invalidateQueries({ queryKey: queryKeys.design(screenId) })
}

export function useInvalidateDesign() {
  const qc = useQueryClient()
  return (screenId: string) => invalidateDesign(qc, screenId)
}
