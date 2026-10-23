import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { Screen } from '../types'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export function useScreens(enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.screens,
    queryFn: () => fetchJson<Screen[]>('/api/v1/screens'),
    enabled,
  })
}

export function useCreateScreen() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (name: string) => {
      const r = await fetchOk('/api/v1/screens', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name }),
      })
      return r.json() as Promise<{ id: string; name: string }>
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.screens })
    },
  })
}

export function useRenameScreen() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, name }: { id: string; name: string }) => {
      await fetchOk(`/api/v1/screens/${id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name }),
      })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.screens })
    },
  })
}

export function useDeleteScreen() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: string) => {
      await fetchOk(`/api/v1/screens/${id}`, { method: 'DELETE' })
      return id
    },
    onSuccess: (deletedId) => {
      qc.invalidateQueries({ queryKey: queryKeys.screens })
      qc.invalidateQueries({ queryKey: queryKeys.layouts(deletedId) })
      qc.invalidateQueries({ queryKey: ['screenDataSources'] })
    },
  })
}

export function useAddSensorToScreen() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ screenId, sensorId }: { screenId: string; sensorId: number }) => {
      await fetchOk(`/api/v1/screens/${screenId}/data-sources`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ data_source_id: sensorId }),
      })
    },
    onSuccess: (_, { screenId }) => {
      qc.invalidateQueries({ queryKey: queryKeys.screenSensors(screenId) })
      qc.invalidateQueries({ queryKey: queryKeys.layouts(screenId) })
    },
  })
}

export function useRemoveSensorFromScreen() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ screenId, sensorId }: { screenId: string; sensorId: number }) => {
      await fetchOk(`/api/v1/screens/${screenId}/data-sources/${sensorId}`, { method: 'DELETE' })
    },
    onSuccess: (_, { screenId }) => {
      qc.invalidateQueries({ queryKey: queryKeys.screenSensors(screenId) })
      qc.invalidateQueries({ queryKey: queryKeys.layouts(screenId) })
    },
  })
}
