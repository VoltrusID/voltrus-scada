import React from 'react'
import { useCompressionStats } from '../queries/compression'
import { formatBytes } from '../storageFormat'
import { cardStyle, noteStyle, sectionTitle } from './storageStyles'

export const CompressionStats: React.FC = () => {
  const { data: stats, isLoading, isError, refetch } = useCompressionStats()

  if (isLoading) return <div style={noteStyle}>Loading storage statistics...</div>
  if (isError || !stats) {
    return (
      <div role="alert" style={{ ...noteStyle, color: 'var(--crit)' }}>
        Could not load storage statistics.{' '}
        <button onClick={() => refetch()} style={{ background: 'none', border: 'none', color: 'var(--cyan)', cursor: 'pointer', textDecoration: 'underline' }}>Retry</button>
      </div>
    )
  }

  const total = stats.hot_rows + stats.packed_samples + stats.warm_rows + stats.cold_rows
  const nothingStored = total === 0

  return (
    <div>
      <div style={sectionTitle}>History storage</div>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(160px, 1fr))', gap: 10, marginBottom: 12 }}>
        <Stat label="Database size" value={formatBytes(stats.db_size_bytes)} />
        <Stat label="Packed full resolution" value={stats.packed_samples > 0 ? formatBytes(stats.packed_payload_bytes) : '0 B'} />
        <Stat label="Bytes per packed sample" value={stats.packed_samples > 0 ? stats.packed_bytes_per_sample.toFixed(2) : '-'} />
      </div>
      {nothingStored ? (
        <div style={noteStyle}>No history stored yet. Samples appear here once devices report.</div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          <TierBar label="Recent, plain rows" count={stats.hot_rows} unit="rows" total={total} color="var(--warn)"
            detail="Newer than the packing delay; packed in the background." />
          <TierBar label="Packed full resolution" count={stats.packed_samples} unit="samples" total={total} color="var(--ok)"
            detail={stats.packed_samples > 0 ? `${stats.packed_blocks.toLocaleString()} blocks, one per tag and hour` : undefined} />
          <TierBar label="Hourly aggregates" count={stats.warm_rows} unit="rows" total={total} color="var(--cyan)" />
          <TierBar label="Daily aggregates" count={stats.cold_rows} unit="rows" total={total} color="var(--muted)" />
        </div>
      )}
    </div>
  )
}

const Stat: React.FC<{ label: string; value: string }> = ({ label, value }) => (
  <div style={cardStyle}>
    <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>{label}</div>
    <div style={{ fontSize: 20, fontWeight: 700, fontFamily: 'var(--font-mono)' }}>{value}</div>
  </div>
)

const TierBar: React.FC<{
  label: string; count: number; unit: string; total: number; color: string; detail?: string
}> = ({ label, count, unit, total, color, detail }) => {
  const pct = total > 0 ? (count / total) * 100 : 0
  return (
    <div style={cardStyle}>
      <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 13 }}>
        <span>{label}</span>
        <span style={{ color: 'var(--muted)', fontFamily: 'var(--font-mono)' }}>{count.toLocaleString()} {unit}</span>
      </div>
      <div style={{ marginTop: 6, height: 6, background: 'var(--border)', borderRadius: 3, overflow: 'hidden' }}>
        <div style={{ height: '100%', width: `${pct}%`, background: color }} />
      </div>
      {detail && <div style={{ fontSize: 11, color: 'var(--muted)', marginTop: 4 }}>{detail}</div>}
    </div>
  )
}
