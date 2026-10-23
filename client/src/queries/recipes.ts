import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { fetchJson, fetchOk } from './fetch'

export interface Recipe {
  id: number
  name: string
  category: string | null
  device_id: string
  values: string
  version: number
  description: string | null
  created_at: number
  updated_at: number
}

export function useRecipes(deviceId?: string) {
  const params = new URLSearchParams()
  if (deviceId) params.set('device_id', deviceId)
  const qs = params.toString()
  return useQuery({
    queryKey: ['recipes', deviceId],
    queryFn: () => fetchJson<Recipe[]>(`/api/v1/recipes?${qs}`),
  })
}

export function useRecipe(id: number | undefined) {
  return useQuery({
    queryKey: ['recipes', id],
    queryFn: () => fetchJson<Recipe>(`/api/v1/recipes/${id}`),
    enabled: !!id,
  })
}

export function useCreateRecipe() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (data: { name: string; device_id: string; category?: string; description?: string }) => {
      const r = await fetchOk('/api/v1/recipes', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
      })
      return r.json() as Promise<Recipe>
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['recipes'] })
    },
  })
}

export function useUpdateRecipe() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, data }: { id: number; data: { name?: string; category?: string | null; description?: string | null } }) => {
      const r = await fetchOk(`/api/v1/recipes/${id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: data.name,
          category: data.category !== undefined ? data.category : undefined,
          description: data.description !== undefined ? data.description : undefined,
        }),
      })
      return r.json() as Promise<Recipe>
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['recipes'] })
    },
  })
}

export function useDeleteRecipe() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/recipes/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['recipes'] })
    },
  })
}

export function useLoadRecipe() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: number) => {
      const r = await fetchOk(`/api/v1/recipes/${id}/load`, { method: 'POST' })
      return r.json() as Promise<{ ok: boolean; command_ids: string[]; message: string }>
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['recipes'] })
    },
  })
}
