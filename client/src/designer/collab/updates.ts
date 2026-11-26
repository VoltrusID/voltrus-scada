// Splitting a Yjs update (v1) that is over the server's size limit into
// several, each under it. Applied in order they give the same document: the
// structs keep their order per client and the delete set goes last.

import * as Y from 'yjs'
import * as encoding from 'lib0/encoding'

type Struct = ReturnType<typeof Y.decodeUpdate>['structs'][number]
interface Run { client: number; structs: Struct[] }
interface Dels { client: number; items: { clock: number; len: number }[] }

/** Bytes a client header (count, client, clock) can take, at most. */
const RUN_HEAD = 15
/** Bytes the section counts of an update can take, at most. */
const UPDATE_HEAD = 10

function sizeOf(s: Struct): number {
  const e = new Y.UpdateEncoderV1()
  s.write(e, 0)
  return e.toUint8Array().length
}

function encode(runs: Run[], dels: Dels[]): Uint8Array {
  const e = new Y.UpdateEncoderV1()
  encoding.writeVarUint(e.restEncoder, runs.length)
  for (const r of runs) {
    encoding.writeVarUint(e.restEncoder, r.structs.length)
    e.writeClient(r.client)
    encoding.writeVarUint(e.restEncoder, r.structs[0].id.clock)
    for (const s of r.structs) s.write(e, 0)
  }
  encoding.writeVarUint(e.restEncoder, dels.length)
  for (const d of dels) {
    e.resetDsCurVal()
    encoding.writeVarUint(e.restEncoder, d.client)
    encoding.writeVarUint(e.restEncoder, d.items.length)
    for (const it of d.items) { e.writeDsClock(it.clock); e.writeDsLen(it.len) }
  }
  return e.toUint8Array()
}

/**
 * `update` as updates of at most `max` bytes each, in the order to apply
 * them; null when one struct alone is over `max` (it cannot be sent).
 */
export function splitUpdate(update: Uint8Array, max: number): Uint8Array[] | null {
  if (update.length <= max) return [update]
  const { structs, ds } = Y.decodeUpdate(update)
  const out: Uint8Array[] = []
  let runs: Run[] = [], dels: Dels[] = [], size = UPDATE_HEAD
  const close = () => {
    if (runs.length || dels.length) out.push(encode(runs, dels))
    runs = []; dels = []; size = UPDATE_HEAD
  }
  for (const s of structs) {
    if (s instanceof Y.Skip) continue
    const n = sizeOf(s)
    if (n + RUN_HEAD + UPDATE_HEAD > max) return null
    let last = runs[runs.length - 1]
    const prev = last?.structs[last.structs.length - 1]
    const joins = !!prev && last.client === s.id.client && prev.id.clock + prev.length === s.id.clock
    if (size + n + (joins ? 0 : RUN_HEAD) > max) { close(); last = undefined as unknown as Run }
    if (last && joins) last.structs.push(s)
    else { runs.push({ client: s.id.client, structs: [s] }); size += RUN_HEAD }
    size += n
  }
  for (const [client, items] of ds.clients) {
    for (const it of items) {
      // client header + two varUints of the range
      let d = dels[dels.length - 1]
      const need = (d?.client === client ? 0 : RUN_HEAD) + 10
      if (size + need > max) { close(); d = undefined as unknown as Dels }
      if (!d || d.client !== client) { d = { client, items: [] }; dels.push(d); size += RUN_HEAD }
      d.items.push({ clock: it.clock, len: it.len })
      size += 10
    }
  }
  close()
  return out
}
