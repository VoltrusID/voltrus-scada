import { useRef, useMemo, useEffect, useState } from 'react'
import { FlowNode } from './FlowNode'
import { NODE_WIDTH, NODE_GAP, NODE_HEIGHT, NODE_META } from './constants'
import type { FlowNodeConfig } from '../../types'

interface Props {
  trigger: any
  nodes: FlowNodeConfig[]
  selectedIndex: number | null
  onSelectTrigger: () => void
  onSelectNode: (index: number) => void
  onRemoveNode: (index: number) => void
  onDragNodeStart: (index: number) => void
  onDrop: (insertIndex: number) => void
  isDragOver: boolean
}

export function CanvasArea({
  trigger, nodes, selectedIndex,
  onSelectTrigger, onSelectNode, onRemoveNode, onDragNodeStart, onDrop, isDragOver,
}: Props) {
  const canvasRef = useRef<HTMLDivElement>(null)
  const [canvasSize, setCanvasSize] = useState({ width: 800, height: 400 })

  useEffect(() => {
    const el = canvasRef.current
    if (!el) return
    const ro = new ResizeObserver(entries => {
      const cr = entries[0].contentRect
      setCanvasSize({ width: cr.width, height: cr.height })
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  // Split nodes into process and output groups
  const processNodes = useMemo(() => nodes.filter(n => NODE_META[n.type]?.category === 'process'), [nodes])
  const outputNodes = useMemo(() => nodes.filter(n => NODE_META[n.type]?.category === 'output'), [nodes])

  // Build index maps (original index in nodes array)
  const processIndices = useMemo(() => {
    const idx: number[] = []
    nodes.forEach((n, i) => { if (NODE_META[n.type]?.category === 'process') idx.push(i) })
    return idx
  }, [nodes])

  const outputIndices = useMemo(() => {
    const idx: number[] = []
    nodes.forEach((n, i) => { if (NODE_META[n.type]?.category === 'output') idx.push(i) })
    return idx
  }, [nodes])

  // Layout: trigger → process chain → outputs stacked vertically
  const { positions, nodeMap } = useMemo(() => {
    const map = new Map<string, { x: number; y: number; nodeIndex: number; isTrigger: boolean }>()
    const OUTPUT_GAP = 16

    // Total width of process chain + trigger
    const processChainWidth = (1 + processNodes.length) * NODE_WIDTH + (processNodes.length) * NODE_GAP
    // Check if output stack would be taller
    const outputStackHeight = outputNodes.length * NODE_HEIGHT + (outputNodes.length - 1) * OUTPUT_GAP

    // Center vertically based on the tallest section
    const centerY = canvasSize.height / 2 - NODE_HEIGHT / 2

    // Trigger position
    let x = Math.max(60, (canvasSize.width - (processChainWidth + NODE_GAP + NODE_WIDTH)) / 2)
    const triggerY = centerY

    map.set('trigger', { x, y: triggerY, nodeIndex: -1, isTrigger: true })

    // Process nodes in a horizontal chain
    x += NODE_WIDTH + NODE_GAP
    for (let i = 0; i < processNodes.length; i++) {
      map.set(`process-${i}`, { x, y: centerY, nodeIndex: processIndices[i], isTrigger: false })
      x += NODE_WIDTH + NODE_GAP
    }

    // Output nodes stacked vertically
    const outputStartY = centerY - (outputStackHeight - NODE_HEIGHT) / 2
    for (let i = 0; i < outputNodes.length; i++) {
      map.set(`output-${i}`, { x, y: outputStartY + i * (NODE_HEIGHT + OUTPUT_GAP), nodeIndex: outputIndices[i], isTrigger: false })
    }

    return { positions: map, nodeMap: map }
  }, [processNodes.length, outputNodes.length, processIndices, outputIndices, canvasSize.width, canvasSize.height])

  // Build edges
  const edges = useMemo(() => {
    const result: { from: { x: number; y: number }; to: { x: number; y: number } }[] = []

    const triggerPos = nodeMap.get('trigger')
    const firstProcess = nodeMap.get('process-0')
    const firstOutput = nodeMap.get('output-0')

    // Trigger → first process
    if (triggerPos && firstProcess) {
      result.push({
        from: { x: triggerPos.x + NODE_WIDTH, y: triggerPos.y + NODE_HEIGHT / 2 },
        to: { x: firstProcess.x, y: firstProcess.y + NODE_HEIGHT / 2 },
      })
    }

    // Process chain
    for (let i = 1; i < processNodes.length; i++) {
      const prev = nodeMap.get(`process-${i - 1}`)
      const curr = nodeMap.get(`process-${i}`)
      if (prev && curr) {
        result.push({
          from: { x: prev.x + NODE_WIDTH, y: prev.y + NODE_HEIGHT / 2 },
          to: { x: curr.x, y: curr.y + NODE_HEIGHT / 2 },
        })
      }
    }

    // Last process → outputs (or trigger → outputs if no process nodes)
    const source = processNodes.length > 0
      ? nodeMap.get(`process-${processNodes.length - 1}`)
      : triggerPos

    if (source) {
      for (let i = 0; i < outputNodes.length; i++) {
        const out = nodeMap.get(`output-${i}`)
        if (out) {
          result.push({
            from: { x: source.x + NODE_WIDTH, y: source.y + NODE_HEIGHT / 2 },
            to: { x: out.x, y: out.y + NODE_HEIGHT / 2 },
          })
        }
      }
    }

    return result
  }, [nodeMap, processNodes.length, outputNodes.length])

  const handleMouseUp = (e: React.MouseEvent) => {
    if (!isDragOver) return
    const rect = canvasRef.current?.getBoundingClientRect()
    if (!rect) return
    const x = e.clientX - rect.left
    const y = e.clientY - rect.top
    const totalItems = 1 + nodes.length
    let bestIndex = totalItems
    let bestDist = Infinity

    // Build gap positions based on the layout
    for (let i = 0; i <= totalItems; i++) {
      let key: string | null = null
      if (i === 0) {
        key = 'trigger'
      } else {
        const ni = i - 1
        const node = nodes[ni]
        const cat = NODE_META[node?.type]?.category
        if (cat === 'process') {
          const pi = processIndices.indexOf(ni)
          if (pi >= 0) key = `process-${pi}`
        } else {
          const oi = outputIndices.indexOf(ni)
          if (oi >= 0) key = `output-${oi}`
        }
      }

      if (!key) continue
      const pos = nodeMap.get(key)
      if (!pos) continue

      const gapX = i === 0
        ? pos.x - NODE_GAP / 2
        : pos.x + NODE_WIDTH + NODE_GAP / 2
      // 2D distance so the vertical output stack and cross-category drops
      // resolve correctly (X alone can't distinguish stacked outputs).
      const gapY = pos.y + NODE_HEIGHT / 2

      const dist = Math.hypot(x - gapX, y - gapY)
      if (dist < bestDist) {
        bestDist = dist
        bestIndex = i
      }
    }
    onDrop(bestIndex)
  }

  // Render all items from the nodeMap
  const items = useMemo(() => {
    const result: { key: string; x: number; y: number; isTrigger: boolean; nodeIndex: number }[] = []

    // Trigger
    const tp = nodeMap.get('trigger')
    if (tp) result.push({ key: 'trigger', ...tp })

    // Process nodes
    for (let i = 0; i < processNodes.length; i++) {
      const p = nodeMap.get(`process-${i}`)
      if (p) result.push({ key: `process-${i}`, ...p })
    }

    // Output nodes
    for (let i = 0; i < outputNodes.length; i++) {
      const p = nodeMap.get(`output-${i}`)
      if (p) result.push({ key: `output-${i}`, ...p })
    }

    return result
  }, [nodeMap, processNodes.length, outputNodes.length])

  return (
    <div
      ref={canvasRef}
      style={{
        flex: 1, position: 'relative', overflow: 'hidden',
        background: 'var(--bg)',
      }}
      onMouseUp={handleMouseUp}
    >
      {/* Dot grid background */}
      <svg style={{ position: 'absolute', inset: 0, width: '100%', height: '100%', pointerEvents: 'none' }}>
        <defs>
          <pattern id="dotGrid" width="20" height="20" patternUnits="userSpaceOnUse">
            <circle cx="1" cy="1" r="1" fill="var(--border)" opacity="0.15" />
          </pattern>
        </defs>
        <rect width="100%" height="100%" fill="url(#dotGrid)" />
      </svg>

      {/* Drop zone indicator */}
      {isDragOver && (
        <div style={{
          position: 'absolute', inset: 0,
          border: '2px dashed var(--green)', background: 'rgba(34,197,94,.05)',
          pointerEvents: 'none', zIndex: 5,
        }} />
      )}

      {/* SVG Connections */}
      <svg style={{ position: 'absolute', inset: 0, width: '100%', height: '100%', pointerEvents: 'none', zIndex: 1 }}>
        <defs>
          <marker id="arrowhead" markerWidth="8" markerHeight="6" refX="7" refY="3" orient="auto">
            <polygon points="0 0, 8 3, 0 6" fill="#4b5563" />
          </marker>
        </defs>
        {edges.map((edge, i) => {
          const mx = (edge.from.x + edge.to.x) / 2
          const d = `M ${edge.from.x} ${edge.from.y} C ${mx} ${edge.from.y}, ${mx} ${edge.to.y}, ${edge.to.x} ${edge.to.y}`
          return <path key={i} d={d} fill="none" stroke="#4b5563" strokeWidth={2} markerEnd="url(#arrowhead)" />
        })}
      </svg>

      {/* Nodes */}
      {items.map(item => (
        <div
          key={item.key}
          style={{ position: 'absolute', left: item.x, top: item.y, zIndex: 2 }}
        >
          {item.isTrigger ? (
            <FlowNode
              isTrigger={true}
              triggerType={trigger?.type}
              isSelected={selectedIndex === null}
              index={0}
              onSelect={onSelectTrigger}
            />
          ) : (
            <FlowNode
              node={nodes[item.nodeIndex]}
              isTrigger={false}
              isSelected={selectedIndex === item.nodeIndex}
              index={item.nodeIndex}
              onSelect={() => onSelectNode(item.nodeIndex)}
              onRemove={() => onRemoveNode(item.nodeIndex)}
              onDragStart={onDragNodeStart}
            />
          )}
        </div>
      ))}
    </div>
  )
}
