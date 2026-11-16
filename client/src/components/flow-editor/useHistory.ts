import { useCallback, useRef, useState } from 'react'

const LIMIT = 100
const COALESCE_MS = 800

export interface UpdateOpts {
  /** false: change the present without an undo step (selection, drag moves). */
  record?: boolean
  /** Consecutive updates with the same key within a short time share one undo step (typing). */
  key?: string
}

/** Undo/redo over immutable snapshots. */
export function useHistory<T>(initial: T | (() => T)) {
  const [state, setState] = useState(() => ({
    past: [] as T[],
    present: typeof initial === 'function' ? (initial as () => T)() : initial,
    future: [] as T[],
  }))
  const last = useRef<{ key?: string; t: number }>({ t: 0 })

  const update = useCallback((fn: (p: T) => T, opts: UpdateOpts = {}) => {
    const record = opts.record !== false
    let coalesce = false
    if (record) {
      const now = Date.now()
      coalesce = !!opts.key && last.current.key === opts.key && now - last.current.t < COALESCE_MS
      last.current = { key: opts.key, t: now }
    }
    setState(s => {
      const next = fn(s.present)
      if (next === s.present) return s
      if (!record) return { ...s, present: next }
      if (coalesce) return { ...s, present: next, future: [] }
      return { past: [...s.past.slice(-(LIMIT - 1)), s.present], present: next, future: [] }
    })
  }, [])

  /** Push the present as an undo step (before a drag starts). */
  const checkpoint = useCallback(() => {
    last.current = { t: 0 }
    setState(s => ({ past: [...s.past.slice(-(LIMIT - 1)), s.present], present: s.present, future: [] }))
  }, [])

  const undo = useCallback(() => {
    last.current = { t: 0 }
    setState(s => s.past.length
      ? { past: s.past.slice(0, -1), present: s.past[s.past.length - 1], future: [s.present, ...s.future] }
      : s)
  }, [])

  const redo = useCallback(() => {
    last.current = { t: 0 }
    setState(s => s.future.length
      ? { past: [...s.past, s.present], present: s.future[0], future: s.future.slice(1) }
      : s)
  }, [])

  return {
    present: state.present,
    update,
    checkpoint,
    undo,
    redo,
    canUndo: state.past.length > 0,
    canRedo: state.future.length > 0,
  }
}
