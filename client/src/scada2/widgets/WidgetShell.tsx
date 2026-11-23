import { useCallback, useRef, useState } from 'react'
import { Rnd } from 'react-rnd'
import type { SensorConfig, SensorData } from '../../types'
import type { WidgetInstance } from './types'
import { getWidgetComponent } from './registry'
import { ScreenWidgetView } from '../../components/ScreenWidgetView'

interface WidgetShellProps {
  instance: WidgetInstance
  sensor: SensorConfig | undefined
  data: SensorData | undefined
  sensors: SensorConfig[]
  currentValues: Record<string, SensorData>
  editMode: boolean
  selected: boolean
  scale: number
  /** Connect tool active: clicks pick endpoints instead of dragging. */
  locked?: boolean
  onSelect: (id: string) => void
  onMove: (id: string, x: number, y: number) => void
  onResize: (id: string, x: number, y: number, width: number, height: number) => void
  onWriteValue?: (value: number) => void
  /** In run mode, activating the widget does something (opens its faceplate): make it a keyboard button. */
  activatable?: boolean
}

// SVG symbols draw strokes right at their edge, so they must not be clipped.
const UNCLIPPED = new Set(['area', 'symbol', 'pump', 'motor', 'valve', 'tank_level', 'horizontal_tank', 'pipe'])

const handle: React.CSSProperties = {
  width: 8, height: 8, background: 'var(--card)', border: '1.5px solid var(--cyan)', borderRadius: 2,
}
const corner = (pos: React.CSSProperties): React.CSSProperties => ({ ...handle, ...pos })
const HANDLES = {
  topLeft: corner({ left: -4, top: -4 }),
  topRight: corner({ right: -4, top: -4 }),
  bottomLeft: corner({ left: -4, bottom: -4 }),
  bottomRight: corner({ right: -4, bottom: -4 }),
}

export default function WidgetShell({
  instance, sensor, data, sensors, currentValues,
  editMode, selected, scale, locked, onSelect, onMove, onResize, onWriteValue, activatable = false,
}: WidgetShellProps) {
  const [hover, setHover] = useState(false)

  // A click selects; only a real pointer drag moves. Selecting opens the inspector and can rescale
  // the stage mid-click, which react-rnd would otherwise report as a drag to a bogus position.
  const dragged = useRef(false)
  const handleDragStop = useCallback(
    (_e: unknown, d: { x: number; y: number }) => {
      const moved = dragged.current
      dragged.current = false
      if (moved && (d.x !== instance.x || d.y !== instance.y)) onMove(instance.id, Math.round(d.x), Math.round(d.y))
    },
    [onMove, instance.id, instance.x, instance.y],
  )

  const handleResizeStop = useCallback(
    (_e: MouseEvent | TouchEvent, _dir: string, ref: HTMLElement, _delta: unknown, position: { x: number; y: number }) => {
      onResize(instance.id, Math.round(position.x), Math.round(position.y), Math.round(ref.offsetWidth), Math.round(ref.offsetHeight))
    },
    [onResize, instance.id],
  )

  // Types core does not know are add-on widgets (or ones this build lacks, drawn as a placeholder).
  const WidgetComponent = getWidgetComponent(instance.type)

  // At runtime `selected` marks the equipment whose faceplate is open.
  const outline = !editMode ? (selected ? '1.5px solid var(--cyan)' : 'none')
    : selected ? '1.5px solid var(--cyan)'
    : hover ? '1px dashed color-mix(in srgb, var(--cyan) 60%, transparent)'
    : '1px dashed transparent'

  return (
    <Rnd
      position={{ x: instance.x, y: instance.y }}
      size={{ width: instance.width, height: instance.height }}
      scale={scale}
      disableDragging={!editMode || locked}
      enableResizing={editMode && selected && !locked}
      onDragStart={() => { dragged.current = false }}
      onDrag={() => { dragged.current = true }}
      onDragStop={handleDragStop}
      onResizeStop={handleResizeStop}
      minWidth={16}
      minHeight={10}
      bounds="parent"
      dragGrid={[10, 10]}
      resizeGrid={[10, 10]}
      resizeHandleStyles={selected ? HANDLES : undefined}
      style={{ display: 'flex', zIndex: selected ? 5 : 2 }}
    >
      <div
        onMouseDown={editMode ? () => onSelect(instance.id) : undefined}
        onClick={editMode ? undefined : () => onSelect(instance.id)}
        {...(!editMode && activatable ? {
          role: 'button',
          tabIndex: 0,
          'aria-label': `${sensor?.name ?? instance.type}: open faceplate`,
          'aria-pressed': selected,
          onKeyDown: (e: React.KeyboardEvent) => {
            if (e.target !== e.currentTarget || (e.key !== 'Enter' && e.key !== ' ')) return
            e.preventDefault()
            onSelect(instance.id)
          },
        } : {})}
        onMouseEnter={() => setHover(true)}
        onMouseLeave={() => setHover(false)}
        style={{
          width: '100%', height: '100%', display: 'flex', flexDirection: 'column',
          outline, outlineOffset: 2,
          cursor: editMode ? 'move' : 'default',
          background: instance.type === 'data_table' ? 'var(--card)' : 'transparent',
          borderRadius: instance.type === 'data_table' ? 6 : 0,
        }}
      >
        <div style={{ flex: 1, minHeight: 0, overflow: UNCLIPPED.has(instance.type) ? 'visible' : 'hidden', pointerEvents: editMode ? 'none' : undefined }}>
          {WidgetComponent ? (
            <WidgetComponent
              instance={instance}
              sensor={sensor}
              data={data}
              sensors={sensors}
              currentValues={currentValues}
              editMode={editMode}
              selected={selected}
              onWriteValue={onWriteValue}
            />
          ) : (
            <ScreenWidgetView
              type={instance.type}
              config={instance.config ?? {}}
              surface="scada"
              width={instance.width}
              height={instance.height}
              editMode={editMode}
              sensors={sensors}
              values={currentValues}
            />
          )}
        </div>
      </div>
    </Rnd>
  )
}
