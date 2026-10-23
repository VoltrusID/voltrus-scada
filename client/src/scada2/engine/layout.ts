import type { ScadaEquipmentItem, ScadaConnectionItem } from '../../types'
import type { ConnectionInstance, ScadaWidgetType } from '../widgets/types'
import type { ScadaLayoutState } from './widgetState'

/** Parse a screen's saved SCADA rows into the editor/runtime layout. Shared by the 2D view and the 3D plant twin. */
export function loadLayoutFromAPI(equipment: ScadaEquipmentItem[], connections: ScadaConnectionItem[]): ScadaLayoutState {
  // Stacking order travels in config.z: the server returns rows sorted by id, not insertion order.
  const parsed = equipment.map((eq, i) => {
    const config = JSON.parse(eq.config || '{}') as Record<string, any>
    const z = typeof config.z === 'number' ? config.z : i
    delete config.z
    return { eq, config, z }
  }).sort((a, b) => a.z - b.z)
  const widgets = parsed.map(({ eq, config }) => {
    const bindings: number[] = JSON.parse(eq.sensor_bindings || '[]')
    return {
      id: eq.equipment_id,
      type: eq.equipment_type as ScadaWidgetType,
      sensorId: bindings[0] ?? null,
      sensorIds: bindings,
      x: eq.x,
      y: eq.y,
      width: eq.width,
      height: eq.height,
      config,
    }
  })
  const conns: ConnectionInstance[] = connections.map(c => ({
    id: c.connection_id,
    fromWidgetId: c.from_equipment,
    toWidgetId: c.to_equipment,
    connectionType: c.connection_type as ConnectionInstance['connectionType'],
    waypoints: JSON.parse(c.waypoints || '[]'),
  }))
  return { widgets, connections: conns }
}
