import { useQuery } from '@tanstack/react-query'
import { fetchJson } from './fetch'
import { queryKeys } from './keys'

// ── Types ──

export interface TopologyDevice {
  id: string; name: string; protocol: string; address: string; enabled: boolean; tag_count: number; online: boolean
}
export interface ProtocolCount { protocol: string; device_count: number; tag_count: number }
export interface TopologySummary { total_devices: number; total_tags: number; protocols: ProtocolCount[]; online_count: number; offline_count: number }
export interface TopologyData { devices: TopologyDevice[]; summary: TopologySummary }

export interface TagXrefEntry {
  key: string; name: string; device_id: string; device_name: string; protocol: string;
  unit: string; has_alarm: boolean; alarm_severity?: string; warn_val: number; crit_val: number
}
export interface TagXrefData { tags: TagXrefEntry[]; total_tags: number }

export interface ScriptDoc { id: number; name: string; enabled: boolean; description: string; cpu_limit_ms: number; syntax_error?: string }
export interface FlowDoc { id: string; name: string; enabled: boolean; trigger_description: string; node_descriptions: string[]; description: string }
export interface AutomationSummary { total_scripts: number; enabled_scripts: number; total_flows: number; enabled_flows: number }
export interface AutomationData { scripts: ScriptDoc[]; flows: FlowDoc[]; summary: AutomationSummary }

// ── Hooks ──

export function useDocsTopology() {
  return useQuery({
    queryKey: queryKeys.docsTopology,
    queryFn: () => fetchJson<TopologyData>('/api/v1/docs/topology'),
  })
}

export function useDocsTagXref() {
  return useQuery({
    queryKey: queryKeys.docsTagXref,
    queryFn: () => fetchJson<TagXrefData>('/api/v1/docs/tag-xref'),
  })
}

export function useDocsAutomation() {
  return useQuery({
    queryKey: queryKeys.docsAutomation,
    queryFn: () => fetchJson<AutomationData>('/api/v1/docs/automation'),
  })
}
