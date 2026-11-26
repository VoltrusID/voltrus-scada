// What the installed app shows when it opens with no server to reach
// (docs/designer.md, phase 5): the last screen seen on this device with its
// last-known values, each with its age, marked offline. Nothing can be
// opened or written; "Try again" reloads.
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import type { SensorConfig } from '../../types'
import { loadLayoutFromAPI } from '../engine/layout'
import { OperatorView } from './OperatorView'
import { asSensorData, type ScreenSnapshot } from './lastKnown'
// The widgets register themselves when the 2D view loads.
import '../ScadaView'

export function OfflineOperator({ snapshot, onRetry, onSignIn }: { snapshot: ScreenSnapshot; onRetry: () => void; onSignIn: () => void }) {
  const { t } = useTranslation()
  const widgets = useMemo(() => loadLayoutFromAPI(snapshot.equipment, snapshot.connections).widgets, [snapshot])
  const sensors = useMemo(() => snapshot.sensors.map(s => ({
    color: '', warn_val: 0, crit_val: 0, base_val: 0, sort_order: 0, source_type: '', source_config: '', equipment_id: null,
    chart_type: '', path: null, on_delay_secs: 0, off_delay_secs: 0, ...s,
  }) as SensorConfig), [snapshot])
  const values = useMemo(() => asSensorData(snapshot), [snapshot])
  return (
    <div className="viewport-height" data-testid="offline-operator" style={{ display: 'flex', flexDirection: 'column', background: 'var(--bg)', fontFamily: 'var(--font-ui)' }}>
      <OperatorView widgets={widgets} sensors={sensors} values={values} live={false} asOf={snapshot.savedAt}
        screenName={snapshot.screenName} onRetry={onRetry} />
      <div style={{ padding: '10px 14px', borderTop: '1px solid var(--border)', display: 'flex', justifyContent: 'flex-end' }}>
        <button type="button" onClick={onSignIn} style={{
          padding: '8px 14px', minHeight: 40, borderRadius: 6, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', fontSize: 13, cursor: 'pointer',
        }}>{t('operator.signIn')}</button>
      </div>
    </div>
  )
}

export default OfflineOperator
