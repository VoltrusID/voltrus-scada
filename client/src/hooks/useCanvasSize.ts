import { useRef, useEffect, useState, useCallback } from 'react'

interface CanvasSize {
  width: number
  height: number
  dpr: number
}

export function useCanvasSize(containerRef: React.RefObject<HTMLElement | null>) {
  const [size, setSize] = useState<CanvasSize>({ width: 0, height: 0, dpr: 1 })
  const rafRef = useRef<number>(0)

  const updateSize = useCallback(() => {
    const el = containerRef.current
    if (!el) return
    const rect = el.getBoundingClientRect()
    if (rect.width <= 0 || rect.height <= 0) return
    const dpr = window.devicePixelRatio || 1
    setSize(prev => {
      if (prev.width === rect.width && prev.height === rect.height && prev.dpr === dpr) return prev
      return { width: rect.width, height: rect.height, dpr }
    })
  }, [containerRef])

  useEffect(() => {
    const el = containerRef.current
    if (!el) return

    // Initial measure
    updateSize()

    const ro = new ResizeObserver(() => {
      // Debounce with rAF to avoid layout thrashing
      if (rafRef.current) cancelAnimationFrame(rafRef.current)
      rafRef.current = requestAnimationFrame(updateSize)
    })
    ro.observe(el)

    return () => {
      ro.disconnect()
      if (rafRef.current) cancelAnimationFrame(rafRef.current)
    }
  }, [containerRef, updateSize])

  return size
}

/**
 * Hook that syncs canvas element size to its container without DOM reads in render loop.
 * Returns a ref to assign to the canvas element.
 */
export function useSyncedCanvas() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const size = useCanvasSize(containerRef)

  useEffect(() => {
    const cv = canvasRef.current
    if (!cv || size.width <= 0 || size.height <= 0) return
    cv.width = size.width * size.dpr
    cv.height = size.height * size.dpr
    const ctx = cv.getContext('2d')
    if (ctx) ctx.setTransform(size.dpr, 0, 0, size.dpr, 0, 0)
  }, [size])

  return { canvasRef, containerRef, size }
}
