// The operator view fed by the live store (docs/designer.md, phase 5): it is
// live while the live stream is connected and the browser is online, keeps a
// last-known snapshot of the screen while live, and says how old the values
// are once it is not.
import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import type { ScadaConnectionItem, ScadaEquipmentItem, SensorConfig } from '../../types'
import { useTrackedCurrentValues, useWsConnected } from '../../store/dashboardStore'
import { OperatorView, type OperatorViewProps } from './OperatorView'
import { pickValues, saveSnapshot, snapshotUser, tagIdsOf } from './lastKnown'

/** How often the snapshot is refreshed while live. */
export const SNAPSHOT_EVERY_MS = 15_000

function subscribeOnline(cb: () => void) {
  window.addEventListener('online', cb)
  window.addEventListener('offline', cb)
  return () => { window.removeEventListener('online', cb); window.removeEventListener('offline', cb) }
}

/** `navigator.onLine`, following the browser's online and offline events. */
export function useOnline(): boolean {
  return useSyncExternalStore(subscribeOnline, () => navigator.onLine, () => true)
}

type Props = Omit<OperatorViewProps, 'values' | 'live' | 'asOf'> & {
  screen: string
  equipment: ScadaEquipmentItem[]
  connections: ScadaConnectionItem[]
}

export function LiveOperatorView({ screen, equipment, connections, ...rest }: Props) {
  const values = useTrackedCurrentValues()
  const connected = useWsConnected()
  const online = useOnline()
  const live = connected && online
  const [lastLive, setLastLive] = useState<number | null>(null)
  const valuesRef = useRef(values)
  valuesRef.current = values
  const inputs = useRef({ equipment, connections, sensors: rest.sensors, widgets: rest.widgets, name: rest.screenName ?? screen })
  inputs.current = { equipment, connections, sensors: rest.sensors, widgets: rest.widgets, name: rest.screenName ?? screen }

  // While live: remember when, and keep the snapshot fresh for when it is not.
  useEffect(() => {
    if (!live) return
    const save = () => {
      const now = Date.now()
      setLastLive(now)
      const { equipment, connections, sensors, widgets, name } = inputs.current
      if (equipment.length === 0) return
      const ids = tagIdsOf(widgets)
      try {
        saveSnapshot(localStorage, snapshotUser(), {
          screen, screenName: name, savedAt: now, equipment, connections,
          sensors: sensors.filter(s => ids.has(s.id)).map(pickSensor),
          values: pickValues(ids, valuesRef.current),
        })
      } catch { /* no storage: nothing kept */ }
    }
    // The first values arrive a moment after the socket opens.
    const first = setTimeout(save, 2000)
    const every = setInterval(save, SNAPSHOT_EVERY_MS)
    return () => { clearTimeout(first); clearInterval(every); setLastLive(Date.now()) }
  }, [live, screen])

  return <OperatorView {...rest} values={values} live={live} asOf={live ? null : lastLive} />
}

const pickSensor = (s: SensorConfig) => ({
  id: s.id, key: s.key, name: s.name, unit: s.unit, decimals: s.decimals, device_id: s.device_id, min_val: s.min_val, max_val: s.max_val,
})
