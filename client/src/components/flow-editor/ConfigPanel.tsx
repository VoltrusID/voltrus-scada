import { NODE_META, TRIGGER_META, TRIGGER_DEFAULTS } from './constants'
import type { FlowNodeConfig } from '../../types'

interface Props {
  node: FlowNodeConfig | null
  trigger: any
  onNodeChange: (key: string, value: any) => void
  onTriggerChange: (key: string, value: any) => void
  onTriggerTypeChange: (type: string) => void
}

export function ConfigPanel({ node, trigger, onNodeChange, onTriggerChange, onTriggerTypeChange }: Props) {
  const inputStyle: React.CSSProperties = {
    background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 6,
    padding: '6px 10px', color: 'var(--fg)', fontSize: 12,
    fontFamily: "'var(--font-mono)', monospace", width: '100%',
  }
  const labelStyle: React.CSSProperties = {
    fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase',
    letterSpacing: 0.5, marginBottom: 4, display: 'block', fontWeight: 600,
  }

  const isTrigger = !node
  const meta = isTrigger ? TRIGGER_META[trigger?.type || 'modbus'] : NODE_META[node!.type]
  const color = meta?.color || '#9ca3af'
  const label = meta?.label || (isTrigger ? trigger?.type : node?.type) || 'Unknown'

  return (
    <div style={{
      width: 300, flexShrink: 0, borderLeft: '1px solid var(--border)',
      background: 'var(--card)', overflow: 'auto', display: 'flex', flexDirection: 'column',
    }}>
      <div style={{ padding: '14px 16px', borderBottom: '1px solid var(--border)' }}>
        <div style={{ fontSize: 11, fontWeight: 700, textTransform: 'uppercase', letterSpacing: 1, color: 'var(--muted)' }}>Configuration</div>
      </div>

      <div style={{ padding: '16px', display: 'flex', flexDirection: 'column', gap: 14 }}>
        {/* Header */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
          <span style={{ width: 8, height: 8, borderRadius: '50%', background: color }} />
          <span style={{ fontSize: 13, fontWeight: 700 }}>{label}</span>
        </div>

        {isTrigger ? (
          <>
            <div>
              <label style={labelStyle}>Trigger Type</label>
              <select style={inputStyle} value={trigger?.type || 'modbus'} onChange={e => onTriggerTypeChange(e.target.value)}>
                <option value="modbus">Modbus</option>
                <option value="timer">Timer</option>
                <option value="mqtt">MQTT</option>
              </select>
            </div>
            {trigger?.type === 'modbus' && (
              <>
                <div>
                  <label style={labelStyle}>Device ID</label>
                  <input style={inputStyle} value={trigger.device_id || ''} onChange={e => onTriggerChange('device_id', e.target.value)} />
                </div>
                <div>
                  <label style={labelStyle}>Metric (optional)</label>
                  <input style={inputStyle} value={trigger.metric || ''} onChange={e => onTriggerChange('metric', e.target.value)} />
                </div>
                <div>
                  <label style={labelStyle}>Debounce (ms)</label>
                  <input style={inputStyle} type="number" value={trigger.debounce_ms || 1000} onChange={e => onTriggerChange('debounce_ms', Number(e.target.value))} />
                </div>
                <label style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 12, cursor: 'pointer' }}>
                  <input type="checkbox" checked={trigger.on_change_only || false} onChange={e => onTriggerChange('on_change_only', e.target.checked)} />
                  On change only
                </label>
              </>
            )}
            {trigger?.type === 'timer' && (
              <div>
                <label style={labelStyle}>Interval (ms)</label>
                <input style={inputStyle} type="number" value={trigger.interval_ms || 5000} onChange={e => onTriggerChange('interval_ms', Number(e.target.value))} />
              </div>
            )}
            {trigger?.type === 'mqtt' && (
              <>
                <div>
                  <label style={labelStyle}>Topic</label>
                  <input style={inputStyle} value={trigger.topic || ''} onChange={e => onTriggerChange('topic', e.target.value)} />
                </div>
                <div>
                  <label style={labelStyle}>Debounce (ms)</label>
                  <input style={inputStyle} type="number" value={trigger.debounce_ms || 1000} onChange={e => onTriggerChange('debounce_ms', Number(e.target.value))} />
                </div>
              </>
            )}
          </>
        ) : (
          <NodeFormFields node={node!} onChange={onNodeChange} inputStyle={inputStyle} labelStyle={labelStyle} />
        )}
      </div>
    </div>
  )
}

function NodeFormFields({ node, onChange, inputStyle, labelStyle }: {
  node: FlowNodeConfig
  onChange: (k: string, v: any) => void
  inputStyle: React.CSSProperties
  labelStyle: React.CSSProperties
}) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
      {Object.entries(node).filter(([k]) => k !== 'id' && k !== 'type').map(([key, value]) => {
        if (typeof value === 'boolean') {
          return (
            <label key={key} style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 12, cursor: 'pointer' }}>
              <input type="checkbox" checked={value} onChange={e => onChange(key, e.target.checked)} />
              {key}
            </label>
          )
        }
        if (typeof value === 'number') {
          return (
            <div key={key}>
              <label style={labelStyle}>{key}</label>
              <input style={inputStyle} type="number" value={value} onChange={e => onChange(key, Number(e.target.value))} />
            </div>
          )
        }
        if (typeof value === 'object') {
          return (
            <div key={key}>
              <label style={labelStyle}>{key}</label>
              <textarea
                style={{ ...inputStyle, minHeight: 80, resize: 'vertical' }}
                value={JSON.stringify(value, null, 2)}
                onChange={e => { try { onChange(key, JSON.parse(e.target.value)) } catch { onChange(key, e.target.value) } }}
              />
            </div>
          )
        }
        return (
          <div key={key}>
            <label style={labelStyle}>{key}</label>
            <input style={inputStyle} value={value} onChange={e => onChange(key, e.target.value)} />
          </div>
        )
      })}
    </div>
  )
}
