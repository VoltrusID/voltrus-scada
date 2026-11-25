import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  blameLine, changedFields, diffHunks, diffOutlines, diffStats, diffSummary, groupBatches, initials, kindSummary, lineDiff,
  nextVersionName, publishState, relativeTime, revertObjectRequest, revertScreenRequest, splitLines, stateLabel, withAlsoRestore,
  type DiffLine,
} from '../src/designer/historyModel.ts'

const change = (seq: number, batch: string, object_id: string, kind: 'add' | 'update' | 'remove' = 'update', extra: Record<string, any> = {}) => ({
  seq, batch, screen_id: 's1', object_type: 'widget', object_id, kind, before: null, after: null,
  username: 'ana', source: 'designer', operation: 'save', note: '', at_ms: 1000 + seq, squashed: 0, ...extra,
})

/** Apply a diff to the old lines: must give the new lines. */
function apply(lines: DiffLine[]): { a: string[]; b: string[] } {
  return { a: lines.filter(l => l.op !== 'add').map(l => l.text), b: lines.filter(l => l.op !== 'del').map(l => l.text) }
}

// ── Line diff ──

test('lineDiff: identical texts are all unchanged, with line numbers on both sides', () => {
  const d = lineDiff(['a', 'b'], ['a', 'b'])
  assert.deepEqual(d, [{ op: 'same', text: 'a', a: 1, b: 1 }, { op: 'same', text: 'b', a: 2, b: 2 }])
})

test('lineDiff: one changed line is a deletion then an addition', () => {
  const d = lineDiff(['x: 1', 'y: 2', 'z: 3'], ['x: 1', 'y: 5', 'z: 3'])
  assert.deepEqual(d.map(l => `${l.op}:${l.text}`), ['same:x: 1', 'del:y: 2', 'add:y: 5', 'same:z: 3'])
  assert.deepEqual(d.find(l => l.op === 'del'), { op: 'del', text: 'y: 2', a: 2 })
  assert.deepEqual(d.find(l => l.op === 'add'), { op: 'add', text: 'y: 5', b: 2 })
})

test('lineDiff: insertions and removals in the middle keep the rest unchanged (shortest edit)', () => {
  const a = ['- id: a', '  x: 1', '- id: b', '  x: 2', '- id: c', '  x: 3']
  const b = ['- id: a', '  x: 1', '- id: c', '  x: 3', '- id: d', '  x: 4']
  const d = lineDiff(a, b)
  assert.deepEqual(apply(d), { a, b })
  assert.deepEqual(diffStats(d), { added: 2, removed: 2 })
})

test('lineDiff: empty sides', () => {
  assert.deepEqual(lineDiff([], ['a']), [{ op: 'add', text: 'a', b: 1 }])
  assert.deepEqual(lineDiff(['a'], []), [{ op: 'del', text: 'a', a: 1 }])
  assert.deepEqual(lineDiff([], []), [])
})

test('lineDiff: random edits always round-trip and are minimal against an LCS', () => {
  let seed = 7
  const rnd = (n: number) => { seed = (seed * 1103515245 + 12345) % 2147483648; return seed % n }
  const lcs = (a: string[], b: string[]) => {
    const t = Array.from({ length: a.length + 1 }, () => new Array(b.length + 1).fill(0))
    for (let i = a.length - 1; i >= 0; i--) for (let j = b.length - 1; j >= 0; j--) t[i][j] = a[i] === b[j] ? t[i + 1][j + 1] + 1 : Math.max(t[i + 1][j], t[i][j + 1])
    return t[0][0]
  }
  for (let run = 0; run < 200; run++) {
    const a = Array.from({ length: rnd(14) }, () => 'abcde'[rnd(5)])
    const b = Array.from({ length: rnd(14) }, () => 'abcde'[rnd(5)])
    const d = lineDiff(a, b)
    assert.deepEqual(apply(d), { a, b }, `run ${run}`)
    const same = d.filter(l => l.op === 'same').length
    assert.equal(same, lcs(a, b), `minimal, run ${run}`)
  }
})

