import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ModbusDevice, ModbusRegister } from '../types'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export function useModbusDevices(enabled = true) {
  return useQuery({
    queryKey: queryKeys.modbusDevices,
    queryFn: () => fetchJson<ModbusDevice[]>('/api/v1/modbus-devices'),
    enabled,
  })
}

export function useModbusDevice(id: string, enabled = true) {
  return useQuery({
    queryKey: queryKeys.modbusDevice(id),
    queryFn: async () => {
      const r = await fetchJson<{ device: ModbusDevice; registers: ModbusRegister[] }>(`/api/v1/modbus-devices/${id}`)
      return r
    },
    enabled: enabled && !!id,
  })
}

export function useModbusRegisters(deviceId: string, enabled = true) {
  return useQuery({
    queryKey: queryKeys.modbusRegisters(deviceId),
    queryFn: async () => {
      const r = await fetchJson<{ device: ModbusDevice; registers: ModbusRegister[] }>(`/api/v1/modbus-devices/${deviceId}`)
      return r.registers
    },
    enabled: enabled && !!deviceId,
  })
}

export function useModbusPollerStatus(enabled = true) {
  return useQuery({
    queryKey: queryKeys.modbusStatus,
    queryFn: () => fetchJson<{ device_id: string; running: boolean; status: string }[]>('/api/v1/modbus-devices/status'),
    enabled,
    refetchInterval: 5_000,
    staleTime: 4_000,
  })
}

export function useCreateModbusDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (device: Partial<ModbusDevice> & { registers?: Partial<ModbusRegister>[] }) => {
      const res = await fetchOk('/api/v1/modbus-devices', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(device),
      })
      return res.json() as Promise<{ ok: boolean; id: string }>
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.modbusDevices })
      qc.invalidateQueries({ queryKey: queryKeys.modbusStatus })
    },
  })
}

export function useUpdateModbusDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, ...fields }: { id: string } & Record<string, any>) => {
      await fetchOk(`/api/v1/modbus-devices/${id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(fields),
      })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.modbusDevices })
      qc.invalidateQueries({ queryKey: queryKeys.modbusStatus })
    },
  })
}

export function useDeleteModbusDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: string) => {
      await fetchOk(`/api/v1/modbus-devices/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.modbusDevices })
      qc.invalidateQueries({ queryKey: queryKeys.modbusStatus })
    },
  })
}

export function useReplaceModbusRegisters() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ deviceId, registers }: { deviceId: string; registers: Partial<ModbusRegister>[] }) => {
      await fetchOk(`/api/v1/modbus-devices/${deviceId}/registers`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ registers }),
      })
    },
    onSuccess: (_, { deviceId }) => {
      qc.invalidateQueries({ queryKey: queryKeys.modbusRegisters(deviceId) })
      qc.invalidateQueries({ queryKey: queryKeys.modbusDevice(deviceId) })
    },
  })
}

export function useRestartModbusPoller() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: string) => {
      await fetchOk(`/api/v1/modbus-devices/${id}/restart`, { method: 'POST' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.modbusStatus })
    },
  })
}

export interface ScannedRegister {
  address: number
  raw_value: number | null
  raw_value2: number | null
  changed: boolean
  readable: boolean
  likely_type: string
  interpreted_value: number | null
}

export function useScanModbusRegisters() {
  return useMutation({
    mutationFn: async (params: {
      ip: string; port: number; unit_id: number
      start_addr: number; end_addr: number
      endianness: string; register_area: string
      scan_delay_ms: number
    }) => {
      const res = await fetchOk('/api/v1/modbus-devices/scan', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(params),
      })
      return res.json() as Promise<ScannedRegister[]>
    },
  })
}
