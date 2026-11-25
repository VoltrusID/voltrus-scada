// Writes web-save.json: what the web Designer saves after the edits that
// `designer::tests::the_save_matches_the_web_designer` makes natively. It
// runs the web Designer's own model (client/src/designer/model.ts,
// bindings.ts) and save (client/src/scada2/engine/layout.ts), so the Rust
// test compares against the web code, not a copy of its rules.
//
// From a checkout that has client/src/designer (feat/designer-phase1, or
// main once the web Designer is merged), with Node 23.6 or later:
//
//   node native/tests/fixtures/designer/web-save.mts
//
// Keep the edit list in step with the Rust test.

import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const client = join(here, '../../../../client/src')
const { loadLayoutFromAPI, saveLayoutToAPI } = await import(join(client, 'scada2/engine/layout.ts'))
const { historyReducer, emptyHistory, bindPrimaryPatch } = await import(join(client, 'designer/model.ts'))
const { withBinding } = await import(join(client, 'designer/bindings.ts'))

const input = JSON.parse(readFileSync(join(here, 'wtp-rows.json'), 'utf8'))
const names = new Map<number, string>(input.tags.map((t: any) => [t.id, t.name]))

let h = historyReducer(emptyHistory, { type: 'load', doc: loadLayoutFromAPI(input.equipment, input.connections) })
const w = (id: string) => h.present.widgets.find((x: any) => x.id === id)
const patch = (id: string, p: any, coalesce?: string) => {
  h = historyReducer(h, { type: 'edit', edit: { type: 'patch', patches: { [id]: p } }, coalesce })
}

// 1. Drag P-101A.
patch('e012', { x: 200, y: 180 })
// 2. Resize T-101.
patch('e016', { x: 448, y: 250, width: 120, height: 230 })
// 3. Type a label, one key at a time (one undo step).
for (const label of ['XV-101 ', 'XV-101 I', 'XV-101 Intake']) patch('e011', { config: { label } }, 'e011:label')
// 4. Bind the first KPI to another tag: the stored name goes.
patch('e003', bindPrimaryPatch(40))
// 5. An expression on T-101's label.
patch('e016', { config: withBinding(w('e016').config, 'label', { kind: 'expr', expr: '{wtp/raw_tank_level} > 85 ? "HIGH" : "T-101 " + text({wtp/raw_tank_level}, 1) + " %"' }) })
// 6. P-101B shows only while it runs; undone and redone.
patch('e013', { config: withBinding(w('e013').config, 'visible', { kind: 'tag', tag: 'wtp/p101b_run' }) })
h = historyReducer(h, { type: 'undo' })
h = historyReducer(h, { type: 'redo' })
// 7. A state map on the trend's colour.
patch('e033', { config: withBinding(w('e033').config, 'lineColor', { kind: 'map', expr: '{wtp/raw_turbidity}', cases: [{ op: '>=', v: 32, out: '#f04646' }, { op: '>=', v: 25, out: '#f5b21b' }], else: 'var(--cyan)' }) })
// 8. A move that is undone: not saved.
patch('e001', { x: 400, y: 400 })
h = historyReducer(h, { type: 'undo' })

const saved = saveLayoutToAPI(h.present, (x: any) => names.get(x.sensorId) || x.type)
writeFileSync(join(here, 'web-save.json'), JSON.stringify({ undo_depth: h.past.length, ...saved }, null, 1) + '\n')
console.log(`web-save.json: ${saved.equipment.length} rows, ${saved.connections.length} connections, ${h.past.length} undo steps`)
