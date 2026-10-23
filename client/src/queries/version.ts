import { useQuery } from '@tanstack/react-query'
import { fetchJson } from './fetch'
import { queryKeys } from './keys'

export function useVersion() {
  return useQuery({
    queryKey: queryKeys.version,
    queryFn: () => fetchJson<{ version: string }>('/api/v1/version'),
    staleTime: Infinity,
  })
}
