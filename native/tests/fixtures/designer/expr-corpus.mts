// Writes expr-corpus.json: what the web Designer's expression language
// (client/src/designer/expr.ts) and bindings (bindings.ts) give for a list
// of expressions, bindings and numbers, so `designer::tests` checks the
// native port against the web code itself. CI runs this and fails when the
// file it writes differs from the one committed (native.yml).
//
// From the repository root, with Node 23.6 or later:
//
//   node native/tests/fixtures/designer/expr-corpus.mts
//
// Deterministic: the random numbers come from a fixed seed. Values are
// written as text: `number:<String(x)>` (`-0` for negative zero),
// `string:…`, `boolean:…`, `null`, `undefined`, `other:<JSON>`, and parse
// errors as `error:<message>@<position in UTF-16 units>`.

import { writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const client = join(here, '../../../../client/src')
const E = await import(join(client, 'designer/expr.ts'))
const B = await import(join(client, 'designer/bindings.ts'))

// The tag values the Rust test reads too.
const tags: Record<string, number> = {
  'd/a': 0, 'd/b': 2.5, 'd/big': 4503599627370497, 'd/neg': -5, 'd/x': 0.49999999999999994, 'd/h': 1e300, 'dev/a}b': 7,
}
const read = (r: string) => (r in tags ? tags[r] : null)
const enc = (v: unknown): string => {
  if (v === undefined) return 'undefined'
  if (v === null) return 'null'
  if (typeof v === 'number') return 'number:' + (Object.is(v, -0) ? '-0' : String(v))
  if (typeof v === 'string') return 'string:' + v
  if (typeof v === 'boolean') return 'boolean:' + v
  return 'other:' + JSON.stringify(v)
}

// Expressions: parse errors and their positions, values and JavaScript's
// conversions. (A character outside the Basic Multilingual Plane outside a
// string is left out: the web's message names half of it, a lone surrogate.)
const exprs = [
  // Numbers and negative zero.
  'text(-{d/a}, 2)', '-{d/a}', '-0', '0 * -1', 'text(-0)', 'text(-0, 1)', 'text(-{d/a})', '"" + -{d/a}',
  'text(-0.0001, 2)', 'text(-0.004, 2)', 'text(-0.005, 2)', 'text(1e-7, 2)', 'text(0.000001)', 'text(1e-7)',
  // Math.round.
  'round({d/x})', 'round({d/big})', 'round(0.49999999999999994)', 'round(-0.4)', 'round(-0.5)', 'round(-0.6)', 'round(2.5)', 'round(-2.5)',
  'round(1.005, 2)', 'round(1.255, 2)', 'round(4503599627370497)', 'round(-4503599627370497)', 'round(9007199254740993)', 'round(1e300)', 'round(-0)',
  // toFixed: exact ties away from zero, near ties by the exact value.
  'text(0.125, 2)', 'text(1.005, 2)', 'text(2.5, 0)', 'text(-2.5, 0)', 'text(1e21, 2)', 'text(123.456, 20)', 'text(1.45, 1)', 'text(8.345, 2)',
  'text(2940.055, 2)', 'text(0.5, 0)', 'text(1.5, 0)', 'text(-1.5, 0)', 'text(9.995, 2)', 'text(99.5, 0)', 'text(0.0625, 3)', 'text(-0.0625, 3)',
  'text(1e20, 2)', 'text(1/3, 25)', 'text(1/3, "2")', 'text(1/3, null)', 'text(1/3, -1)', 'text(1/3, 2.9)', 'text(1/3, true)',
  // Function names: own ones only.
  'toString(1)', 'valueOf()', 'constructor(1)', '__proto__(1)', 'hasOwnProperty(1)', 'isPrototypeOf(1,2)', 'abs(-3)',
  // Number(): hex, octal, binary, Infinity, white space, no `inf`.
  '"0x10" * 1', '"0X1f" * 1', '"0o17" * 1', '"0b11" * 1', '"-0x10" * 1', '"0x" * 1', '"0x1g" * 1', '"0b12" * 1',
  '"inf" * 1', '"Infinity" * 1', '"-Infinity" * 1', '"+Infinity" * 1', '"infinity" * 1', '"nan" * 1', '"NaN" * 1',
  '" 12 " * 1', '"1_0" * 1', '"+5" * 1', '"5." * 1', '".5" * 1', '"." * 1', '"1e5" * 1', '"1e" * 1', '"1e+5" * 1', '"-.5e-1" * 1',
  '" 12" * 1', '"﻿12" * 1', '"\u008512" * 1', '"  12 　" * 1', '"᠎12" * 1', '"\t\n12\r" * 1', '"" * 1', '"   " * 1',
  '"12abc" * 1', '"1,5" * 1', '"0012" * 1', '"1e400" * 1', '"-0" * 1',
  // Number literals.
  '1.e5', '1.', '.5e-3', '1e', '1.5.3', '1e+', '00.5', '5e-324', '1e309',
  // Number to text.
  '"a" + 1e21', '"a" + 1e-7', '"a" + 0.1 + 0.2', '"" + (0.1 + 0.2)', '"" + 1/3', '"" + 123456789012345680000', '"" + 1e300*1e10',
  '"" + -1e-7', '"" + 5e-324', '"" + 1.7976931348623157e308', '"" + 0.000001', '"" + 0.0000015', '"" + 1e20', '"" + 2**1',
  // Equality and truthiness.
  'true == 1', '"1" == 1', 'null == null', 'null == 0', 'true + true', '"" == 0', 'false == 0', '1/0 == null', '"a" != "a"', 'true != 2',
  '!""', '!"0"', '!0', '!null', '!(0/0)',
  // bit().
  'bit(-1, 52)', 'bit(5, 0)', 'bit(5, 2.5)', 'bit(5, 53)', 'bit(4503599627370497, 0)', 'bit(-2.5, 0)', 'bit({d/big}, 0)', 'bit(5, -1)', 'bit(1/0, 0)',
  // min, max, clamp, scale and signed zeros.
  '"" + min(0, -0)', 'min(0, -0)', 'max(-0, 0)', 'min(-0, 0)', 'max(0, -0)', 'min("a", 1)', 'clamp(5, 10, 0)', 'clamp(-0, 0, 1)',
  'scale(5, 0, 10, 0, 100)', 'scale(5, 1, 1, 0, 1)', 'min(1)', 'max(3, "7", true)',
  // Arithmetic and no value.
  '1 / 0', '1 % 0', '-5 % 3', '5.5 % -2', '-{d/missing}', '{d/missing} + "x"', '!{d/missing}', '{d/missing} == null', '1e400 * 0',
  '{d/h} * 1e10', '-"abc"', '-""', '{d/neg} * {d/b}', '{ dev/a\\}b } + 1', '{ d/b } * 2',
  // Parse errors and positions (UTF-16 units).
  '\u0085 1', '﻿1', '"😀" +', '"😀" 1', '"é😀é" #', '{😀} ?', '1 2', '(1', 'foo(1)', 'abs()', 'abs(1,2)', '1 ? 2', '"abc', '{abc', '{ }', '#',
  'min()', 'clamp(1, 2)', 'if(1, 2)', ')', '1 +', '(',
  // Precedence.
  '1 + 2 * 3 - 4 / 2 % 3', '1 < 2 == 2 > 1', '!1 == 0', '-2 * -3', '1 || 0 && 0', 'true ? 1 : 0 ? 2 : 3', '1 - - 1', '2 > 1 > 0', '"b" > "a"',
  'if(1, "a", 1/0)', 'text(true)', 'text(null)', 'text("x", 2)',
]
const exprOut = exprs.map(src => {
  const c = E.compile(src)
  if (c instanceof E.ExprError) return { src, r: 'error:' + c.message + '@' + c.pos }
  return { src, r: enc(E.safeEvaluate(c, read)) }
})

// Bindings: an expression's value as each property type takes it.
const types = ['string', 'number', 'boolean', null]
const bexprs = [
  '{d/h} * 1e10', '-{d/h} * 1e10', '0.0005', '-0.0005', '0.0015', '2.0005', '1.0005', '{d/x}', '"abc"', '"12"', '""', '"0x1f"', '1 > 0',
  '{d/a}', '-{d/a}', '1.0004999999999999', '"Infinity"', '"  "', 'false', '1/0', '{d/missing}',
]
const bindOut: any[] = []
for (const src of bexprs) for (const t of types) bindOut.push({ b: { kind: 'expr', expr: src }, type: t, r: enc(B.evaluateBinding({ kind: 'expr', expr: src }, read, t ?? undefined)) })
for (const tag of ['d/b', 'd/h', 'd/missing', 'd/x']) for (const t of types) bindOut.push({ b: { kind: 'tag', tag }, type: t, r: enc(B.evaluateBinding({ kind: 'tag', tag }, read, t ?? undefined)) })

// Map bindings, malformed ones too.
const maps: any[] = [
  { kind: 'map', expr: '1', cases: 'nope', else: 'E' },
  { kind: 'map', expr: '1', cases: [{ v: 1, out: 'A' }], else: 'E' },
  { kind: 'map', expr: '1', cases: [{ v: 2, out: 'A' }], else: 'E' },
  { kind: 'map', expr: '1', cases: [null, 5, { op: '==', v: 1, out: 'A' }], else: 'E' },
  { kind: 'map', expr: '1', cases: [{ op: '==', v: '1', out: 'A' }], else: 'E' },
  { kind: 'map', expr: '1', cases: [{ op: '==', v: 1, out: [1, 2] }], else: 'E' },
  { kind: 'map', expr: '1', cases: [{ op: '==', v: 1, out: { a: 1 } }], else: 'E' },
  { kind: 'map', expr: '1', cases: [{ op: '==', v: 1, out: [5] }], else: 'E' },
  { kind: 'map', expr: '1', cases: [{ op: '==', v: 1, out: [] }], else: 'E' },
  { kind: 'map', expr: '1', cases: [{ op: '==', v: 1, out: [null, 'a', [2, 3]] }], else: 'E' },
  { kind: 'map', expr: '1', cases: [{ op: '==', v: 1, out: null }], else: 'E' },
  { kind: 'map', expr: '1', cases: [{ op: '==', v: 1 }], else: 'E' },
  { kind: 'map', expr: '1', cases: [{ op: '==', v: 1, out: 0.0005 }], else: 'E' },
  { kind: 'map', expr: '1', cases: [{ op: '==', v: 1, out: '0x10' }], else: 'E' },
  { kind: 'map', expr: '1', cases: [], else: true },
  { kind: 'map', expr: '1', cases: [] },
  { kind: 'map', expr: '"a"', cases: [{ op: '<', v: 5, out: 'A' }], else: 'E' },
  { kind: 'map', expr: '1 > 0', cases: [{ op: '>=', v: 1, out: 'ON' }], else: 'OFF' },
  { kind: 'map', expr: '{d/b}', cases: [{ op: '>', v: 3, out: 'H' }, { op: '>', v: 2, out: 'M' }], else: 'L' },
  { kind: 'map', expr: '{d/missing}', cases: [{ op: '<', v: 5, out: 'A' }], else: 'E' },
  { kind: 'map', expr: '1 +', cases: [], else: 'E' },
]
for (const m of maps) for (const t of types) bindOut.push({ b: m, type: t, r: enc(B.evaluateBinding(m, read, t ?? undefined)) })

// Numbers: toFixed, Math.round and String() of random values, of exact
// ties and of the doubles either side of them.
let seed = 12345
const rnd = () => { seed = (seed * 1103515245 + 12345) % 2147483648; return seed / 2147483648 }
const f64 = new Float64Array(1)
const i64 = new BigInt64Array(f64.buffer)
const step = (x: number, by: bigint) => { f64[0] = x; i64[0] += by; return f64[0] }
const numbers: [string, number, string, string][] = []
const add = (x: number, d: number) => {
  if (!Number.isFinite(x)) return
  numbers.push([Object.is(x, -0) ? '-0' : String(x), d, x.toFixed(d), enc(Math.round(x))])
}
for (let i = 0; i < 2500; i++) {
  const d = Math.floor(rnd() * 5)
  const k = Math.floor(rnd() * 2000000) - 1000000
  const r = rnd()
  let x: number
  if (r < 0.3) x = (k + 0.5) / 10 ** d
  else if (r < 0.6) x = Number(`${k}.${String(Math.floor(rnd() * 10 ** d)).padStart(d, '0')}5`)
  else if (r < 0.8) x = k / 10 ** (d + Math.floor(rnd() * 3))
  else x = (rnd() - 0.5) * 10 ** Math.floor(rnd() * 30 - 10)
  add(x, d)
  if (r < 0.6) { add(step(x, 1n), d); add(step(x, -1n), d) }
}
for (const x of [1.45, 2940.055, 8.345, 1.005, 0.125, 2.5, -2.5, 0.5, 1.5, 9.995, 0.0625, 1e-7, 5e-324, 0.49999999999999994, 4503599627370497, -0, 0]) {
  for (const d of [0, 1, 2, 3, 20]) add(x, d)
}

writeFileSync(join(here, 'expr-corpus.json'), JSON.stringify({ tags, exprs: exprOut, bindings: bindOut, numbers }, null, 0)
  .replace(/\},\{/g, '},\n{').replace(/\],\[/g, '],\n[') + '\n')
console.log(`expr-corpus.json: ${exprOut.length} expressions, ${bindOut.length} bindings, ${numbers.length} numbers`)
