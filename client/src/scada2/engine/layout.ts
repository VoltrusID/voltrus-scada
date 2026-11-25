import type { ScadaEquipmentItem, ScadaConnectionItem } from '../../types'
import type { ConnectionInstance, ScadaWidgetType, WidgetInstance } from '../widgets/types'
import type { ScadaLayoutState } from './widgetState'

/** Parse a screen's saved SCADA rows into the editor/runtime layout. Shared by the 2D view and the 3D plant twin. */
export function loadLayoutFromAPI(equipment: ScadaEquipmentItem[], connections: ScadaConnectionItem[]): ScadaLayoutState {
  // Stacking order travels in config.z: the server returns rows sorted by id, not insertion order.
  const parsed = equipment.map((eq, i) => {
    const config = JSON.parse(eq.config || '{}') as Record<string, any>
    const z = typeof config.z === 'number' ? config.z : i
    delete config.z
    // Rows written by the API or a project file may carry the rotation only in its column.
    if (config.rotation == null && eq.rotation) config.rotation = eq.rotation
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
      // Kept so a save does not rename equipment that the API or MCP named.
      ...(eq.equipment_name ? { name: eq.equipment_name } : {}),
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

/**
 * The rows the editor saves (PUT .../scada-equipment and .../scada-connections).
 * `fallbackName` names a widget that has neither a label nor a loaded name.
 * Loading rows and saving them back gives the same rows; see
 * tests/scadaRoundTrip.test.ts.
 */
export function saveLayoutToAPI(
  layout: ScadaLayoutState,
  fallbackName: (w: WidgetInstance) => string,
): { equipment: ScadaEquipmentItem[]; connections: ScadaConnectionItem[] } {
  const equipment: ScadaEquipmentItem[] = layout.widgets.map((w, i) => ({
    equipment_id: w.id,
    equipment_type: w.type as ScadaEquipmentItem['equipment_type'],
    equipment_name: w.config?.label || w.name || fallbackName(w),
    x: w.x,
    y: w.y,
    width: w.width,
    height: w.height,
    rotation: Number(w.config?.rotation ?? 0),
    sensor_bindings: JSON.stringify(w.sensorIds ?? (w.sensorId != null ? [w.sensorId] : [])),
    config: JSON.stringify({ ...w.config, z: i }),
  }))
  const connections: ScadaConnectionItem[] = layout.connections.map(c => ({
    connection_id: c.id,
    from_equipment: c.fromWidgetId,
    to_equipment: c.toWidgetId,
    waypoints: JSON.stringify(c.waypoints),
    connection_type: c.connectionType,
  }))
  return { equipment, connections }
}
