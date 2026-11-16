import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { Flow, FlowCatalogue, FlowRun, FlowTestInput, FlowTestResult } from '../types'
import { queryKeys } from './keys'
import { fetchJson, fetchOk, fetchSigned, fetchWithRefresh } from './fetch'
import { FALLBACK_CATALOGUE } from '../components/flow-editor/catalogue'

/** Error from a flow endpoint; `nodeId` is set when the server names a node. */
export class FlowApiError extends Error {
  status: number
  nodeId?: string
  constructor(message: string, status: number, nodeId?: string) {
    super(message)
    this.status = status
    this.nodeId = nodeId
  }
}

/** `signed`: the request may need an electronic signature (a manual run). */
async function flowRequest(url: string, init?: RequestInit, signed = false): Promise<Response> {
  const r = await (signed ? fetchSigned : fetchWithRefresh)(url, init)
  if (!r.ok) {
    const body = await r.json().catch(() => ({})) as { error?: string; node_id?: string }
    throw new FlowApiError(body.error || `Request failed: ${r.status}`, r.status, body.node_id)
  }
  return r
}

const jsonInit = (method: string, body?: unknown): RequestInit => ({
  method,
  headers: { 'Content-Type': 'application/json' },
  body: body === undefined ? undefined : JSON.stringify(body),
})

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

export function useFlowRuns(id: string, enabled: boolean, opts: { refetchInterval?: number } = {}) {
  return useQuery({
    queryKey: queryKeys.flowRuns(id),
    queryFn: () => fetchJson<{ runs: FlowRun[] }>(`/api/v1/flows/${id}/runs`).then(r => r.runs),
    enabled: enabled && !!id,
    retry: false,
    ...opts,
  })
}

/**
 * Node catalogue. Servers before the graph engine have no such endpoint: a
 * 404 falls back to the built-in list of the old node types.
 */
export function useFlowNodeTypes(enabled = true) {
  return useQuery({
    queryKey: ['flowNodeTypes'] as const,
    queryFn: async (): Promise<FlowCatalogue & { fallback?: boolean }> => {
      const r = await fetchWithRefresh('/api/v1/flows/node-types')
      if (r.status === 404) return { ...FALLBACK_CATALOGUE, fallback: true }
      if (!r.ok) throw new Error(`node types: ${r.status}`)
      const c = await r.json() as FlowCatalogue
      return { triggers: c.triggers ?? [], nodes: c.nodes ?? [] }
    },
    enabled,
    staleTime: 5 * 60_000,
  })
}

export function useCreateFlow() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (flow: Flow) => {
      const r = await flowRequest('/api/v1/flows', jsonInit('POST', flow))
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
      await flowRequest(`/api/v1/flows/${flow.id}`, jsonInit('PUT', flow))
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

/** Dry run of an unsaved graph: side-effect nodes are skipped. */
export function useTestFlow() {
  return useMutation({
    mutationFn: async (req: { flow: { trigger_config: string; nodes: string }; input: FlowTestInput }) => {
      const r = await flowRequest('/api/v1/flows/test', jsonInit('POST', req))
      return r.json() as Promise<FlowTestResult>
    },
  })
}

/** Manual run of the saved flow (real side effects). */
export function useRunFlow() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, input }: { id: string; input?: FlowTestInput }) => {
      const r = await flowRequest(`/api/v1/flows/${id}/run`, jsonInit('POST', input ? { input } : {}), true)
      return r.json().catch(() => ({})) as Promise<unknown>
    },
    onSuccess: (_, { id }) => {
      qc.invalidateQueries({ queryKey: queryKeys.flowRuns(id) })
    },
  })
}

/** Generate (or regenerate) the webhook token; the server shows it only once. */
export function useWebhookToken() {
  return useMutation({
    mutationFn: async (id: string) => {
      const r = await flowRequest(`/api/v1/flows/${id}/webhook-token`, { method: 'POST' })
      return r.json() as Promise<{ token: string }>
    },
  })
}
