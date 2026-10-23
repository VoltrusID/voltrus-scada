import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export interface MqttBrokerStatus {
  status: string
  connected_clients: number
  clients: MqttClient[]
  config: {
    port: number
    max_connections: number
    auth_enabled: boolean
  } | null
}

export interface MqttClient {
  client_id: string
  connected_at: number
  subscriptions: string[]
}

export interface MqttConfigUpdate {
  port?: number
  max_connections?: number
  auth?: {
    enabled?: boolean
    users?: { username: string; password: string }[]
  }
}

export interface TopicInfo {
  topic: string
  last_payload: string
  last_update: number
  message_count: number
  retain: boolean
}

export interface TopicTree {
  [key: string]: TopicTree | { _leaf?: boolean; _message_count?: number; _last_update?: number }
}

export interface MqttTopicsResponse {
  topics: TopicInfo[]
  tree: TopicTree
}

export interface RecentMessage {
  topic: string
  payload: string
  timestamp: number
  retain: boolean
}

export interface MqttMessagesResponse {
  messages: RecentMessage[]
}

export interface MqttTopicMessagesResponse {
  topic: string
  messages: RecentMessage[]
}

export function useMqttBrokerStatus(enabled = true) {
  return useQuery({
    queryKey: queryKeys.mqttBroker,
    queryFn: () => fetchJson<MqttBrokerStatus>('/api/v1/mqtt-broker/status'),
    enabled,
    refetchInterval: 5000,
  })
}

export function useUpdateMqttConfig() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (config: MqttConfigUpdate) => {
      const r = await fetchOk('/api/v1/mqtt-broker/config', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(config),
      })
      return r.json()
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.mqttBroker })
    },
  })
}

export function useStartMqttBroker() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async () => {
      const r = await fetchOk('/api/v1/mqtt-broker/start', { method: 'POST' })
      return r.json()
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.mqttBroker })
    },
  })
}

export function useStopMqttBroker() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async () => {
      const r = await fetchOk('/api/v1/mqtt-broker/stop', { method: 'POST' })
      return r.json()
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.mqttBroker })
    },
  })
}

export function useMqttTopics(enabled = true) {
  return useQuery({
    queryKey: queryKeys.mqttTopics,
    queryFn: () => fetchJson<MqttTopicsResponse>('/api/v1/mqtt-broker/topics'),
    enabled,
    refetchInterval: 3000,
  })
}

export function useMqttTopicMessages(topic: string | null, enabled = true) {
  return useQuery({
    queryKey: queryKeys.mqttTopicMessages(topic ?? ''),
    queryFn: () =>
      fetchJson<MqttTopicMessagesResponse>(
        `/api/v1/mqtt-broker/topics/${encodeURIComponent(topic!)}/messages`,
      ),
    enabled: enabled && !!topic,
    refetchInterval: 3000,
  })
}

export function useMqttMessages(enabled = true) {
  return useQuery({
    queryKey: queryKeys.mqttMessages,
    queryFn: () => fetchJson<MqttMessagesResponse>('/api/v1/mqtt-broker/messages'),
    enabled,
    refetchInterval: 2000,
  })
}

export function useMqttPublish() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ topic, payload }: { topic: string; payload: string }) => {
      const r = await fetchOk('/api/v1/mqtt-broker/publish', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ topic, payload }),
      })
      return r.json()
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.mqttTopics })
      qc.invalidateQueries({ queryKey: queryKeys.mqttMessages })
    },
  })
}
