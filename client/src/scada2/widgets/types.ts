import type { ComponentType } from 'react'
import type { SensorConfig, SensorData } from '../../types'

// ── Widget type identifiers ──

export type ScadaWidgetType =
  | 'gauge' | 'indicator' | 'bar' | 'status_light'
  | 'tank_level' | 'on_off' | 'label' | 'trend'
  | 'big_number' | 'data_table' | 'pipe'
  | 'slider' | 'number_input' | 'toggle'
  | 'meter' | 'semicircle_gauge' | 'donut' | 'led_display'
  | 'motor' | 'valve' | 'pump' | 'push_button'
  | 'bar_chart' | 'pie_chart' | 'multi_trend'
  | 'annunciator' | 'pid_faceplate' | 'horizontal_tank' | 'progress_bar'
  | 'symbol' | 'area'

// ── Config field schema ──

export type ConfigFieldType = 'string' | 'number' | 'boolean' | 'select' | 'color' | 'tag'

export interface ConfigFieldDef {
  key: string
  label: string
  type: ConfigFieldType
  default: any
  options?: { value: string; label: string }[]
  min?: number
  max?: number
}

// ── Widget manifest ──

export type DataTypeCategory = 'analog' | 'digital' | 'any'

export interface WidgetManifest {
  type: ScadaWidgetType
  displayName: string
  icon: string
  defaultWidth: number
  defaultHeight: number
  minBindings: number
  maxBindings: number
  compatibleDataTypes: DataTypeCategory[]
  configSchema: ConfigFieldDef[]
}

// ── Widget instance (stored in state & API) ──

export interface WidgetInstance {
  id: string
  type: ScadaWidgetType
  sensorId: number | null
  sensorIds: number[]   // multi-sensor binding (for table, etc.)
  x: number
  y: number
  width: number
  height: number
  config: Record<string, any>
}

// ── Connection instance ──

export type ConnectionType = 'power' | 'data' | 'flow'

export interface ConnectionInstance {
  id: string
  fromWidgetId: string
  toWidgetId: string
  connectionType: ConnectionType
  waypoints: [number, number][]
}

// ── Widget component props ──

export interface WidgetProps {
  instance: WidgetInstance
  sensor: SensorConfig | undefined
  data: SensorData | undefined
  sensors: SensorConfig[]
  currentValues: Record<string, SensorData>
  editMode: boolean
  selected: boolean
  onWriteValue?: (value: number) => void
}

// ── Registry entry ──

export interface WidgetRegistryEntry {
  manifest: WidgetManifest
  component: ComponentType<WidgetProps>
}

// ── Canvas state ──

export type ScadaTool = 'select' | 'connect' | 'label' | 'delete' | 'delete_connection'

export interface ScadaCanvasState {
  editMode: boolean
  activeTool: ScadaTool
  selectedId: string | null
  connectFrom: string | null
  pendingSensor: { sensorId: number; x: number; y: number } | null
}

// ── Visual rule (kept from old engine) ──

export interface VisualRule {
  target: 'fill' | 'stroke' | 'opacity' | 'visible' | 'strokeWidth'
  expression: string
}
