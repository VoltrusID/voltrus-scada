import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { keepPreviousData } from '@tanstack/react-query'
import type { SensorConfig } from '../types'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export function useSensors(enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.sensors,
    queryFn: () => fetchJson<SensorConfig[]>('/api/v1/data-sources'),
    enabled,
  })
}

export function useScreenSensors(screenId: string, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.screenSensors(screenId),
    queryFn: () => fetchJson<SensorConfig[]>(
      screenId === 'all' ? '/api/v1/data-sources' : `/api/v1/screens/${screenId}/data-sources`
    ),
    enabled,
    placeholderData: keepPreviousData,
  })
}

export function useUpdateSensor() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (s: SensorConfig) => {
      await fetchOk(`/api/v1/data-sources/${s.id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(s),
      })
      return s
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.sensors })
      qc.invalidateQueries({ queryKey: ['screenDataSources'] })
    },
  })
}

export function useDeleteSensor() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/data-sources/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.sensors })
      qc.invalidateQueries({ queryKey: ['screenDataSources'] })
    },
  })
}

export function useAddSensor() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (form: Partial<SensorConfig>) => {
      const res = await fetchOk('/api/v1/data-sources', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(form),
      })
      return res.json() as Promise<{ id: number }>
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.sensors })
    },
  })
}

export function useReorderSensors() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (order: number[]) => {
      await fetchOk('/api/v1/data-sources/reorder', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ order }),
      })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.sensors })
      qc.invalidateQueries({ queryKey: ['screenDataSources'] })
    },
  })
}
