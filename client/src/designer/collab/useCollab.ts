// The Designer's live document while collaboration is on: the Y.Doc of the
// provider as a model Doc, edits as Yjs transactions, undo per user.
import { useCallback, useEffect, useRef, useState } from 'react'
import type * as Y from 'yjs'
import type { WidgetInstance } from '../../scada2/widgets/types'
import { getWsInstance, onWsReady, type DesignServerMessage, type DesignSessionInfo } from '../../api/websocket'
import type { Doc, Edit } from '../model'
import { connectionsOf, readDoc, widgetsOf } from './ydoc.ts'
import { CollabProvider, type CollabStatus, type DesignSocket, type Peer } from './provider.ts'
import { CollabUndo, editDoc } from './undo.ts'

const EMPTY: Doc = { widgets: [], connections: [] }

export interface CollabHandlers {
  onFlushed?: (m: Extract<DesignServerMessage, { type: 'DesignFlushed' }>) => void
  onRejected?: (m: Extract<DesignServerMessage, { type: 'DesignRejected' }>) => void
  onComment?: (m: Extract<DesignServerMessage, { type: 'DesignComment' }>) => void
  onError?: (e: { code: string; message: string }) => void
  onFallback?: (e: { code: string; message: string }) => void
  /** Dropped from the session and joining again (edits kept). */
  onRejoin?: (e: { code: string; message: string }) => void
  /** Unsaved edits written again into a restarted session. */
  onReplayed?: (e: { objects: number }) => void
}

export interface Collab {
  /** Collaboration is in use for this screen (live, connecting or offline). */
  active: boolean
  status: CollabStatus
  /** The server's document has arrived at least once. */
  synced: boolean
  doc: Doc
  me: { user: string; color: string }
  peers: Peer[]
  sessions: DesignSessionInfo[] | null
  pending: boolean
  /** Local edits not written to the draft yet, read now (for beforeunload). */
  hasUnsaved: () => boolean
  undoDepth: number
  redoDepth: number
  edit: (e: Edit, coalesce?: string, at?: number) => void
  undo: () => void
  redo: () => void
  flush: () => void
  setPresence: (fields: Partial<Omit<Peer, 'clientId' | 'user' | 'color'>>) => void
}

export function useCollab(screen: string, enabled: boolean, nameOf: (w: WidgetInstance) => string, handlers: CollabHandlers = {}, socketOverride?: DesignSocket | null): Collab {
  const [status, setStatus] = useState<CollabStatus>('off')
  const [active, setActive] = useState(false)
  const [synced, setSynced] = useState(false)
  const [doc, setDoc] = useState<Doc>(EMPTY)
  const [peers, setPeers] = useState<Peer[]>([])
  const [me, setMe] = useState({ user: '', color: '#888' })
  const [sessions, setSessions] = useState<DesignSessionInfo[] | null>(null)
  const [pending, setPending] = useState(false)
  const [depth, setDepth] = useState({ undo: 0, redo: 0 })
  const providerRef = useRef<CollabProvider | null>(null)
  const undoRef = useRef<CollabUndo | null>(null)
  const docRef = useRef<Doc>(EMPTY)
  const nameRef = useRef(nameOf)
  nameRef.current = nameOf
  const handlersRef = useRef(handlers)
  handlersRef.current = handlers

  useEffect(() => {
    if (!enabled) { setActive(false); setStatus('off'); return }
    let provider: CollabProvider | null = null
    let unbind: (() => void) | null = null
    let cancelled = false

    const bindDoc = (yd: Y.Doc) => {
      unbind?.()
      undoRef.current?.destroy()
      const undo = new CollabUndo(yd)
      undoRef.current = undo
      const refresh = () => {
        const next = readDoc(yd, docRef.current)
        docRef.current = next
        setDoc(next)
      }
      const onStack = () => setDepth({ undo: undo.um.undoStack.length, redo: undo.um.redoStack.length })
      const ws = widgetsOf(yd), cs = connectionsOf(yd)
      ws.observeDeep(refresh)
      cs.observeDeep(refresh)
      undo.um.on('stack-item-added', onStack)
      undo.um.on('stack-item-popped', onStack)
      undo.um.on('stack-cleared', onStack)
      docRef.current = EMPTY
      refresh()
      onStack()
      unbind = () => { ws.unobserveDeep(refresh); cs.unobserveDeep(refresh) }
    }

    const startWith = (socket: DesignSocket) => {
      if (cancelled || provider) return
      provider = new CollabProvider(socket, screen)
      providerRef.current = provider
      const p = provider
      bindDoc(p.doc)
      p.on('doc', d => bindDoc(d))
      p.on('status', s => { setStatus(s); setMe({ user: p.user, color: p.color }); setPending(p.pending) })
      p.on('synced', () => { setSynced(true); setMe({ user: p.user, color: p.color }) })
      p.on('peers', setPeers)
      p.on('sessions', setSessions)
      p.on('pending', () => setPending(p.pending))
      p.on('flushed', m => { setPending(p.pending); handlersRef.current.onFlushed?.(m) })
      p.on('rejected', m => handlersRef.current.onRejected?.(m))
      p.on('comment', m => handlersRef.current.onComment?.(m))
      p.on('error', e => handlersRef.current.onError?.(e))
      p.on('rejoin', e => handlersRef.current.onRejoin?.(e))
      p.on('replayed', e => handlersRef.current.onReplayed?.(e))
      p.on('fallback', e => { setActive(false); handlersRef.current.onFallback?.(e) })
      setActive(true)
      p.start()
      setStatus(p.status)
    }

    if (socketOverride) startWith(socketOverride)
    else {
      const ws = getWsInstance()
      if (ws) startWith(ws)
      else {
        const off = onWsReady(w => { off(); startWith(w) })
        const prev = unbind
        unbind = () => { off(); (prev as (() => void) | null)?.() }
      }
    }
    return () => {
      cancelled = true
      unbind?.()
      undoRef.current?.destroy()
      undoRef.current = null
      provider?.stop()
      providerRef.current = null
      docRef.current = EMPTY
      setDoc(EMPTY)
      setSynced(false)
      setPeers([])
      setActive(false)
      setStatus('off')
    }
  }, [screen, enabled, socketOverride])

  const edit = useCallback((e: Edit, coalesce?: string, at: number = Date.now()) => {
    const p = providerRef.current, u = undoRef.current
    if (!p || !u) return
    u.before(coalesce, at)
    editDoc(p.doc, docRef.current, e, nameRef.current)
  }, [])
  const undo = useCallback(() => undoRef.current?.undo(), [])
  const redo = useCallback(() => undoRef.current?.redo(), [])
  const flush = useCallback(() => providerRef.current?.flushNow(), [])
  const hasUnsaved = useCallback(() => providerRef.current?.pending ?? false, [])
  const setPresence = useCallback((f: Partial<Omit<Peer, 'clientId' | 'user' | 'color'>>) => providerRef.current?.setPresence(f), [])

  return { active, status, synced, doc, me, peers, sessions, pending, hasUnsaved, undoDepth: depth.undo, redoDepth: depth.redo, edit, undo, redo, flush, setPresence }
}
