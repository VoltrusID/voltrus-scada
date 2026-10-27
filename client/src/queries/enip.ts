import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { createdId, enipDeviceBody, enipTagBody } from '../protocolPayloads'
import type { EnipDeviceForm, EnipTagForm } from '../protocolPayloads'
import { fetchJson, fetchOk } from './fetch'

/** A row of `GET /api/v1/enip/devices`; `url` is `host:port`. */
export interface EnipDevice {
  id: number
  name: string
  url: string
  slot: number
  interval_ms: number
  assembly_instance: number
  enabled: boolean
  created_at: number
  tag_count: number
}

/** A row of `GET /api/v1/enip/devices/{id}/tags`; `tag_name` is the CIP symbol. */
export interface EnipTag {
  id: number
  device_id: number
  tag_name: string
  name: string
  display_name: string
  data_type: string
  assembly_offset: number
  assembly_length: number
  created_at: number
}

export function useEnipDevices(enabled = true) {
  return useQuery({
    queryKey: queryKeys.enipDevices,
    queryFn: () => fetchJson<EnipDevice[]>('/api/v1/enip/devices'),
    enabled,
  })
}

export function useEnipDevice(id: number, enabled = true) {
  return useQuery({
    queryKey: queryKeys.enipDevice(id),
    queryFn: () => fetchJson<EnipTag[]>(`/api/v1/enip/devices/${id}/tags`),
    enabled: enabled && !!id,
  })
}

/** Creates the device, then its tags (the create route takes no tags). */
export function useCreateEnipDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ device, tags = [] }: { device: EnipDeviceForm; tags?: EnipTagForm[] }) => {
      const res = await fetchOk('/api/v1/enip/devices', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(enipDeviceBody(device)),
      })
      const id = createdId(await res.json())
      for (const tag of tags) {
        await fetchOk(`/api/v1/enip/devices/${id}/tags`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(enipTagBody(tag)),
        })
      }
      return id
    },
    onSettled: () => {
      qc.invalidateQueries({ queryKey: queryKeys.enipDevices })
    },
  })
}

export function useUpdateEnipDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, ...fields }: { id: number } & Record<string, any>) => {
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
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/enip/devices/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.enipDevices })
    },
  })
}

export function useBrowseEnipDevice() {
  return useMutation({
    mutationFn: async ({ id, params }: { id: number; params?: Record<string, any> }) => {
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
    mutationFn: async (id: number) => {
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
    mutationFn: async (id: number) => {
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
    mutationFn: async ({ deviceId, tag }: { deviceId: number; tag: EnipTagForm }) => {
      const res = await fetchOk(`/api/v1/enip/devices/${deviceId}/tags`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(enipTagBody(tag)),
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
    mutationFn: async ({ deviceId, tagId }: { deviceId: number; tagId: number }) => {
      await fetchOk(`/api/v1/enip/devices/${deviceId}/tags/${tagId}`, { method: 'DELETE' })
    },
    onSuccess: (_, { deviceId }) => {
      qc.invalidateQueries({ queryKey: queryKeys.enipDevice(deviceId) })
    },
  })
}
