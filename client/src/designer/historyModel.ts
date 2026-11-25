// Version history in the Designer, without React: the change log grouped into
// batches, labels (relative time, sources, the title bar's draft/published
// line), changed fields of a change, the revert requests the History pane
// sends (with "also restore" for a dangling connection), the outlines of a
// compare, and a line diff for the YAML of two states. Tested directly by
// client/tests/historyModel.test.ts.

import type {
  BlameEntry, ChangeKind, DesignChange, DesignStatus, DesignVersion, RevertBody, ScreenDiff,
} from '../queries/designHistory'

// ── Batches ──

/** One save (or revert, import…) as the History pane lists it. */
export interface Batch {
  batch: string
  username: string
  source: string
  operation: string
  note: string
  at_ms: number
  /** Highest seq of the batch: "the state after this batch" is `seq:<maxSeq>`. */
  maxSeq: number
  minSeq: number
  changes: DesignChange[]
}

/**
 * Group change rows (newest first, possibly several pages) by batch. A batch
 * split across two pages comes out whole; order is by the batch's newest row.
 * Rows of a batch are listed by seq.
 */
export function groupBatches(changes: DesignChange[]): Batch[] {
  const out: Batch[] = []
  const index = new Map<string, Batch>()
  const seen = new Set<number>()
  for (const c of changes) {
    if (seen.has(c.seq)) continue
    seen.add(c.seq)
    let b = index.get(c.batch)
    if (!b) {
      b = { batch: c.batch, username: c.username, source: c.source, operation: c.operation, note: c.note, at_ms: c.at_ms, maxSeq: c.seq, minSeq: c.seq, changes: [] }
      index.set(c.batch, b)
      out.push(b)
    }
    b.changes.push(c)
    b.maxSeq = Math.max(b.maxSeq, c.seq)
    b.minSeq = Math.min(b.minSeq, c.seq)
    b.at_ms = Math.max(b.at_ms, c.at_ms)
    if (!b.note && c.note) b.note = c.note
  }
  for (const b of out) b.changes.sort((x, y) => x.seq - y.seq)
  return out.sort((a, b) => b.maxSeq - a.maxSeq)
}

/** "2 added · 1 changed" for a batch or a list of change kinds. */
export function kindSummary(kinds: ChangeKind[]): string {
  const n = { add: 0, update: 0, remove: 0 }
  for (const k of kinds) n[k] = (n[k] ?? 0) + 1
  const parts: string[] = []
  if (n.add) parts.push(`${n.add} added`)
  if (n.update) parts.push(`${n.update} changed`)
  if (n.remove) parts.push(`${n.remove} removed`)
  return parts.join(' · ') || 'no changes'
}

// ── Labels ──

const MIN = 60_000
const HOUR = 60 * MIN
const DAY = 24 * HOUR

/** "just now", "5 min ago", "3 h ago", "yesterday", "4 days ago", then the date. */
export function relativeTime(atMs: number, now = Date.now()): string {
  const d = now - atMs
  if (d < 45_000) return 'just now'
  if (d < HOUR) return `${Math.max(1, Math.round(d / MIN))} min ago`
  if (d < DAY) return `${Math.round(d / HOUR)} h ago`
  if (d < 2 * DAY) return 'yesterday'
  if (d < 7 * DAY) return `${Math.floor(d / DAY)} days ago`
  return shortDate(atMs)
}

export function shortDate(atMs: number): string {
  const t = new Date(atMs)
  const m = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'][t.getMonth()]
  return `${t.getDate()} ${m} ${t.getFullYear()}`
}

/** Two letters for an avatar: "Ana Lim" → "AL", "admin" → "AD", "api_key:ci" → "CI". */
export function initials(username: string): string {
  const name = username.replace(/^api_key:/, '').trim()
  const words = name.split(/[\s._-]+/).filter(Boolean)
  if (words.length >= 2) return (words[0][0] + words[1][0]).toUpperCase()
  return (name.slice(0, 2) || '?').toUpperCase()
}

const SOURCES: Record<string, string> = {
  user: 'Web', designer: 'Designer', native_designer: 'Native Designer', editor: '2D editor', api_key: 'API key',
  mcp: 'MCP', copilot: 'Copilot', flow: 'Flow', system: 'System',
}

/** Where a change came from, for its badge. */
export function sourceLabel(source: string): string {
  return SOURCES[source] ?? source
}

