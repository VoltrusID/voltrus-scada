import { EquipmentType } from '../types'

export interface DefaultEquipment {
  id: string
  type: EquipmentType
  name: string
  position: [number, number, number]
  rotation: number
}

export const DEFAULT_EQUIPMENT: DefaultEquipment[] = [
  { id: 'solar_panel_1', type: 'solar_panel', name: 'Solar Panel Array', position: [-3, 0, 0], rotation: 0 },
  { id: 'inverter_1', type: 'inverter', name: 'Inverter', position: [2, 0, 0], rotation: 0 },
  { id: 'gateway_1', type: 'gateway', name: 'Gateway (PLC)', position: [4, 0, 2], rotation: 0 },
]