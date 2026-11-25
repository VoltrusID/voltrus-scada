import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { fetchJson, fetchOk, fetchSigned } from './fetch'
import { copilotRequestError } from '../copilot/proposalChecks'
import { queryKeys } from './keys'
import type { AiSettings, AiProposal, EquipmentTemplate } from '../types'
import type { AiSettingsBody } from '../copilot/aiSettings'

// ── Settings ──

export function useAiSettings() {
  return useQuery({
    queryKey: queryKeys.aiSettings,
    queryFn: () => fetchJson<AiSettings>('/api/v1/ai/settings'),
  })
}

export function useUpdateAiSettings() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: AiSettingsBody) =>
      fetchOk('/api/v1/ai/settings', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.aiSettings }),
  })
}

export function useTestAiSettings() {
  return useMutation({
    mutationFn: () => fetchOk('/api/v1/ai/settings/test', { method: 'POST' }).then(r => r.json()),
  })
}

/** fetchSigned that throws a CopilotRequestError (status, code, Retry-After) when refused. */
async function copilotFetch(url: string, init?: RequestInit): Promise<Response> {
  const r = await fetchSigned(url, init)
  if (!r.ok) throw await copilotRequestError(r)
  return r
}

// ── Proposals ──

export function useAiProposals(type?: string, status?: string) {
  return useQuery({
    queryKey: queryKeys.aiProposals(type, status),
    queryFn: () => {
      const params = new URLSearchParams()
      if (type) params.set('proposal_type', type)
      if (status) params.set('status', status)
      const qs = params.toString()
      return fetchJson<AiProposal[]>(`/api/v1/ai/proposals${qs ? '?' + qs : ''}`)
    },
  })
}

export function useAiProposal(id: number | null) {
  return useQuery({
    queryKey: queryKeys.aiProposal(id!),
    queryFn: () => fetchJson<AiProposal>(`/api/v1/ai/proposals/${id}`),
    enabled: id !== null,
  })
}

export function useApproveProposal() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, applied_config }: { id: number; applied_config?: any }) =>
      copilotFetch(`/api/v1/ai/proposals/${id}/approve`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ applied_config }),
      }),
    onSettled: () => refreshProposals(qc),
  })
}

export function useRejectProposal() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: number) =>
      copilotFetch(`/api/v1/ai/proposals/${id}/reject`, { method: 'POST' }),
    onSettled: () => refreshProposals(qc),
  })
}

export function useApplyProposal() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: number) =>
      copilotFetch(`/api/v1/ai/proposals/${id}/apply`, { method: 'POST' }),
    // A failed apply changes the proposal too (back to approved, or failed).
    onSettled: () => refreshProposals(qc),
  })
}

/** Refetch the proposal list and any open proposal, whatever the action's outcome. */
function refreshProposals(qc: ReturnType<typeof useQueryClient>) {
  return Promise.all([
    qc.invalidateQueries({ queryKey: ['aiProposals'] }),
    qc.invalidateQueries({ queryKey: ['aiProposal'] }),
  ])
}

// ── Generation ──

export function useGenerateRegisterMap() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: { file_content: string; device_name: string; file_type?: string; device_id?: string }) =>
      copilotFetch('/api/v1/ai/generate/register-map', {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
      }).then(r => r.json() as Promise<AiProposal>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['aiProposals'] }),
  })
}

export function useGenerateDashboardLayout() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: { description: string; screen_id?: string }) =>
      copilotFetch('/api/v1/ai/generate/dashboard-layout', {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
      }).then(r => r.json() as Promise<AiProposal>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['aiProposals'] }),
  })
}

export function useGenerateAlarmThresholds() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: { device_type: string; tags: { id?: number; source_key: string; name: string; unit?: string; min_val?: number; max_val?: number }[] }) =>
      copilotFetch('/api/v1/ai/generate/alarm-thresholds', {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
      }).then(r => r.json() as Promise<AiProposal>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['aiProposals'] }),
  })
}

export function useGenerateTagRename() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: { tags: { current: string }[]; convention: string; custom_pattern?: string }) =>
      copilotFetch('/api/v1/ai/generate/tag-rename', {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
      }).then(r => r.json() as Promise<AiProposal>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['aiProposals'] }),
  })
}

export function useGenerateFlowRule() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: { description: string; available_tags: string[] }) =>
      copilotFetch('/api/v1/ai/generate/flow-rule', {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
      }).then(r => r.json() as Promise<AiProposal>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['aiProposals'] }),
  })
}

// ── Templates ──

export function useAiTemplates(industry?: string) {
  return useQuery({
    queryKey: queryKeys.aiTemplates(industry),
    queryFn: () => {
      const params = industry ? `?industry=${encodeURIComponent(industry)}` : ''
      return fetchJson<EquipmentTemplate[]>(`/api/v1/ai/templates${params}`)
    },
  })
}

export function useGenerateFromTemplate() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: { template_id: number; device_id?: string; prefix?: string }) =>
      copilotFetch('/api/v1/ai/generate/from-template', {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
      }).then(r => r.json() as Promise<AiProposal>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['aiProposals'] }),
  })
}