test('diffHunks keeps changes with context and counts the lines skipped', () => {
  const a = Array.from({ length: 20 }, (_, i) => `l${i}`)
  const b = [...a]
  b[10] = 'changed'
  const h = diffHunks(lineDiff(a, b), 2)
  assert.equal(h.length, 1)
  assert.equal(h[0].skippedBefore, 8)
  assert.deepEqual(h[0].lines.map(l => l.text), ['l8', 'l9', 'l10', 'changed', 'l11', 'l12'])
  assert.deepEqual(diffHunks(lineDiff(a, a)), [])
})

test('splitLines drops the trailing newline only', () => {
  assert.deepEqual(splitLines('a\nb\n'), ['a', 'b'])
  assert.deepEqual(splitLines(''), [])
  assert.deepEqual(splitLines(undefined), [])
})

// ── Batches ──

test('groupBatches groups rows of one save, newest batch first, rows by seq', () => {
  const rows = [change(9, 'B2', 'b', 'add'), change(8, 'B2', 'a'), change(5, 'B1', 'a', 'add')]
  const bs = groupBatches(rows)
  assert.deepEqual(bs.map(b => [b.batch, b.minSeq, b.maxSeq, b.changes.map(c => c.seq)]), [['B2', 8, 9, [8, 9]], ['B1', 5, 5, [5]]])
  assert.equal(bs[0].username, 'ana')
})

test('groupBatches merges a batch split across pages and ignores repeated rows', () => {
  const page1 = [change(12, 'B3', 'a'), change(11, 'B3', 'b')]
  const page2 = [change(11, 'B3', 'b'), change(10, 'B3', 'c'), change(4, 'B1', 'a', 'add', { note: 'first' })]
  const bs = groupBatches([...page1, ...page2])
  assert.equal(bs.length, 2)
  assert.deepEqual(bs[0].changes.map(c => c.seq), [10, 11, 12])
  assert.equal(bs[1].note, 'first')
})

test('kindSummary counts by kind', () => {
  assert.equal(kindSummary(['add', 'add', 'update', 'remove']), '2 added · 1 changed · 1 removed')
  assert.equal(kindSummary([]), 'no changes')
})

// ── Labels ──

test('relativeTime reads like a person', () => {
  const now = Date.UTC(2026, 10, 25, 12)
  assert.equal(relativeTime(now - 10_000, now), 'just now')
  assert.equal(relativeTime(now - 5 * 60_000, now), '5 min ago')
  assert.equal(relativeTime(now - 3 * 3600_000, now), '3 h ago')
  assert.equal(relativeTime(now - 30 * 3600_000, now), 'yesterday')
  assert.equal(relativeTime(now - 4 * 86400_000, now), '4 days ago')
  assert.match(relativeTime(now - 40 * 86400_000, now), /^\d{1,2} [A-Z][a-z]{2} 2026$/)
})

test('initials for avatars', () => {
  assert.equal(initials('Ana Lim'), 'AL')
  assert.equal(initials('admin'), 'AD')
  assert.equal(initials('api_key:ci-bot'), 'CB')
  assert.equal(initials('j.doe'), 'JD')
})

test('changedFields names fields inside the JSON-text columns like the server', () => {
  const before = { x: 10, y: 20, config: JSON.stringify({ label: 'P-101', z: 1 }), sensor_bindings: '[1]' }
  const after = { x: 15, y: 20, config: JSON.stringify({ label: 'P-102', z: 1, locked: true }), sensor_bindings: '[2]' }
  assert.deepEqual(changedFields(before, after), ['config.label', 'config.locked', 'sensor_bindings', 'x'])
  assert.deepEqual(changedFields({ config: '{"a":1}' }, { config: '{ "a": 1 }' }), [])
  assert.deepEqual(changedFields(null, after), [])
})

const status = (o: Record<string, any> = {}) => ({
  screen_id: 's1', seq: 10, revision: 'r', publishing: true, approval: 'none' as const, published: null,
  changes_since_published: 0, last_version: null, changes_since_version: 0, pending: [], ...o,
})
const pub = (o: Record<string, any> = {}) => ({ version_id: 3, name: 'v12', upto_seq: 7, at_ms: 1, by: 'ana', draft_differs: false, ...o })

