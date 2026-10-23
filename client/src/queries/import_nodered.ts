import { useMutation } from '@tanstack/react-query'
import { fetchOk } from './fetch'

export interface NodeRedParseResult {
  total_nodes: number
  convertible: number
  manual_review: number
  ui_decoration: number
  unsupported: number
}

export interface NodeRedConversionSummary {
  modbus_devices: number
  mqtt_sources: number
  widgets: number
  flow_nodes: number
  function_nodes: number
  skipped_nodes: number
}

export interface ConvertedModbusDevice {
  device_id: string
  name: string
  ip: string
  port: number
  unit_id: number
  interval_ms: number
  tags: ConvertedTag[]
}

export interface ConvertedTag {
  name: string
  address: number
  data_type: string
  register_area: string
  scale: number
}

export interface ConvertedMqttSource {
  topic: string
  qos: number
  name: string
  direction: string
  source_node_id: string
}

export interface ConvertedWidget {
  widget_type: string
  name: string
  label: string | null
  min_val: number | null
  max_val: number | null
  source_node_id: string
}

export interface ConvertedFlowNode {
  node_type: string
  name: string
  config: Record<string, any>
  source_node_id: string
}

export interface ConvertedFunctionNode {
  node_id: string
  name: string
  source_code: string | null
  warning: string
}

export interface SkippedNode {
  node_id: string
  node_type: string
  name: string | null
  reason: string
}

export interface NodeRedImportResult {
  parse: NodeRedParseResult
  conversion: NodeRedConversionSummary
  details: {
    modbus_devices: ConvertedModbusDevice[]
    mqtt_sources: ConvertedMqttSource[]
    widgets: ConvertedWidget[]
    flow_nodes: ConvertedFlowNode[]
    function_nodes: ConvertedFunctionNode[]
    skipped_nodes: SkippedNode[]
  }
  warnings: string[]
  license: {
    current_devices: number
    new_devices: number
    limit: number | null
  }
}

export function useImportNodeRed() {
  return useMutation({
    mutationFn: async (file: File) => {
      const formData = new FormData()
      formData.append('file', file)
      const r = await fetchOk('/api/v1/import/node-red', {
        method: 'POST',
        body: formData,
      })
      return r.json() as Promise<NodeRedImportResult>
    },
  })
}
