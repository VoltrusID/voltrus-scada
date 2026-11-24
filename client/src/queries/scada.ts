import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ScadaEquipmentItem, ScadaConnectionItem } from '../types'
import type { FaceplateDef } from '../scada2/hmi/faceplateMap'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

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