test('publishState: the title bar line for every case', () => {
  assert.equal(publishState(undefined), null)
  assert.equal(publishState(status({ publishing: false })), null)
  assert.equal(publishState(status())!.text, 'Not published (operators see the draft)')
  assert.equal(publishState(status({ published: pub() }))!.text, 'Published v12')
  assert.equal(publishState(status({ published: pub({ draft_differs: true }), changes_since_published: 3 }))!.text, 'Draft · 3 changes since v12')
  assert.equal(publishState(status({ published: pub({ draft_differs: true }), changes_since_published: 1 }))!.text, 'Draft · 1 change since v12')
  const pending = status({ published: pub(), pending: [{ name: 'v13', requested_by: 'bob', username: 'bob' }] })
  assert.equal(publishState(pending as any)!.text, 'Publish pending approval')
})

test('nextVersionName follows the v<n> names', () => {
  assert.equal(nextVersionName([]), 'v1')
  assert.equal(nextVersionName([{ name: 'v3' }, { name: 'before shutdown' }, { name: 'v1' }]), 'v4')
  assert.equal(nextVersionName([{ name: 'a' }, { name: 'b' }]), 'v3')
})

test('blameLine', () => {
  const now = Date.UTC(2026, 10, 25, 12)
  const line = blameLine({
    object_type: 'widget', object_id: 'w1', exists: true, seq: 9, batch: 'B', kind: 'update', username: 'ana', source: 'designer',
    operation: 'save', note: '', at_ms: now - 5 * 60_000, changes: 4, created_by: 'bob', created_at_ms: Date.UTC(2026, 9, 3, 12),
  }, now)
  assert.equal(line, 'Last changed by ana · 5 min ago · via Designer (4 changes) · added by bob on 3 Oct 2026')
})

test('stateLabel and diffSummary', () => {
  const vs = [{ id: 4, name: 'v-smoke' }]
  assert.equal(stateLabel('draft'), 'Draft')
  assert.equal(stateLabel('published'), 'Published')
  assert.equal(stateLabel('seq:12'), 'After change #12')
  assert.equal(stateLabel('4', vs), 'v-smoke')
  assert.equal(diffSummary({ added: 1, removed: 0, changed: 2 }), '1 added · 2 changed')
  assert.equal(diffSummary({ added: 0, removed: 0, changed: 0 }), 'No differences')
})

// ── Revert requests ──

test('revert requests: one object, the whole screen', () => {
  assert.deepEqual(revertObjectRequest('w1', 'widget', 12, 30), { object: 'w1', to_seq: 12, object_type: 'widget', base_seq: 30 })
  assert.deepEqual(revertObjectRequest('w1', 'screen', 12), { object: 'w1', to_seq: 12 })
  assert.deepEqual(revertScreenRequest(12), { to: 'seq:12' })
  assert.deepEqual(revertScreenRequest('5', 30), { to: '5', base_seq: 30 })
})

test('withAlsoRestore: the connection and its missing widgets in one batch, at the same state', () => {
  const body = revertObjectRequest('c1', 'connection', 12, 30)
  assert.deepEqual(withAlsoRestore(body, ['P-101', 'T-1']), {
    objects: [
      { object_id: 'c1', object_type: 'connection', to_seq: 12 },
      { object_id: 'P-101', object_type: 'widget', to_seq: 12 },
      { object_id: 'T-1', object_type: 'widget', to_seq: 12 },
    ],
    base_seq: 30,
  })
  // Already a multi-object request: adds only what is missing, once.
  const again = withAlsoRestore({ objects: [{ object_id: 'c1', to: 'v2' }, { object_id: 'P-101', to: 'v2' }] }, ['P-101', 'T-1'])
  assert.deepEqual(again.objects, [{ object_id: 'c1', to: 'v2' }, { object_id: 'P-101', to: 'v2' }, { object_id: 'T-1', object_type: 'widget', to: 'v2' }])
})

test('diffOutlines: old side removed/changed, new side added/changed', () => {
  const row = (id: string) => ({ equipment_id: id }) as any
  const o = diffOutlines({
    widgets: { added: [row('n')], removed: [row('r')], changed: [{ id: 'c', fields: ['x'], before: row('c'), after: row('c') }] },
    connections: { added: [], removed: [], changed: [] },
    summary: { added: 1, removed: 1, changed: 1 },
  })
  assert.deepEqual(o, { from: { r: 'removed', c: 'changed' }, to: { n: 'added', c: 'changed' } })
})
