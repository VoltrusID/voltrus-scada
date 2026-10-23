import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { keepPreviousData } from '@tanstack/react-query'
import type { ScreenLayouts } from '../types'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export function useLayouts(screenId: string, enabled: boolean) {
  const actualId = screenId === 'all' ? '__overview' : screenId
  return useQuery({
    queryKey: queryKeys.layouts(screenId),
    queryFn: async () => {
      const data = await fetchJson<any>(`/api/v1/screens/${actualId}/layouts`)
      return { lg: data.lg || [], md: data.md || [], sm: data.sm || [] } as ScreenLayouts
    },
    enabled,
    placeholderData: keepPreviousData,
  })
}

export function useSaveLayout() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ screenId, layouts }: { screenId: string; layouts: ScreenLayouts }) => {
      const actualId = screenId === 'all' ? '__overview' : screenId
      await fetchOk(`/api/v1/screens/${actualId}/layouts`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(layouts),
      })
    },
    onSuccess: (_, { screenId }) => {
      qc.invalidateQueries({ queryKey: queryKeys.layouts(screenId) })
    },
  })
}
