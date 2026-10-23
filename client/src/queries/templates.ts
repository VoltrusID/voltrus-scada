import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { fetchJson, fetchOk } from './fetch'
import { queryKeys } from './keys'

// ── Types ──

export interface TemplateSummary {
  id: string; name: string; industry: string; description: string
  tag_count: number; alarm_count: number; flow_count: number; has_oee: boolean
}
export interface IndustryInfo { id: string; name: string }
export interface TemplateListResponse { templates: TemplateSummary[]; industries: IndustryInfo[] }

export interface TemplateDetail {
  id: string; name: string; industry: string; description: string; equipment_type: string
  tags: { name_pattern: string; description: string; unit: string; data_type: string; group: string; widget: string }[]
  alarms: { tag_pattern: string; severity: string; comparison: string; threshold: number; message_template: string }[]
  flows: { name: string; description: string; trigger_type: string }[]
  oee_config: any
  dashboard: { screen_name: string }
}

export interface PreviewResult {
  expanded_tags: any[]
  expanded_alarms: any[]
}

export interface InstantiateResult {
  tags_created: number; alarms_created: number; flows_created: number
  device_id: string
}

// ── Hooks ──

export function useTemplates(industry?: string) {
  return useQuery({
    queryKey: queryKeys.templates(industry),
    queryFn: () => fetchJson<TemplateListResponse>('/api/v1/templates' + (industry ? `?industry=${industry}` : '')),
  })
}

export function useTemplate(id: string | null) {
  return useQuery({
    queryKey: queryKeys.template(id!),
    queryFn: () => fetchJson<TemplateDetail>(`/api/v1/templates/${id}`),
    enabled: !!id,
  })
}

export function usePreviewTemplate() {
  return useMutation({
    mutationFn: (body: { template_id: string; instance_counts: { n: number } }) =>
      fetchOk('/api/v1/templates/preview', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      }).then(r => r.json() as Promise<PreviewResult>),
  })
}

export function useInstantiateTemplate() {
  return useMutation({
    mutationFn: (body: { template_id: string; name: string; instance_counts: { n: number } }) =>
      fetchOk(`/api/v1/templates/${body.template_id}/instantiate`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      }).then(r => r.json() as Promise<InstantiateResult>),
  })
}