const OPERATIONS: Record<string, string> = {
  save: 'Saved', import: 'Project import', equipment: 'Equipment change', revert: 'Revert', rollback: 'Rollback',
  tag_rename: 'Tag rename', wizard: 'Wizard', faceplate: 'Faceplate',
}

export function operationLabel(op: string): string {
  return OPERATIONS[op] ?? op
}

/** A short colour per user, stable across renders. */
export function avatarHue(username: string): number {
  let h = 0
  for (const ch of username) h = (h * 31 + ch.charCodeAt(0)) % 360
  return h
}

/** The name a row shows for an object: its label, equipment name, or id. */
export function objectLabel(row: Record<string, any> | null | undefined, id: string): string {
  if (!row) return id
  const cfg = parseMaybe(row.config)
  const label = (cfg && typeof cfg === 'object' && (cfg.label || cfg.equipment || cfg.text)) || row.equipment_name
  return label ? `${label}` : id
}

function parseMaybe(v: unknown): any {
  if (typeof v !== 'string') return v
  try { return JSON.parse(v) } catch { return v }
}

const NESTED = new Set(['config', 'sensor_bindings', 'waypoints'])

/**
 * Field names that differ between two rows, as the server's diff names them:
 * `x`, `width`, `config.label` (inside config and the other JSON-text columns).
 */
export function changedFields(before: Record<string, any> | null | undefined, after: Record<string, any> | null | undefined): string[] {
  if (!before || !after) return []
  const keys = [...new Set([...Object.keys(before), ...Object.keys(after)])].sort()
  const out: string[] = []
  for (const k of keys) {
    const a = before[k], b = after[k]
    if (JSON.stringify(a) === JSON.stringify(b)) continue
    if (NESTED.has(k)) {
      const pa = parseMaybe(a), pb = parseMaybe(b)
      if (JSON.stringify(pa) === JSON.stringify(pb)) continue
      if (pa && pb && typeof pa === 'object' && typeof pb === 'object' && !Array.isArray(pa) && !Array.isArray(pb)) {
        for (const ik of [...new Set([...Object.keys(pa), ...Object.keys(pb)])].sort()) {
          if (JSON.stringify(pa[ik]) !== JSON.stringify(pb[ik])) out.push(`${k}.${ik}`)
        }
        continue
      }
    }
    out.push(k)
  }
  return out
}

/** The title bar's line about drafts and publishing. `tone` colours it. */
export function publishState(s: DesignStatus | undefined): { text: string; tone: 'ok' | 'warn' | 'muted' | 'info'; title: string } | null {
  if (!s || !s.publishing) return null
  if (s.pending.length) {
    const v = s.pending[0]
    return { text: 'Publish pending approval', tone: 'info', title: `${v.name} requested by ${v.requested_by ?? v.username}; an administrator must approve it` }
  }
  if (!s.published) {
    return { text: 'Not published (operators see the draft)', tone: 'muted', title: 'This screen was never published: operators see the draft as it is saved' }
  }
  const name = s.published.name ?? `#${s.published.version_id}`
  const n = s.changes_since_published
  if (n > 0 || s.published.draft_differs) {
    const count = n > 0 ? `${n} change${n === 1 ? '' : 's'}` : 'changes'
    return { text: `Draft · ${count} since ${name}`, tone: 'warn', title: `Operators see ${name}; the draft has ${count} not yet published` }
  }
  return { text: `Published ${name}`, tone: 'ok', title: `Operators see ${name}, the same as the draft` }
}

/** The name the server would give the next version (`v<n>`), for a placeholder. */
export function nextVersionName(versions: Pick<DesignVersion, 'name'>[]): string {
  let max = 0
  for (const v of versions) {
    const m = /^v(\d+)$/i.exec(v.name)
    if (m) max = Math.max(max, Number(m[1]))
  }
  return `v${Math.max(max, versions.length) + 1}`
}

/** "Last changed by X · 5 min ago · via Designer (n changes) · added by Y on date" */
export function blameLine(b: BlameEntry, now = Date.now()): string {
  const parts = [`Last changed by ${b.username}`, relativeTime(b.at_ms, now), `via ${sourceLabel(b.source)} (${b.changes} change${b.changes === 1 ? '' : 's'})`]
  if (b.created_by) parts.push(`added by ${b.created_by}${b.created_at_ms ? ` on ${shortDate(b.created_at_ms)}` : ''}`)
  return parts.join(' · ')
}

// ── Revert requests ──

type ObjType = 'widget' | 'connection'

