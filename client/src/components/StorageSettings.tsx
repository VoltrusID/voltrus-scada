import { CompressionStats } from './CompressionStats'
import { DeadbandConfig } from './DeadbandConfig'
import { RetentionSettings } from './RetentionSettings'

/** System > Storage: what history costs, how long it is kept, which tags thin their writes. */
export function StorageSettings() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 24, maxWidth: 900 }}>
      <CompressionStats />
      <RetentionSettings />
      <DeadbandConfig />
    </div>
  )
}
