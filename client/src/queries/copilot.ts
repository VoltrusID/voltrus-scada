import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { fetchJson, fetchOk } from './fetch'
import { queryKeys } from './keys'
import type { AiSettings, AiProposal, EquipmentTemplate } from '../types'

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
    mutationFn: (body: { provider?: string; api_key?: string; default_model?: string }) =>
      fetchOk('/api/v1/ai/settings', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.aiSettings }),
  })
}

export function useTestAiSettings() {
  return useMutation({
    mutationFn: () => fetchOk('/api/v1/ai/settings/test').then(r => r.json()),
  })
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
      fetchOk(`/api/v1/ai/proposals/${id}/approve`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ applied_config }),
      }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['aiProposals'] }),
  })
}

export function useRejectProposal() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: number) =>
      fetchOk(`//api/v1/ai/proposals/${id}/reject`, { method: 'POST' }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['aiProposals'] }),
  })
}

export function useApplyProposal() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: number) =>
      fetchOk(`/api/v1/ai/proposals/${id}/apply`, { method: 'POST' }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['aiProposals'] }),
  })
}

// ── Generation ──

export function useGenerateRegisterMap() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: { file_content: string; device_name: string; file_type?: string; device_id?: string }) =>
      fetchOk('/api/v1/ai/generate/register-map', {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
      }).then(r => r.json() as Promise<AiProposal>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['aiProposals'] }),
  })
}

export function useGenerateDashboardLayout() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: { description: string; screen_id?: string }) =>
      fetchOk('/api/v1/ai/generate/dashboard-layout', {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
      }).then(r => r.json() as Promise<AiProposal>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['aiProposals'] }),
  })
}

export function useGenerateAlarmThresholds() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: { device_type: string; tags: { source_key: string; name: string; unit?: string; min_val?: number; max_val?: number }[] }) =>
      fetchOk('/api/v1/ai/generate/alarm-thresholds', {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
      }).then(r => r.json() as Promise<AiProposal>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['aiProposals'] }),
  })
}

export function useGenerateTagRename() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: { tags: { current: string }[]; convention: string; custom_pattern?: string }) =>
      fetchOk('/api/v1/ai/generate/tag-rename', {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
      }).then(r => r.json() as Promise<AiProposal>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['aiProposals'] }),
  })
}

export function useGenerateFlowRule() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: { description: string; available_tags: string[] }) =>
      fetchOk('/api/v1/ai/generate/flow-rule', {
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
      fetchOk('/api/v1/ai/generate/from-template', {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
      }).then(r => r.json() as Promise<AiProposal>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['aiProposals'] }),
  })
}
