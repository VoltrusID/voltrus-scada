import { Component, useEffect, useRef, useState, type ReactNode } from 'react'
import { resolveScreenWidget, type ScreenSurface } from '../edition/widgets'
import type { SensorConfig, SensorData } from '../types'

interface Props {
  type: string
  config: Record<string, unknown>
  surface: ScreenSurface
  width: number
  height: number
  editMode: boolean
  sensors: SensorConfig[]
  values: Record<string, SensorData>
}

/** Draws an add-on widget, or a placeholder when this build does not have its type. */
export function ScreenWidgetView({ type, config, surface, width, height, editMode, sensors, values }: Props) {
  const resolved = resolveScreenWidget(type, surface)
  if (!resolved.available) return <WidgetPlaceholder text={resolved.message} detail={type} />
  const Render = resolved.def.render
  return (
    <WidgetErrorBoundary type={type}>
      <Render config={config} width={width} height={height} editMode={editMode} surface={surface} sensors={sensors} values={values} />
    </WidgetErrorBoundary>
  )
}

export function WidgetPlaceholder({ text, detail }: { text: string; detail?: string }) {
  return (
    <div style={{
      width: '100%', height: '100%', display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center',
      gap: 4, padding: 8, boxSizing: 'border-box', textAlign: 'center',
      border: '1px dashed var(--border)', borderRadius: 6, color: 'var(--muted)', fontSize: 12,
    }}>
      <span>{text}</span>
      {detail && <span style={{ fontSize: 10, fontFamily: 'var(--font-mono)', opacity: 0.7 }}>{detail}</span>}
    </div>
  )
}

/**
 * A broken widget shows a placeholder instead of taking the whole screen down.
 * `fallback` replaces the default placeholder; a new `resetKey` (the edited
 * widget, say) tries drawing it again.
 */
export class WidgetErrorBoundary extends Component<
  { type: string; children: ReactNode; fallback?: ReactNode; resetKey?: unknown },
  { failed: boolean; key: unknown }
> {
  state = { failed: false, key: this.props.resetKey }
  static getDerivedStateFromError() {
    return { failed: true }
  }
  static getDerivedStateFromProps(props: { resetKey?: unknown }, state: { failed: boolean; key: unknown }) {
    return props.resetKey !== state.key ? { failed: false, key: props.resetKey } : null
  }
  componentDidCatch(error: unknown) {
    console.warn(`Widget ${this.props.type} failed to draw`, error)
  }
  render() {
    if (!this.state.failed) return this.props.children
    return this.props.fallback ?? <WidgetPlaceholder text="This widget failed to load" detail={this.props.type} />
  }
}

/** Tracks an element's size in CSS pixels. */
export function useElementSize<T extends HTMLElement>() {
  const ref = useRef<T>(null)
  const [size, setSize] = useState({ width: 0, height: 0 })
  useEffect(() => {
    const el = ref.current
    if (!el || typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect
      setSize(prev => (prev.width === Math.round(width) && prev.height === Math.round(height) ? prev : { width: Math.round(width), height: Math.round(height) }))
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [])
  return { ref, ...size }
}
