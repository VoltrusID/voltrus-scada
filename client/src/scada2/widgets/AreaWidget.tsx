import React from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

/** Titled process-area frame. Sits behind equipment (send to back) to group a stage of the process. */
const AreaWidget: React.FC<WidgetProps> = ({ instance }) => {
  const title: string = instance.config?.title ?? 'Process Area'
  const subtitle: string = instance.config?.subtitle ?? ''
  return (
    <div style={{
      width: '100%', height: '100%', boxSizing: 'border-box', position: 'relative',
      background: 'var(--hmi-area)', border: '1px solid color-mix(in srgb, var(--hmi-stroke) 22%, transparent)', borderRadius: 4,
    }}>
      <div style={{
        position: 'absolute', top: 0, left: 0, display: 'flex', alignItems: 'baseline', gap: 8,
        padding: '5px 10px 4px', borderBottomRightRadius: 4,
        background: 'color-mix(in srgb, var(--hmi-stroke) 10%, transparent)',
        borderRight: '1px solid color-mix(in srgb, var(--hmi-stroke) 22%, transparent)',
        borderBottom: '1px solid color-mix(in srgb, var(--hmi-stroke) 22%, transparent)',
        whiteSpace: 'nowrap', userSelect: 'none',
      }}>
        <span style={{ fontSize: 12, fontWeight: 700, letterSpacing: '.08em', textTransform: 'uppercase', color: 'var(--text-dim)' }}>{title}</span>
        {subtitle && <span style={{ fontSize: 11, color: 'var(--text-muted)', fontFamily: 'var(--font-mono)' }}>{subtitle}</span>}
      </div>
    </div>
  )
}

registerWidget({ manifest: WIDGET_MANIFESTS.area, component: AreaWidget })

export default AreaWidget
