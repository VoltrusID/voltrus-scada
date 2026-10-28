import { useQuery } from '@tanstack/react-query'
import { fetchJson } from './fetch'
import { queryKeys } from './keys'

// ── Types ──

export interface CompressionStats {
  /** Full-resolution rows not packed yet (the hot window). */
  hot_rows: number
  /** Hourly aggregate rows. */
  warm_rows: number
  /** Daily aggregate rows. */
  cold_rows: number
  /** Compressed blocks of full-resolution history, one per tag and hour. */
  packed_blocks: number
  packed_samples: number
  packed_payload_bytes: number
  packed_bytes_per_sample: number
  db_size_bytes: number
  db_size_mb: number
}

export interface DeadbandEntry {
  device_id: string
  metric: string
  /** `absolute` (engineering units) or `percent` of the last stored value. */
  deadband_type: string
  deadband_value: number
  /** Longest gap between stored samples of a flat signal. */
  max_interval_secs: number | null
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
