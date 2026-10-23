import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { CanvasLayoutItem } from '../types'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export function useCanvasLayouts(screenId: string, enabled: boolean) {
  const actualId = screenId === 'all' ? '__overview' : screenId
  return useQuery({
    queryKey: queryKeys.canvasLayouts(screenId),
    queryFn: () => fetchJson<CanvasLayoutItem[]>(`/api/v1/screens/${actualId}/canvas-layouts`),
    enabled,
  })
}

export function useSaveCanvasLayout() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ screenId, layouts }: { screenId: string; layouts: CanvasLayoutItem[] }) => {
      const actualId = screenId === 'all' ? '__overview' : screenId
      await fetchOk(`/api/v1/screens/${actualId}/canvas-layouts`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(layouts),
      })
    },
    onSuccess: (_, { screenId }) => {
      qc.invalidateQueries({ queryKey: queryKeys.canvasLayouts(screenId) })
    },
  })
}
