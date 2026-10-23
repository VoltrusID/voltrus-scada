import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export interface TagTreeNode {
  name: string
  tagCount: number
  tagIds: number[]
  children: TagTreeNode[]
}

export interface NamingConvention {
  template: string
  regex: string
  custom_regex: string | null
  templates: { id: string; label: string }[]
}

export function useTagTree(search?: string, enabled = true) {
  const sp = search ? `?search=${encodeURIComponent(search)}` : ''
  return useQuery({
    queryKey: queryKeys.tagTree(search),
    queryFn: () => fetchJson<{ tree: TagTreeNode[] }>(`/api/v1/tags/tree${sp}`),
    enabled,
  })
}

export function useUpdateTagPath() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, path }: { id: number; path: string | null }) => {
      await fetchOk(`/api/v1/tags/${id}/path`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path }),
      })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['tagTree'] })
    },
  })
}

export function useNamingConvention(enabled = true) {
  return useQuery({
    queryKey: queryKeys.namingConvention,
    queryFn: () => fetchJson<NamingConvention>('/api/v1/settings/naming-convention'),
    enabled,
  })
}

export function useUpdateNamingConvention() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (body: { template: string; custom_regex?: string }) => {
      await fetchOk('/api/v1/settings/naming-convention', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.namingConvention })
    },
  })
}
