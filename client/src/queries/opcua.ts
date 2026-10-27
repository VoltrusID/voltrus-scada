import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { fetchJson, fetchOk } from './fetch'
import { opcuaBrowseNodes, opcuaDeviceBody } from '../protocolPayloads'
import type { OpcUaDeviceForm, OpcUaSecurityMode, OpcUaSecurityPolicy } from '../protocolPayloads'

/** A row of `GET /api/v1/opcua/devices` (the password is never sent back). */
export interface OpcUaDevice {
  id: number
  name: string
  url: string
  security_policy: OpcUaSecurityPolicy
  security_mode: OpcUaSecurityMode
  username: string | null
  interval_ms: number
  enabled: boolean
  created_at: number
  node_count: number
}

export interface OpcUaNode {
  id: number
  device_id: number
  node_id: string
  name: string
  display_name: string
  data_type: string
  created_at: number
}

export interface BrowseResult {
  node_id: string
  browse_name: string
  display_name: string
  node_class: string
}

export function useOpcUaDevices() {
  return useQuery({
    queryKey: ['opcua-devices'],
    queryFn: () => fetchJson<OpcUaDevice[]>('/api/v1/opcua/devices'),
  })
}

export function useOpcUaNodes(deviceId: number | null) {
  return useQuery({
    queryKey: ['opcua-nodes', deviceId],
    queryFn: () => fetchJson<OpcUaNode[]>(`/api/v1/opcua/devices/${deviceId}/nodes`),
    enabled: deviceId !== null,
  })
}

export function useCreateOpcUaDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (device: OpcUaDeviceForm) => {
      const r = await fetchOk('/api/v1/opcua/devices', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(opcuaDeviceBody(device)),
      })
      return r.json()
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ['opcua-devices'] }),
  })
}

export function useUpdateOpcUaDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, ...data }: { id: number } & Partial<Pick<OpcUaDevice, 'name' | 'url' | 'security_policy' | 'security_mode' | 'username' | 'interval_ms' | 'enabled'>>) => {
      const r = await fetchOk(`/api/v1/opcua/devices/${id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
      })
      return r.json()
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ['opcua-devices'] }),
  })
}

export function useDeleteOpcUaDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/opcua/devices/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ['opcua-devices'] }),
  })
}

export function useBrowseOpcUaDevice() {
  return useMutation({
    mutationFn: async (id: number) => {
      const r = await fetchOk(`/api/v1/opcua/devices/${id}/browse`, { method: 'POST' })
      return opcuaBrowseNodes<BrowseResult>(await r.json())
    },
  })
}

export function useAddOpcUaNode() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ deviceId, node }: { deviceId: number; node: { node_id: string; name: string; display_name?: string; data_type?: string } }) => {
      const r = await fetchOk(`/api/v1/opcua/devices/${deviceId}/nodes`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(node),
      })
      return r.json()
    },
    onSuccess: (_d, vars) => qc.invalidateQueries({ queryKey: ['opcua-nodes', vars.deviceId] }),
  })
}

export function useDeleteOpcUaNode() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ deviceId, nodeId }: { deviceId: number; nodeId: number }) => {
      await fetchOk(`/api/v1/opcua/devices/${deviceId}/nodes/${nodeId}`, { method: 'DELETE' })
    },
    onSuccess: (_d, vars) => qc.invalidateQueries({ queryKey: ['opcua-nodes', vars.deviceId] }),
  })
}

export function useStartOpcUaDevice() {
  return useMutation({
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/opcua/devices/${id}/start`, { method: 'POST' })
    },
  })
}

export function useStopOpcUaDevice() {
  return useMutation({
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/opcua/devices/${id}/stop`, { method: 'POST' })
    },
  })
}
