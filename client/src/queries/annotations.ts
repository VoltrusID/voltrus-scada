import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export interface Annotation {
  id: number
  tag_id: string
  annotation_text: string
  data_timestamp: number
  author_id: number
  created_at: number
  updated_at: number
}

export function useAnnotations(params: { tag_id?: string; from?: number; to?: number } = {}) {
  const sp = new URLSearchParams()
  if (params.tag_id) sp.set('tag_id', params.tag_id)
  if (params.from) sp.set('from', String(params.from))
  if (params.to) sp.set('to', String(params.to))
  const qs = sp.toString()
  return useQuery({
    queryKey: queryKeys.annotationsFiltered(params.tag_id, params.from, params.to),
    queryFn: () => fetchJson<{ annotations: Annotation[] }>(`/api/v1/annotations${qs ? '?' + qs : ''}`),
    staleTime: 5_000,
  })
}

export function useCreateAnnotation() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (data: { tag_id: string; annotation_text: string; data_timestamp: number }) => {
      const res = await fetchOk('/api/v1/annotations', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
      })
      return res.json()
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.annotations })
    },
  })
}

export function useUpdateAnnotation() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (data: { id: number; annotation_text: string; data_timestamp: number }) => {
      const res = await fetchOk(`/api/v1/annotations/${data.id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
      })
      return res.json()
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.annotations })
    },
  })
}

export function useDeleteAnnotation() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/annotations/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.annotations })
    },
  })
}
