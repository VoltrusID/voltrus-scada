import { useQuery } from '@tanstack/react-query'
import { expandCompactData } from '../utils'
import { queryKeys } from './keys'
import { fetchJson } from './fetch'
import { historyUrl } from '../historyQuery'

export function useSystemInfo(enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.systemInfo,
    queryFn: () => fetchJson<any>('/api/v1/system'),
    enabled,
    refetchInterval: 5_000,
    staleTime: 4_000,
  })
}

/** The running server's version; it only changes when the server is replaced. */
export function useServerVersion() {
  return useQuery({
    queryKey: queryKeys.serverVersion,
    queryFn: async () => (await fetchJson<{ version: string }>('/api/v1/version')).version,
    staleTime: Infinity,
  })
}

/** Every tag's history, keyed by data source id (tag keys are not unique). */
export function useHistory(range: string, screenId: string, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.history(range, screenId),
    queryFn: async () => {
      const result = await fetchJson<{ data: Record<string, any>; sensors: any[] }>(historyUrl({ range, screen: screenId, keyedBy: 'id' }))
      if (!result.data) return {}
      return expandCompactData(result.data)
    },
    enabled,
    staleTime: 10_000,
  })
}
