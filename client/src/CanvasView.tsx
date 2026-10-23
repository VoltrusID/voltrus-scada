import { useState, useRef, useCallback, useMemo, Fragment } from 'react'
import { Rnd } from 'react-rnd'
import { SensorConfig, CanvasLayoutItem } from './types'
import { SensorCard } from './SensorCard'
import { EditPanel } from './EditPanel'
import { AlignmentToolbar, type AlignOp } from './AlignmentToolbar'

interface Props {
  screenSensors: SensorConfig[]
  canvasLayouts: CanvasLayoutItem[]
  editMode: boolean
  activeScreen: string
  editingId: number | null
  setEditingId: (id: number | null) => void
  setSelectedSensor: (id: number | null) => void
  loadSensorsForScreen: () => void
  onLayoutChange: (items: CanvasLayoutItem[]) => void
  onSave: (s: SensorConfig, shortcut?: string | null) => void
  onDelete: (id: number) => void
  shortcuts: Record<number, string>
  mode: 'live' | 'history' | 'system' | 'flows' | 'tags' | 'assets' | 'scripts'
}

const DEFAULT_W = 300
const DEFAULT_H = 200
const GAP = 20

function generateDefaultPositions(sensors: SensorConfig[]): CanvasLayoutItem[] {
  const cols = Math.max(1, Math.floor(1200 / (DEFAULT_W + GAP)))
  return sensors.map((s, i) => ({
    data_source_id: s.id,
    x: (i % cols) * (DEFAULT_W + GAP),
    y: Math.floor(i / cols) * (DEFAULT_H + GAP),
    width: DEFAULT_W,
    height: DEFAULT_H,
  }))
}