const objType = (t: string | null | undefined): ObjType | undefined => (t === 'widget' || t === 'connection' ? t : undefined)

/** Put one object back as it was right after change `toSeq`. */
export function revertObjectRequest(objectId: string, objectType: string | null | undefined, toSeq: number, baseSeq?: number | null): RevertBody {
  const body: RevertBody = { object: objectId, to_seq: toSeq }
  const t = objType(objectType)
  if (t) body.object_type = t
  if (baseSeq != null) body.base_seq = baseSeq
  return body
}

/** Put the whole screen back as it was after change `seq`, or at a version / `published`. */
export function revertScreenRequest(to: number | string, baseSeq?: number | null): RevertBody {
  const body: RevertBody = { to: typeof to === 'number' ? `seq:${to}` : to }
  if (baseSeq != null) body.base_seq = baseSeq
  return body
}

/**
 * The same revert plus the widgets a restored connection ends on (the
 * server's 409 `dangling_connection` names them), all in one batch at the
 * same state.
 */
export function withAlsoRestore(body: RevertBody, missing: string[]): RevertBody {
  const objects = [...(body.objects ?? [])]
  let at: { to_seq?: number; to?: string } = {}
  if (body.object) {
    at = body.to_seq != null ? { to_seq: body.to_seq } : body.to != null ? { to: body.to } : {}
    objects.push({ object_id: body.object, ...(body.object_type ? { object_type: body.object_type } : {}), ...at })
  } else if (objects.length) {
    const o = objects[0]
    at = o.to_seq != null ? { to_seq: o.to_seq } : o.to != null ? { to: o.to } : {}
  }
  const have = new Set(objects.map(o => o.object_id))
  for (const id of missing) {
    if (!have.has(id)) objects.push({ object_id: id, object_type: 'widget', ...at })
  }
  const out: RevertBody = { objects }
  if (body.base_seq != null) out.base_seq = body.base_seq
  if (body.base_revision != null) out.base_revision = body.base_revision
  if (body.message) out.message = body.message
  return out
}

// ── Compare ──

export type Outline = 'added' | 'removed' | 'changed'

export const OUTLINE_COLORS: Record<Outline, string> = { added: '#22c55e', removed: '#ef4444', changed: '#f59e0b' }

/** Widget outlines of a compare: the old side shows removed and changed, the new side added and changed. */
export function diffOutlines(d: ScreenDiff['diff']): { from: Record<string, Outline>; to: Record<string, Outline> } {
  const from: Record<string, Outline> = {}
  const to: Record<string, Outline> = {}
  for (const r of d.widgets.removed) from[r.equipment_id] = 'removed'
  for (const r of d.widgets.added) to[r.equipment_id] = 'added'
  for (const c of d.widgets.changed) { from[c.id] = 'changed'; to[c.id] = 'changed' }
  return { from, to }
}

/** Plain-words summary of a diff: "2 added · 1 removed · 3 changed", or "No differences". */
export function diffSummary(s: { added: number; removed: number; changed: number } | undefined): string {
  if (!s) return ''
  const parts = [s.added && `${s.added} added`, s.removed && `${s.removed} removed`, s.changed && `${s.changed} changed`].filter(Boolean)
  return parts.length ? parts.join(' · ') : 'No differences'
}

/** A state spec for the server (`draft`, `published`, a version id) as a person reads it. */
export function stateLabel(spec: string, versions: Pick<DesignVersion, 'id' | 'name'>[] = []): string {
  if (spec === 'draft') return 'Draft'
  if (spec === 'published') return 'Published'
  if (spec.startsWith('seq:')) return `After change #${spec.slice(4)}`
  const v = versions.find(x => String(x.id) === spec || x.name === spec)
  return v ? v.name : spec
}

// ── Line diff ──

export type DiffOp = 'same' | 'add' | 'del'
export interface DiffLine { op: DiffOp; text: string; /** 1-based line in the old text */ a?: number; /** 1-based line in the new text */ b?: number }

/**
 * Line diff of two texts (Myers' O(ND) shortest edit script), common prefix
 * and suffix trimmed first. Deletions come before additions in a changed run.
 */
