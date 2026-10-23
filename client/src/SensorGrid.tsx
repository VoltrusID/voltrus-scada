import { useRef, useCallback } from 'react'
import { ResponsiveGridLayout, useContainerWidth, verticalCompactor } from 'react-grid-layout'
import { SensorConfig, ScreenLayouts } from './types'
import { EditPanel } from './EditPanel'
import { SensorCard } from './SensorCard'

const BREAKPOINTS = { lg: 1200, md: 768, sm: 480, xs: 0 }
const COLS = { lg: 24, md: 12, sm: 4, xs: 2 }
const BP_WIDTHS: Record<string, number> = { lg: 1200, md: 996, sm: 480, xs: 360 }

interface Props {
  screenLayouts: ScreenLayouts
  screenSensors: SensorConfig[]
  editMode: boolean
  activeScreen: string
  gap: number
  editingId: number | null
  setEditingId: (id: number | null) => void
  setSelectedSensor?: (id: number | null) => void
  loadSensorsForScreen: () => void
  onLayoutChange: (currentLayout: any[], allLayouts: { [bp: string]: any[] }) => void
  onSave: (s: SensorConfig, shortcut?: string | null) => void
  onDelete: (id: number) => void
  forcedBreakpoint: string | null
  shortcuts: Record<number, string>
  mode: 'live' | 'history' | 'system' | 'flows' | 'tags' | 'assets' | 'scripts'
}

export function SensorGrid({ screenLayouts, screenSensors, editMode, activeScreen, gap, editingId, setEditingId, setSelectedSensor, loadSensorsForScreen, onLayoutChange, onSave, onDelete, forcedBreakpoint, shortcuts, mode }: Props) {
  const { width: measuredWidth, containerRef, mounted } = useContainerWidth({ initialWidth: 1200 })
  const didDrag = useRef(false)

  const width = forcedBreakpoint ? BP_WIDTHS[forcedBreakpoint] : measuredWidth

  const handleInteractionEnd = useCallback((layout: any) => {
    didDrag.current = true
    const activeBp = forcedBreakpoint || 'lg'
    const normalize = (items: readonly any[]) => items.map((l: any) => ({ i: String(l.i), x: l.x, y: l.y, w: l.w, h: l.h }))
    const allLayouts: Record<string, any[]> = {
      lg: activeBp === 'lg' ? normalize(layout) : screenLayouts.lg,
      md: activeBp === 'md' ? normalize(layout) : screenLayouts.md,
      sm: activeBp === 'sm' ? normalize(layout) : screenLayouts.sm,
      xs: activeBp === 'xs' ? normalize(layout) : screenLayouts.xs,
    }
    onLayoutChange(layout, allLayouts)
  }, [onLayoutChange, forcedBreakpoint, screenLayouts])

  const ready = screenSensors.length > 0 && (screenLayouts.lg.length > 0 || screenLayouts.md.length > 0 || screenLayouts.sm.length > 0)

  // Stable key that only changes when sensor set changes (not on every data tick)
  const stableKey = screenSensors.map(s => s.id).sort((a, b) => a - b).join(',')

  return (
    <div ref={containerRef} style={{ maxWidth: '100%' }}>
      {mounted && ready && (
        <ResponsiveGridLayout
          key={stableKey}
          width={width}
          layouts={screenLayouts as any}
          breakpoints={BREAKPOINTS}
          breakpoint={forcedBreakpoint || undefined}
          cols={COLS}
          rowHeight={50}
          margin={[gap, gap] as [number, number]}
          compactor={verticalCompactor}
          onDragStop={editMode ? (handleInteractionEnd as any) : undefined}
          onResizeStop={editMode ? (handleInteractionEnd as any) : undefined}
          dragConfig={editMode ? { handle: '.drag-handle' } : { enabled: false }}
          resizeConfig={editMode ? { enabled: true } : { enabled: false }}
        >
          {screenSensors.map((s) => (
            <div key={String(s.id)} style={{ position: 'relative' }}>
              <SensorCard
                sensor={s}
                editMode={editMode}
                activeScreen={activeScreen}
                editingId={editingId}
                shortcut={shortcuts[s.id]}
                loadSensorsForScreen={loadSensorsForScreen}
                setEditingId={setEditingId}
                showDragHandle={true}
                mode={mode}
                onClick={e => {
                  if (didDrag.current) { didDrag.current = false; return; }
                  if (editMode) { e.stopPropagation(); setEditingId(editingId === s.id ? null : s.id); }
                  else { setSelectedSensor?.(s.id); }
                }}
              />
              {editingId === s.id && <EditPanel sensor={s} activeScreen={activeScreen} onSave={onSave} onDelete={onDelete} onClose={() => setEditingId(null)} shortcuts={shortcuts} />}
            </div>
          ))}
        </ResponsiveGridLayout>
      )}
    </div>
  );
}
