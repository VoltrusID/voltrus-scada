import { useEffect, useState } from 'react'

/**
 * Whether the element is on screen (or within `margin` of it). Live charts
 * use it to skip redrawing canvases nobody can see: a dashboard with a
 * thousand tags otherwise repaints every card on every update.
 */
export function useInView(ref: React.RefObject<HTMLElement | null>, margin = '200px'): boolean {
  const [inView, setInView] = useState(true)
  useEffect(() => {
    const el = ref.current
    if (!el || typeof IntersectionObserver === 'undefined') return
    const io = new IntersectionObserver(entries => {
      const last = entries[entries.length - 1]
      if (last) setInView(last.isIntersecting)
    }, { rootMargin: margin })
    io.observe(el)
    return () => io.disconnect()
  }, [ref, margin])
  return inView
}
