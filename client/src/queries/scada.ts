import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ScadaEquipmentItem, ScadaConnectionItem } from '../types'
import type { FaceplateDef } from '../scada2/hmi/faceplateMap'
import { queryKeys } from './keys'
import { fetchJson, fetchOk, fetchSigned } from './fetch'

export function useScadaEquipment(screenId: string, enabled: boolean) {
  const actualId = screenId === 'all' ? '__overview' : screenId
  return useQuery({
    queryKey: queryKeys.scadaEquipment(screenId),
    queryFn: () => fetchJson<{ items: ScadaEquipmentItem[] }>(`/api/v1/screens/${actualId}/scada-equipment`).then(r => r.items),
    enabled,
  })
}

export function useScadaConnections(screenId: string, enabled: boolean) {
  const actualId = screenId === 'all' ? '__overview' : screenId
  return useQuery({
    queryKey: queryKeys.scadaConnections(screenId),
    queryFn: () => fetchJson<{ items: ScadaConnectionItem[] }>(`/api/v1/screens/${actualId}/scada-connections`).then(r => r.items),
    enabled,
  })
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

/** A screen's 2D layout with its revision (what the Designer edits). */
export interface ScadaLayout { equipment: ScadaEquipmentItem[]; connections: ScadaConnectionItem[]; revision: string }

const layoutId = (screenId: string) => (screenId === 'all' ? '__overview' : screenId)

/**
 * The layout and its revision, read together. Refetched when the window gets
 * focus again, so the Designer learns about a save made elsewhere.
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

export type LayoutSaveResult = { ok: true; revision: string } | { ok: false; conflict: true; revision: string; message: string }

/**
 * Save equipment and connections in one request. With `baseRevision` the
 * server refuses (409) when the screen changed since that revision; the
 * result then carries the current revision instead of throwing.
 */
export async function saveScadaLayout(
  screenId: string, equipment: ScadaEquipmentItem[], connections: ScadaConnectionItem[], baseRevision: string | null,
): Promise<LayoutSaveResult> {
  const r = await fetchSigned(`/api/v1/screens/${layoutId(screenId)}/scada-layout`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ equipment, connections, base_revision: baseRevision }),
  })
  const body = await r.json().catch(() => ({}))
  if (r.status === 409 && typeof body.revision === 'string') {
    return { ok: false, conflict: true, revision: body.revision, message: body.error || 'This screen was changed elsewhere' }
  }
  if (!r.ok) throw new Error(body.error || `Request failed: ${r.status}`)
  return { ok: true, revision: String(body.revision ?? '') }
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
