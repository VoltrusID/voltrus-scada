import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { fetchJson, fetchOk } from './fetch'
import { queryKeys } from './keys'

// ── Types ──

export interface FeedTemplateSummary {
  id: string; name: string; industry: string; description: string
  estimated_tag_count: number; equipment_units: string[]
}
export interface FeedTemplatesResponse { templates: FeedTemplateSummary[] }

export interface FeedDetail {
  id: string; name: string; industry: string; description: string;
  estimated_tag_count: number; equipment_units: { name: string; quantity: string; description: string }[]
  io_list: { signal: string; description: string; io_type: string; unit: string; data_type: string; equipment: string; criticality: string; notes: string }[]
  alarm_strategy: { equipment: string; rules: { signal_pattern: string; severity: string; condition: string; setpoint: string; deadband: string; description: string; alarm_class: string }[] }[]
  dashboard: { description: string; recommended_screens: { name: string; layout: string; sections: { title: string; equipment_group: string; widget_type: string; signals: string[] }[] }[] }
  network_notes?: string
}

export interface WizardResult {
  success: boolean; device_id: string
  tags_created: number; alarms_created: number; flows_created: number; screens_created: number; tags_discovered: number
  warnings: string[]; next_steps: string[]
}

// ── Hooks ──

export function useFeedTemplates() {
  return useQuery({
    queryKey: queryKeys.wizardFeedTemplates,
    queryFn: () => fetchJson<FeedTemplatesResponse>('/api/v1/wizard/feed-templates'),
  })
}

export function useFeedTemplate(id: string | null) {
  return useQuery({
    queryKey: queryKeys.wizardFeedTemplate(id!),
    queryFn: () => fetchJson<FeedDetail>(`/api/v1/wizard/feed-templates/${id}`),
    enabled: !!id,
  })
}

export function useWizardCreate() {
  return useMutation({
    mutationFn: (body: any) =>
      fetchOk('/api/v1/wizard/create', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      }).then(r => r.json() as Promise<WizardResult>),
  })
}
