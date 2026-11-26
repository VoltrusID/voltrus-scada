// Undo per user in a shared document, and model edits as Yjs transactions.
import * as Y from 'yjs'
import type { WidgetInstance } from '../../scada2/widgets/types'
import { applyEdit, COALESCE_MS, type Doc, type Edit } from '../model.ts'
import { connectionsOf, widgetsOf, writeDiff } from './ydoc.ts'
import { LOCAL } from './provider.ts'

/** Undo per user over a provider's document, with phase 1's coalescing. */
export class CollabUndo {
  um: Y.UndoManager
  private lastKey: string | null = null
  private lastAt: number | null = null
  readonly doc: Y.Doc
  constructor(doc: Y.Doc) {
    this.doc = doc
    this.um = new Y.UndoManager([widgetsOf(doc), connectionsOf(doc)], { trackedOrigins: new Set([LOCAL]), captureTimeout: COALESCE_MS })
  }
  /** Call before a local edit: a new key, no key or a pause starts a new step. */
  before(coalesce: string | undefined, at: number) {
    const paused = this.lastAt != null && at - this.lastAt > COALESCE_MS
    if (!coalesce || coalesce !== this.lastKey || paused) this.um.stopCapturing()
    this.lastKey = coalesce ?? null
    this.lastAt = at
  }
  undo() { this.lastKey = null; this.um.undo() }
  redo() { this.lastKey = null; this.um.redo() }
  destroy() { this.um.destroy() }
}

/** Apply a model edit to the document as one local transaction. */
export function editDoc(doc: Y.Doc, current: Doc, e: Edit, nameOf: (w: WidgetInstance) => string): Doc {
  const next = applyEdit(current, e)
  if (next !== current) writeDiff(doc, current, next, nameOf, LOCAL)
  return next
}

