import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export interface EnipDevice {
  id: string
  name: string
  ip: string
  port: number
  slot: number
  assembly_instance: number
  enabled: boolean
  tag_count?: number
  created_at: number
  updated_at: number
}

export interface EnipTag {
  id: number
  name: string
  data_type: string
  assembly_offset: number
  assembly_length: number
}

export function useEnipDevices(enabled = true) {
  return useQuery({
    queryKey: queryKeys.enipDevices,
    queryFn: () => fetchJson<EnipDevice[]>('/api/v1/enip/devices'),
    enabled,
  })
}

export function useEnipDevice(id: string, enabled = true) {
  return useQuery({
    queryKey: queryKeys.enipDevice(id),
    queryFn: () => fetchJson<EnipTag[]>(`/api/v1/enip/devices/${id}/tags`),
    enabled: enabled && !!id,
  })
}

export function useCreateEnipDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (device: Partial<EnipDevice> & { tags?: Partial<EnipTag>[] }) => {
      const res = await fetchOk('/api/v1/enip/devices', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(device),
      })
      return res.json()
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.enipDevices })
    },
  })
}

export function useUpdateEnipDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, ...fields }: { id: string } & Record<string, any>) => {
      await fetchOk(`/api/v1/enip/devices/${id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(fields),
      })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.enipDevices })
    },
  })
}

export function useDeleteEnipDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: string) => {
      await fetchOk(`/api/v1/enip/devices/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.enipDevices })
    },
  })
}

export function useBrowseEnipDevice() {
  return useMutation({
    mutationFn: async ({ id, params }: { id: string; params?: Record<string, any> }) => {
      const res = await fetchOk(`/api/v1/enip/devices/${id}/browse`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(params || {}),
      })
      return res.json()
    },
  })
}

export function useStartEnipDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: string) => {
      await fetchOk(`/api/v1/enip/devices/${id}/start`, { method: 'POST' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.enipDevices })
    },
  })
}

export function useStopEnipDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: string) => {
      await fetchOk(`/api/v1/enip/devices/${id}/stop`, { method: 'POST' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.enipDevices })
    },
  })
}

export function useAddEnipTag() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ deviceId, tag }: { deviceId: string; tag: Partial<EnipTag> }) => {
      const res = await fetchOk(`/api/v1/enip/devices/${deviceId}/tags`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(tag),
      })
      return res.json()
    },
    onSuccess: (_, { deviceId }) => {
      qc.invalidateQueries({ queryKey: queryKeys.enipDevice(deviceId) })
    },
  })
}

export function useRemoveEnipTag() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ deviceId, tagId }: { deviceId: string; tagId: number }) => {
      await fetchOk(`/api/v1/enip/devices/${deviceId}/tags/${tagId}`, { method: 'DELETE' })
    },
    onSuccess: (_, { deviceId }) => {
      qc.invalidateQueries({ queryKey: queryKeys.enipDevice(deviceId) })
    },
  })
}
