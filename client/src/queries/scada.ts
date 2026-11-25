import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ScadaEquipmentItem, ScadaConnectionItem } from '../types'
import type { FaceplateDef } from '../scada2/hmi/faceplateMap'
import { queryKeys } from './keys'
import { fetchJson, fetchOk, fetchSigned } from './fetch'

/**
 * A screen's rows as the 2D runtime or an editor reads them. `published`: what
 * operators see (the published version, or the draft of a screen never
 * published); otherwise the draft the editors change. The server always
 * answers published to roles that cannot configure.
 */
export function useScadaEquipment(screenId: string, enabled: boolean, opts: { published?: boolean } = {}) {
  const actualId = screenId === 'all' ? '__overview' : screenId
  const published = !!opts.published
  return useQuery({
    queryKey: queryKeys.scadaEquipment(screenId, published ? 'published' : 'draft'),
    queryFn: () => fetchJson<{ items: ScadaEquipmentItem[] }>(`/api/v1/screens/${actualId}/scada-equipment${published ? '?published=1' : ''}`).then(r => r.items),
    enabled,
  })
}

export function useScadaConnections(screenId: string, enabled: boolean, opts: { published?: boolean } = {}) {
  const actualId = screenId === 'all' ? '__overview' : screenId
  const published = !!opts.published
  return useQuery({
    queryKey: queryKeys.scadaConnections(screenId, published ? 'published' : 'draft'),
    queryFn: () => fetchJson<{ items: ScadaConnectionItem[] }>(`/api/v1/screens/${actualId}/scada-connections${published ? '?published=1' : ''}`).then(r => r.items),
    enabled,
  })
}

/**
 * The rows the 2D view and the 3D twin show: the published screen (what
 * operators see) at runtime, the draft while the 2D view's Edit mode is on.
 * `setEditing` is the 2D view's `onEditModeChange`.
 */
export function useScadaRows(screenId: string, enabled: boolean, viewMode: string) {
  const [editing, setEditing] = useState(false)
  const view = { published: !(editing && viewMode === 'scada') }
  const equipment = useScadaEquipment(screenId, enabled, view)
  const connections = useScadaConnections(screenId, enabled, view)
  return { equipment, connections, loaded: equipment.isFetched && connections.isFetched, setEditing }
}

export function useSaveScadaEquipment() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ screenId, items }: { screenId: string; items: ScadaEquipmentItem[] }) => {
      const actualId = screenId === 'all' ? '__overview' : screenId
      await fetchOk(`/api/v1/screens/${actualId}/scada-equipment`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ items }),
      })
    },
    // Returned so mutateAsync resolves only after the refetch lands.
    onSuccess: (_, { screenId }) => qc.invalidateQueries({ queryKey: queryKeys.scadaEquipment(screenId) }),
  })
}

export function useSaveScadaConnections() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ screenId, items }: { screenId: string; items: ScadaConnectionItem[] }) => {
      const actualId = screenId === 'all' ? '__overview' : screenId
      await fetchOk(`/api/v1/screens/${actualId}/scada-connections`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ items }),
      })
    },
    onSuccess: (_, { screenId }) => qc.invalidateQueries({ queryKey: queryKeys.scadaConnections(screenId) }),
  })
}

/**
 * A screen's 2D layout with its revision and change-log head `seq` (what the
 * Designer edits). `view` says which rows came back: `draft`, or `published`
 * when asked for the operators' view of a published screen.
 */
export interface ScadaLayout {
  equipment: ScadaEquipmentItem[]
  connections: ScadaConnectionItem[]
  revision: string
  seq?: number
  view?: 'draft' | 'published'
}

const layoutId = (screenId: string) => (screenId === 'all' ? '__overview' : screenId)

/**
 * The draft layout, its revision and seq, read together. Refetched when the
 * window gets focus again, so the Designer learns about a save made elsewhere.
 */
export const fetchScadaLayout = (screenId: string) => fetchJson<ScadaLayout>(`/api/v1/screens/${layoutId(screenId)}/scada-layout`)

export function useScadaLayout(screenId: string, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.scadaLayout(screenId),
    queryFn: () => fetchScadaLayout(screenId),
    enabled,
    staleTime: 0,
    refetchOnWindowFocus: true,
  })
}

/** One batch of changes made since the base a save was sent with (409 `changes_since`). */
export interface ChangeSince {
  batch: string
  username: string
  source: string
  operation: string
  note: string
  at_ms: number
  objects: { object_type: string; object_id: string; kind: 'add' | 'update' | 'remove'; seq: number }[]
}

export type LayoutSaveResult =
  | { ok: true; revision: string; seq: number | null; batch: string | null }
  | { ok: false; conflict: true; revision: string; seq: number | null; message: string; changesSince: ChangeSince[] }

/** What a save is based on: the revision (rows hash) and the change-log head the editor loaded. */
export interface LayoutBase { revision: string | null; seq: number | null }

/**
 * Save equipment and connections in one request, recorded in the change log
 * as made in the Designer. With a base the server refuses (409) when the
 * screen changed since; the result then carries the current revision and seq
 * and who changed what since, instead of throwing.
 */
export async function saveScadaLayout(
  screenId: string, equipment: ScadaEquipmentItem[], connections: ScadaConnectionItem[], base: string | null | LayoutBase,
): Promise<LayoutSaveResult> {
  const b: LayoutBase = base == null || typeof base === 'string' ? { revision: base ?? null, seq: null } : base
  const r = await fetchSigned(`/api/v1/screens/${layoutId(screenId)}/scada-layout`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json', 'X-Voltrus-Client': 'designer' },
    body: JSON.stringify({ equipment, connections, base_revision: b.revision, base_seq: b.seq }),
  })
  const body = await r.json().catch(() => ({}))
  if (r.status === 409 && typeof body.revision === 'string') {
    return {
      ok: false, conflict: true, revision: body.revision, seq: typeof body.seq === 'number' ? body.seq : null,
      message: body.error || 'This screen was changed elsewhere',
      changesSince: Array.isArray(body.changes_since) ? body.changes_since : [],
    }
  }
  if (!r.ok) throw new Error(body.error || `Request failed: ${r.status}`)
  return { ok: true, revision: String(body.revision ?? ''), seq: typeof body.seq === 'number' ? body.seq : null, batch: body.batch ?? null }
}

/** Faceplate definitions (one per equipment kind, opened from many symbols). */
export function useFaceplates(enabled = true) {
  return useQuery({
    queryKey: queryKeys.faceplates,
    queryFn: () => fetchJson<FaceplateDef[]>('/api/v1/faceplates'),
    enabled,
  })
}

export function useCreateFaceplate() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (def: FaceplateDef) => {
      await fetchOk('/api/v1/faceplates', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(def),
      })
      return def
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.faceplates }),
  })
}
