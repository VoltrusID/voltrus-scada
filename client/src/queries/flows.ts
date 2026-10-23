import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { Flow, FlowRun, FlowTestResult } from '../types'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export function useFlows(enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.flows,
    queryFn: () => fetchJson<{ flows: Flow[] }>('/api/v1/flows').then(r => r.flows),
    enabled,
  })
}

export function useFlow(id: string, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.flow(id),
    queryFn: () => fetchJson<Flow>(`/api/v1/flows/${id}`),
    enabled: enabled && !!id,
  })
}

export function useFlowRuns(id: string, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.flowRuns(id),
    queryFn: () => fetchJson<{ runs: FlowRun[] }>(`/api/v1/flows/${id}/runs`).then(r => r.runs),
    enabled: enabled && !!id,
  })
}

export function useCreateFlow() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (flow: Flow) => {
      const r = await fetchOk('/api/v1/flows', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(flow),
      })
      return r.json() as Promise<{ id: string }>
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.flows })
    },
  })
}

export function useUpdateFlow() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (flow: Flow) => {
      await fetchOk(`/api/v1/flows/${flow.id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(flow),
      })
    },
    onSuccess: (_, flow) => {
      qc.invalidateQueries({ queryKey: queryKeys.flows })
      qc.invalidateQueries({ queryKey: queryKeys.flow(flow.id) })
    },
  })
}

export function useDeleteFlow() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: string) => {
      await fetchOk(`/api/v1/flows/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.flows })
    },
  })
}

export function useStartFlow() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: string) => {
      await fetchOk(`/api/v1/flows/${id}/start`, { method: 'POST' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.flows })
    },
  })
}

export function useStopFlow() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: string) => {
      await fetchOk(`/api/v1/flows/${id}/stop`, { method: 'POST' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.flows })
    },
  })
}

export function useTestFlow() {
  return useMutation({
    mutationFn: async ({ id, payload }: { id: string; payload: Record<string, any> }) => {
      const r = await fetchOk(`/api/v1/flows/${id}/test`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ payload }),
      })
      return r.json() as Promise<FlowTestResult>
    },
  })
}
