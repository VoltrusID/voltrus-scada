import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export function useTwinLayouts(screenId: string, enabled: boolean) {
  const actualId = screenId === 'all' ? '__overview' : screenId
  return useQuery({
    queryKey: queryKeys.twinLayouts(screenId),
    queryFn: () => fetchJson<{ items: any[]; camera: { x: number; y: number; z: number } }>(`/api/v1/screens/${actualId}/twin-layouts`),
    enabled,
  })
}

export function useTwinConnections(screenId: string, enabled: boolean) {
  const actualId = screenId === 'all' ? '__overview' : screenId
  return useQuery({
    queryKey: queryKeys.twinConnections(screenId),
    queryFn: () => fetchJson<{ from_id: string; to_id: string }[]>(`/api/v1/screens/${actualId}/twin-connections`),
    enabled,
  })
}

export function useSaveTwinLayout() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ screenId, items }: { screenId: string; items: any[] }) => {
      const actualId = screenId === 'all' ? '__overview' : screenId
      await fetchOk(`/api/v1/screens/${actualId}/twin-layouts`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(items),
      })
    },
    onSuccess: (_, { screenId }) => {
      qc.invalidateQueries({ queryKey: queryKeys.twinLayouts(screenId) })
    },
  })
}

export function useSaveTwinConnections() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ screenId, connections }: { screenId: string; connections: { from_id: string; to_id: string }[] }) => {
      const actualId = screenId === 'all' ? '__overview' : screenId
      await fetchOk(`/api/v1/screens/${actualId}/twin-connections`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(connections),
      })
    },
    onSuccess: (_, { screenId }) => {
      qc.invalidateQueries({ queryKey: queryKeys.twinConnections(screenId) })
    },
  })
}
