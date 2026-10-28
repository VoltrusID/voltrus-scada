import React from 'react'
import { useCompressionDeadband } from '../queries/compression'
import { cardStyle, noteStyle, sectionTitle } from './storageStyles'

const th: React.CSSProperties = { textAlign: 'left', padding: '6px 8px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', borderBottom: '1px solid var(--border)' }
const td: React.CSSProperties = { padding: '6px 8px', fontSize: 13, borderBottom: '1px solid var(--border)' }

/** Tags that filter their history writes (GET /api/v1/compression/deadband). */
export const DeadbandConfig: React.FC = () => {
  const { data: entries, isLoading, isError, refetch } = useCompressionDeadband()

  return (
    <div>
      <div style={sectionTitle}>Tags with a history deadband</div>
      {isLoading && <div style={noteStyle}>Loading deadband settings...</div>}
      {isError && (
        <div role="alert" style={{ ...noteStyle, color: 'var(--crit)' }}>
          Could not load deadband settings.{' '}
          <button onClick={() => refetch()} style={{ background: 'none', border: 'none', color: 'var(--cyan)', cursor: 'pointer', textDecoration: 'underline' }}>Retry</button>
        </div>
      )}
      {entries && entries.length === 0 && (
        <div style={cardStyle}>
          No tag filters its history, so every sample is stored. Set a history deadband on a tag
          (Data Manager, Sources, select tags and choose History deadband) to store far less for steady signals.
        </div>
      )}
      {entries && entries.length > 0 && (
        <div style={{ ...cardStyle, padding: 0, overflow: 'auto' }}>
          <table style={{ width: '100%', borderCollapse: 'collapse' }}>
            <thead>
              <tr>
                <th style={th}>Device</th>
                <th style={th}>Metric</th>
                <th style={th}>Band</th>
                <th style={th}>Heartbeat</th>
              </tr>
            </thead>
            <tbody>
              {entries.map(e => (
                <tr key={`${e.device_id}/${e.metric}`}>
                  <td style={td}>{e.device_id}</td>
                  <td style={{ ...td, fontFamily: 'var(--font-mono)' }}>{e.metric}</td>
                  <td style={{ ...td, fontFamily: 'var(--font-mono)' }}>{e.deadband_value}{e.deadband_type === 'percent' ? ' %' : ''}</td>
                  <td style={{ ...td, fontFamily: 'var(--font-mono)' }}>{e.max_interval_secs ? `${e.max_interval_secs} s` : 'none'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}