export function CanvasView({
  screenSensors, canvasLayouts, editMode, activeScreen,
  editingId, setEditingId, setSelectedSensor,
  loadSensorsForScreen, onLayoutChange, onSave, onDelete, shortcuts, mode,
}: Props) {
  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set())
  const [zCounter, setZCounter] = useState(100)
  const [zMap, setZMap] = useState<Record<number, number>>({})
  const containerRef = useRef<HTMLDivElement>(null)

  // Merge saved layouts with defaults for new sensors
  const positions = useMemo(() => {
    const defaults = generateDefaultPositions(screenSensors)
    const saved = new Map(canvasLayouts.map(l => [l.data_source_id, l]))
    return defaults.map(d => saved.get(d.data_source_id) ?? d)
  }, [screenSensors, canvasLayouts])

  const updatePosition = useCallback((id: number, patch: Partial<CanvasLayoutItem>) => {
    const updated = positions.map(p =>
      p.data_source_id === id ? { ...p, ...patch } : p
    )
    onLayoutChange(updated)
  }, [positions, onLayoutChange])

  const bringToFront = useCallback((id: number) => {
    setZCounter(z => {
      const next = z + 1
      setZMap(m => ({ ...m, [id]: next }))
      return next
    })
  }, [])

  const handleCardClick = useCallback((id: number, e: React.MouseEvent) => {
    if (!editMode) {
      const s = screenSensors.find(s => s.id === id)
      if (s) setSelectedSensor(s.id)
      return
    }
    if (e.shiftKey) {
      setSelectedIds(prev => {
        const next = new Set(prev)
        if (next.has(id)) next.delete(id)
        else next.add(id)
        return next
      })
    } else {
      setSelectedIds(new Set([id]))
      setEditingId(editingId === id ? null : id)
    }
    bringToFront(id)
  }, [editMode, editingId, setEditingId, setSelectedSensor, screenSensors, bringToFront])

  const handleCanvasClick = useCallback((e: React.MouseEvent) => {
    if (e.target === containerRef.current) {
      setSelectedIds(new Set())
      setEditingId(null)
    }
  }, [setEditingId])

  // Alignment operations
  const handleAlign = useCallback((op: AlignOp) => {
    const selected = positions.filter(p => selectedIds.has(p.data_source_id))
    if (selected.length < 2) return

    const updated = positions.map(p => {
      if (!selectedIds.has(p.data_source_id)) return p
      const s = { ...p }
      switch (op) {
        case 'left': {
          const minX = Math.min(...selected.map(x => x.x))
          s.x = minX; break
        }
        case 'center-h': {
          const centers = selected.map(x => x.x + x.width / 2)
          const avg = centers.reduce((a, b) => a + b, 0) / centers.length
          s.x = avg - s.width / 2; break
        }
        case 'right': {
          const maxRight = Math.max(...selected.map(x => x.x + x.width))
          s.x = maxRight - s.width; break
        }
        case 'top': {
          const minY = Math.min(...selected.map(x => x.y))
          s.y = minY; break
        }
        case 'center-v': {
          const centers = selected.map(x => x.y + x.height / 2)
          const avg = centers.reduce((a, b) => a + b, 0) / centers.length
          s.y = avg - s.height / 2; break
        }
        case 'bottom': {
          const maxBottom = Math.max(...selected.map(x => x.y + x.height))
          s.y = maxBottom - s.height; break
        }
        case 'dist-h': {
          const sorted = [...selected].sort((a, b) => a.x - b.x)
          if (sorted.length < 2) break
          const leftmost = sorted[0].x
          const rightmost = sorted[sorted.length - 1].x + sorted[sorted.length - 1].width
          const totalWidth = sorted.reduce((a, x) => a + x.width, 0)
          const gap = (rightmost - leftmost - totalWidth) / (sorted.length - 1)
          const idx = sorted.indexOf(selected.find(x => x.data_source_id === s.data_source_id)!)
          if (idx > 0) {
            let cx = leftmost
            for (let i = 0; i < idx; i++) cx += sorted[i].width + gap
            s.x = cx
          }
          break
        }
        case 'dist-v': {
          const sorted = [...selected].sort((a, b) => a.y - b.y)
          if (sorted.length < 2) break
          const topmost = sorted[0].y
          const bottommost = sorted[sorted.length - 1].y + sorted[sorted.length - 1].height
          const totalHeight = sorted.reduce((a, x) => a + x.height, 0)
          const gap = (bottommost - topmost - totalHeight) / (sorted.length - 1)
          const idx = sorted.indexOf(selected.find(x => x.data_source_id === s.data_source_id)!)
          if (idx > 0) {
            let cy = topmost
            for (let i = 0; i < idx; i++) cy += sorted[i].height + gap
            s.y = cy
          }
          break
        }
      }
      return s
    })
    onLayoutChange(updated)
  }, [positions, selectedIds, onLayoutChange])

  return (
    <div>
      {editMode && (
        <AlignmentToolbar selectedCount={selectedIds.size} onAlign={handleAlign} />
      )}
      <div
        ref={containerRef}
        style={{
          position: 'relative',
          minHeight: Math.max(600, ...positions.map(p => p.y + p.height + GAP)),
          background: editMode ? 'rgba(0,0,0,.05)' : 'transparent',
          borderRadius: 8,
          cursor: editMode ? 'default' : 'default',
        }}
        onClick={handleCanvasClick}
      >
        {screenSensors.map((s) => {
          const pos = positions.find(p => p.data_source_id === s.id)
          if (!pos) return null
          return (
            <Fragment key={s.id}>
              <Rnd
                position={{ x: pos.x, y: pos.y }}
                size={{ width: pos.width, height: pos.height }}
                onDragStop={editMode ? (_e, d) => {
                  updatePosition(s.id, { x: d.x, y: d.y })
                } : undefined}
                onResizeStop={editMode ? (_e, _dir, ref, _delta, pos) => {
                  updatePosition(s.id, {
                    x: pos.x, y: pos.y,
                    width: parseInt(ref.style.width),
                    height: parseInt(ref.style.height),
                  })
                } : undefined}
                disableDragging={!editMode}
                enableResizing={editMode}
                dragHandleClassName="drag-handle"
                bounds="parent"
                style={{ zIndex: zMap[s.id] ?? 1 }}
                resizeHandleStyles={editMode ? {
                  bottomRight: { width: 8, height: 8, background: 'var(--cyan)', borderRadius: 4, opacity: 0.7 },
                  bottomLeft: { width: 8, height: 8, background: 'var(--cyan)', borderRadius: 4, opacity: 0.7 },
                  topRight: { width: 8, height: 8, background: 'var(--cyan)', borderRadius: 4, opacity: 0.7 },
                  topLeft: { width: 8, height: 8, background: 'var(--cyan)', borderRadius: 4, opacity: 0.7 },
                } : undefined}
              >
                <SensorCard
                  sensor={s}
                  editMode={editMode}
                  activeScreen={activeScreen}
                  editingId={editingId}
                  shortcut={shortcuts[s.id]}
                  loadSensorsForScreen={loadSensorsForScreen}
                  setEditingId={setEditingId}
                  showDragHandle={true}
                  selected={selectedIds.has(s.id)}
                  mode={mode}
                  onClick={e => { e.stopPropagation(); handleCardClick(s.id, e) }}
                />
              </Rnd>
              {editingId === s.id && <EditPanel sensor={s} activeScreen={activeScreen} onSave={onSave} onDelete={onDelete} onClose={() => setEditingId(null)} shortcuts={shortcuts} />}
            </Fragment>
          )
        })}
      </div>
    </div>
  )
}
