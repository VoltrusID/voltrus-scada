import { useQuery } from '@tanstack/react-query'
import { expandCompactData } from '../utils'
import { queryKeys } from './keys'
import { fetchJson } from './fetch'

export function useSystemInfo(enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.systemInfo,
    queryFn: () => fetchJson<any>('/api/v1/system'),
    enabled,
    refetchInterval: 5_000,
    staleTime: 4_000,
  })
}

export function useHistory(range: string, screenId: string, enabled: boolean) {
  const dp = screenId === 'all' ? '' : `&screen=${screenId}`
  return useQuery({
    queryKey: queryKeys.history(range, screenId),
    queryFn: async () => {
      const result = await fetchJson<{ data: Record<string, any>; sensors: any[] }>(`/api/v1/history?range=${range}${dp}`)
      if (!result.data) return {}
      return expandCompactData(result.data)
    },
    enabled,
    staleTime: 10_000,
  })
}
