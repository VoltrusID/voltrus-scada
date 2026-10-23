import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export function useShortcuts(enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.shortcuts,
    queryFn: () => fetchJson<Record<number, string>>('/api/v1/shortcuts'),
    enabled,
  })
}

export function useNavShortcuts(enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.navShortcuts,
    queryFn: () => fetchJson<Record<string, string>>('/api/v1/nav-shortcuts'),
    enabled,
  })
}

export function useSaveShortcuts() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (shortcuts: Record<number, string | null>) => {
      await fetchOk('/api/v1/shortcuts', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(shortcuts),
      })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.shortcuts })
    },
  })
}

export function useSaveNavShortcuts() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (shortcuts: Record<string, string | null>) => {
      await fetchOk('/api/v1/nav-shortcuts', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(shortcuts),
      })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.navShortcuts })
    },
  })
}
