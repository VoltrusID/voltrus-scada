import React from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const PIDFaceplateWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensor, data } = props
  const config = instance.config ?? {}
  const showOutput = config.showOutput !== false
  const showBars = config.showBars !== false

  const min = sensor?.min_val ?? 0
  const max = sensor?.max_val ?? 100
  const warn = sensor?.warn_val ?? min + (max - min) * 0.75
  const crit = sensor?.crit_val ?? min + (max - min) * 0.9
  const decimals = sensor?.decimals ?? 1
  const unit = sensor?.unit ?? ''

  const value = data?.value ?? min
  const range = max - min || 1
  const pct = Math.max(0, Math.min(1, (value - min) / range))
  const setpoint = config.setpoint ?? (min + max) / 2
  const setpointPct = Math.max(0, Math.min(1, (setpoint - min) / range))

  const output = config.output ?? value
  const outputPct = Math.max(0, Math.min(1, (output - min) / range))

  const valueColor = pct >= (crit - min) / range
    ? 'var(--crit)' : pct >= (warn - min) / range
    ? 'var(--warn)' : 'var(--ok)'

  const barH = 8
  const barW = 160
  const barX = 20

  return (
    <div style={{
      width: '100%', height: '100%', display: 'flex', flexDirection: 'column',
      alignItems: 'center', justifyContent: 'center', gap: 4,
      padding: 6, fontSize: 10, fontFamily: 'monospace',
    }}>
      <div style={{
        fontSize: 9, fontWeight: 700, color: 'var(--muted)',
        letterSpacing: 1, textTransform: 'uppercase', width: '100%',
        textAlign: 'center', borderBottom: '1px solid var(--border)', paddingBottom: 3,
      }}>
        PID
      </div>

      <div style={{ width: '100%', display: 'flex', flexDirection: 'column', gap: 3 }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
          <span style={{ color: 'var(--muted)', fontSize: 8 }}>PV</span>
          <span style={{ color: valueColor, fontWeight: 700, fontSize: 14 }}>
            {data?.value !== undefined ? value.toFixed(decimals) : '--'}
          </span>
          <span style={{ color: 'var(--muted)', fontSize: 8 }}>{unit}</span>
        </div>

        {showBars && (
          <div style={{ position: 'relative', height: barH, backgroundColor: 'var(--border)', borderRadius: barH / 2 }}>
            <div style={{
              position: 'absolute', top: 0, left: 0, height: barH,
              width: `${pct * 100}%`, backgroundColor: valueColor,
              borderRadius: barH / 2, transition: 'width 0.4s ease',
            }} />
            <div style={{
              position: 'absolute', top: -2, height: barH + 4,
              left: `${setpointPct * 100}%`, width: 2,
              backgroundColor: 'var(--fg)', opacity: 0.7,
            }} />
          </div>
        )}

        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
          <span style={{ color: 'var(--muted)', fontSize: 8 }}>SP</span>
          <span style={{ color: 'var(--fg)', fontWeight: 600 }}>{setpoint.toFixed(decimals)}</span>
          <span style={{ color: 'var(--muted)', fontSize: 8 }}>{unit}</span>
        </div>

        {showBars && (
          <div style={{ position: 'relative', height: barH, backgroundColor: 'var(--border)', borderRadius: barH / 2 }}>
            <div style={{
              position: 'absolute', top: 0, left: 0, height: barH,
              width: `${setpointPct * 100}%`, backgroundColor: 'var(--fg)',
              borderRadius: barH / 2, opacity: 0.4, transition: 'width 0.4s ease',
            }} />
          </div>
        )}

        {showOutput && (
          <>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
              <span style={{ color: 'var(--muted)', fontSize: 8 }}>OUT</span>
              <span style={{ color: 'var(--cyan)', fontWeight: 600 }}>
                {outputPct !== undefined ? `${(outputPct * 100).toFixed(1)}%` : '--'}
              </span>
            </div>
            {showBars && (
              <div style={{ position: 'relative', height: barH, backgroundColor: 'var(--border)', borderRadius: barH / 2 }}>
                <div style={{
                  position: 'absolute', top: 0, left: 0, height: barH,
                  width: `${outputPct * 100}%`, backgroundColor: 'var(--cyan)',
                  borderRadius: barH / 2, opacity: 0.6, transition: 'width 0.4s ease',
                }} />
              </div>
            )}
          </>
        )}

        <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 7, color: 'var(--muted)', marginTop: 2 }}>
          <span>Mode: {config.mode ?? 'AUTO'}</span>
          <span>P: {config.pGain ?? '1.0'} I: {config.iGain ?? '0.1'} D: {config.dGain ?? '0.0'}</span>
        </div>
      </div>

      <div style={{
        display: 'flex', gap: 6, marginTop: 2, width: '100%', justifyContent: 'center',
      }}>
        <span style={{
          padding: '1px 6px', borderRadius: 3, fontSize: 7, fontWeight: 600,
          backgroundColor: (config.mode ?? 'AUTO') === 'AUTO' ? 'var(--ok)' : 'var(--border)',
          color: (config.mode ?? 'AUTO') === 'AUTO' ? 'var(--bg)' : 'var(--muted)',
        }}>AUTO</span>
        <span style={{
          padding: '1px 6px', borderRadius: 3, fontSize: 7, fontWeight: 600,
          backgroundColor: config.mode === 'MAN' ? 'var(--warn)' : 'var(--border)',
          color: config.mode === 'MAN' ? 'var(--bg)' : 'var(--muted)',
        }}>MAN</span>
        <span style={{
          padding: '1px 6px', borderRadius: 3, fontSize: 7, fontWeight: 600,
          backgroundColor: config.mode === 'CAS' ? 'var(--cyan)' : 'var(--border)',
          color: config.mode === 'CAS' ? 'var(--bg)' : 'var(--muted)',
        }}>CAS</span>
      </div>
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.pid_faceplate,
  component: PIDFaceplateWidget,
})

export default PIDFaceplateWidget
