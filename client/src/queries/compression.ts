import { useQuery } from '@tanstack/react-query'
import { fetchJson } from './fetch'
import { queryKeys } from './keys'

// ── Types ──

export interface CompressionStats {
  hot_rows: number
  warm_rows: number
  cold_rows: number
  db_size_bytes: number
  db_size_mb: number
  estimated_hot_mb: number
  estimated_warm_mb: number
  estimated_cold_mb: number
}

export interface DeadbandEntry {
  device_id: string
  metric: string
  deadband_type: string
  deadband_value: number
}

// ── Hooks ──

export function useCompressionStats() {
  return useQuery({
    queryKey: queryKeys.compressionStats,
    queryFn: () => fetchJson<CompressionStats>('/api/v1/compression/stats'),
    refetchInterval: 30_000,
  })
}

export function useCompressionDeadband() {
  return useQuery({
    queryKey: queryKeys.compressionDeadband,
    queryFn: () => fetchJson<DeadbandEntry[]>('/api/v1/compression/deadband'),
  })
}
