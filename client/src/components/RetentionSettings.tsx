import React from 'react'
import { useCompressionConfig } from '../queries/compression'
import { eventRetentionRows, formatDuration } from '../storageFormat'
import { cardStyle, noteStyle, sectionTitle } from './storageStyles'

/**
 * The running retention and packing settings. They come from the `retention:` block of
 * config.yaml and need a restart to change, so this panel is read-only.
 */
export const RetentionSettings: React.FC = () => {
  const { data: cfg, isLoading, isError, refetch } = useCompressionConfig()

  if (isLoading) return <div style={noteStyle}>Loading retention settings...</div>
  if (isError || !cfg) {
    return (
      <div role="alert" style={{ ...noteStyle, color: 'var(--crit)' }}>
        Could not load retention settings.{' '}
        <button onClick={() => refetch()} style={{ background: 'none', border: 'none', color: 'var(--cyan)', cursor: 'pointer', textDecoration: 'underline' }}>Retry</button>
      </div>
    )
  }

  const rows: [string, string, string][] = [
    ['Retention worker', cfg.enabled ? 'on' : 'off', 'retention.enabled. When off, nothing is packed or expired (expired login tokens are still removed).'],
    ['Packing', cfg.pack_enabled ? 'on' : 'off', 'retention.pack_enabled. Packs full-resolution samples into compressed blocks.'],
    ['Pack after', formatDuration(cfg.pack_after_secs), 'retention.pack_after_secs. Complete hours older than this are packed.'],
    ['Pack every', formatDuration(cfg.pack_interval_secs), 'retention.pack_interval_secs.'],
    ['Full resolution kept', formatDuration(cfg.raw_keep_secs), 'tier raw, keep. Then rolled into hourly aggregates.'],
    ['Hourly kept', formatDuration(cfg.hourly_keep_secs), 'tier hourly, keep, counted past the full-resolution window.'],
    ['Daily kept', formatDuration(cfg.daily_keep_secs), 'tier daily, keep, counted past the hourly window.'],
    ['Archive expiring data', cfg.archive_raw ? 'on' : 'off', 'tier raw, archive. Exports what leaves full resolution to CSV before deleting it; the files are large.'],
    ...eventRetentionRows(cfg),
  ]

  return (
    <div>
      <div style={sectionTitle}>Retention and packing</div>
      <div style={{ ...cardStyle, padding: 0 }}>
        {rows.map(([label, value, help]) => (
          <div key={label} style={{ display: 'grid', gridTemplateColumns: '180px 100px 1fr', gap: 8, padding: '8px 12px', borderBottom: '1px solid var(--border)', fontSize: 13, alignItems: 'baseline' }}>
            <span>{label}</span>
            <span style={{ fontFamily: 'var(--font-mono)', fontWeight: 600 }}>{value}</span>
            <span style={{ fontSize: 11, color: 'var(--muted)' }}>{help}</span>
          </div>
        ))}
      </div>
      <div style={noteStyle}>
        These come from the <code>retention:</code> block of config.yaml and apply after a restart.
        See docs/historian.md for what each tier costs on disk.
      </div>
    </div>
  )
}
