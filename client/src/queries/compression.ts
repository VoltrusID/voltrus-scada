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

/** The running retention and packing settings (GET /api/v1/compression/config). */
export interface CompressionConfig {
  enabled: boolean
  pack_enabled: boolean
  pack_after_secs: number
  pack_interval_secs: number
  /** Full-resolution window; null keeps it forever. */
  raw_keep_secs: number | null
  /** Hourly window past the raw window; null keeps it forever. */
  hourly_keep_secs: number | null
  /** Daily window past the hourly window; null keeps it forever. */
  daily_keep_secs: number | null
  /** Expiring full-resolution data is exported to CSV first. */
  archive_raw: boolean
  /** Cleared alarms (with their journal) and downtime events; null keeps them forever. */
  event_keep_secs: number | null
  /** Security event log, as in effect: null when kept forever (always so under IEC 62443). */
  security_keep_secs: number | null
  /** Audit trails, as in effect: null (the default) keeps them forever. Always null under 21 CFR Part 11 or IEC 62443. */
  audit_keep_secs: number | null
  /** Flow run and script execution history; null keeps it forever. */
  run_log_keep_secs: number | null
  /** The licence holds audit trails and the security log whatever retention: says. */
  compliance_hold: boolean
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

export function useCompressionConfig() {
  return useQuery({
    queryKey: queryKeys.compressionConfig,
    queryFn: () => fetchJson<CompressionConfig>('/api/v1/compression/config'),
  })
}