export function lineDiff(oldLines: string[], newLines: string[]): DiffLine[] {
  let pre = 0
  while (pre < oldLines.length && pre < newLines.length && oldLines[pre] === newLines[pre]) pre++
  let suf = 0
  while (suf < oldLines.length - pre && suf < newLines.length - pre &&
    oldLines[oldLines.length - 1 - suf] === newLines[newLines.length - 1 - suf]) suf++
  const a = oldLines.slice(pre, oldLines.length - suf)
  const b = newLines.slice(pre, newLines.length - suf)
  const mid = myers(a, b)
  const out: DiffLine[] = []
  for (let i = 0; i < pre; i++) out.push({ op: 'same', text: oldLines[i], a: i + 1, b: i + 1 })
  for (const m of mid) {
    if (m.op === 'same') out.push({ op: 'same', text: a[m.i!], a: pre + m.i! + 1, b: pre + m.j! + 1 })
    else if (m.op === 'del') out.push({ op: 'del', text: a[m.i!], a: pre + m.i! + 1 })
    else out.push({ op: 'add', text: b[m.j!], b: pre + m.j! + 1 })
  }
  for (let k = 0; k < suf; k++) {
    const i = oldLines.length - suf + k, j = newLines.length - suf + k
    out.push({ op: 'same', text: oldLines[i], a: i + 1, b: j + 1 })
  }
  return out
}

function myers(a: string[], b: string[]): { op: DiffOp; i?: number; j?: number }[] {
  const n = a.length, m = b.length
  if (!n) return b.map((_, j) => ({ op: 'add' as const, j }))
  if (!m) return a.map((_, i) => ({ op: 'del' as const, i }))
  const max = n + m
  const off = max
  let v = new Int32Array(2 * max + 2)
  const trace: Int32Array[] = []
  let found = false
  for (let d = 0; d <= max && !found; d++) {
    trace.push(v.slice())
    const next = v.slice()
    for (let k = -d; k <= d; k += 2) {
      let x = (k === -d || (k !== d && v[off + k - 1] < v[off + k + 1])) ? v[off + k + 1] : v[off + k - 1] + 1
      let y = x - k
      while (x < n && y < m && a[x] === b[y]) { x++; y++ }
      next[off + k] = x
      if (x >= n && y >= m) { found = true; break }
    }
    v = next
  }
  // Walk back through the trace from (n, m).
  const ops: { op: DiffOp; i?: number; j?: number }[] = []
  let x = n, y = m
  for (let d = trace.length - 1; d > 0; d--) {
    const vd = trace[d]
    const k = x - y
    const prevK = (k === -d || (k !== d && vd[off + k - 1] < vd[off + k + 1])) ? k + 1 : k - 1
    const px = vd[off + prevK]
    const py = px - prevK
    while (x > px && y > py) { x--; y--; ops.push({ op: 'same', i: x, j: y }) }
    if (x === px) { y--; ops.push({ op: 'add', j: y }) } else { x--; ops.push({ op: 'del', i: x }) }
  }
  while (x > 0 && y > 0) { x--; y--; ops.push({ op: 'same', i: x, j: y }) }
  ops.reverse()
  // Within each run of edits, deletions first.
  const out: typeof ops = []
  let run: typeof ops = []
  const flush = () => { out.push(...run.filter(o => o.op === 'del'), ...run.filter(o => o.op === 'add')); run = [] }
  for (const o of ops) {
    if (o.op === 'same') { flush(); out.push(o) } else run.push(o)
  }
  flush()
  return out
}

/** A run of diff lines to show: changes with `context` unchanged lines around them. */
export interface Hunk { lines: DiffLine[]; skippedBefore: number }

export function diffHunks(lines: DiffLine[], context = 3): Hunk[] {
  const keep = new Array(lines.length).fill(false)
  lines.forEach((l, i) => {
    if (l.op === 'same') return
    for (let k = Math.max(0, i - context); k <= Math.min(lines.length - 1, i + context); k++) keep[k] = true
  })
  const hunks: Hunk[] = []
  let cur: Hunk | null = null
  let skipped = 0
  lines.forEach((l, i) => {
    if (!keep[i]) { skipped++; cur = null; return }
    if (!cur) { cur = { lines: [], skippedBefore: skipped }; hunks.push(cur); skipped = 0 }
    cur.lines.push(l)
  })
  return hunks
}

/** Lines added and removed. */
export function diffStats(lines: DiffLine[]): { added: number; removed: number } {
  let added = 0, removed = 0
  for (const l of lines) { if (l.op === 'add') added++; else if (l.op === 'del') removed++ }
  return { added, removed }
}

export const splitLines = (text: string | undefined | null): string[] => {
  if (!text) return []
  const lines = text.split('\n')
  if (lines[lines.length - 1] === '') lines.pop()
  return lines
}
