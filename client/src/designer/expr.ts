// Binding expressions of the Designer: a small, side-effect-free language
// evaluated in the browser on every live update. No `eval`, no property
// access, no loops: numbers, strings, booleans, tag references, arithmetic,
// comparison, logic, a ternary and a fixed list of functions.
//
//   {wtp/raw_tank_level} > 85 ? "HIGH" : "OK"
//   round({wtp/intake_flow} / 3600, 2)
//   {wtp/p101a_run} && !{wtp/p101b_fault}
//
// A tag is referenced by its project ref, `{device/key}` (the same ref the
// project format uses), so an expression survives export and import. Inside
// the braces a backslash escapes the next character: `{dev/a\}b}` reads the
// tag `dev/a}b` (escapeRef writes refs that way; the server reads them the
// same way when it rewrites refs on a tag rename).
//
// Semantics worth knowing:
// - `==` and `!=` never convert between text and numbers: `"1" == 1` is
//   false. Numbers and booleans compare as numbers (`true == 1`).
// - A missing tag reads as null; arithmetic with null gives null ("no value").
// - Evaluation never throws on runtime values (division by zero, bad
//   decimals...): it gives null. Malformed or oversized source fails to parse.

export type Value = number | string | boolean | null

export type Node =
  | { k: 'lit'; v: Value }
  | { k: 'tag'; ref: string }
  | { k: 'un'; op: '-' | '!'; a: Node }
  | { k: 'bin'; op: string; a: Node; b: Node }
  | { k: 'cond'; c: Node; a: Node; b: Node }
  | { k: 'call'; fn: string; args: Node[] }

export class ExprError extends Error {
  pos: number
  constructor(message: string, pos: number) {
    super(message)
    this.pos = pos
  }
}

type Tok = { t: 'num' | 'str' | 'tag' | 'id' | 'op' | 'end'; v: string; pos: number }

const OPS = ['&&', '||', '==', '!=', '>=', '<=', '?', ':', '(', ')', ',', '+', '-', '*', '/', '%', '>', '<', '!']

function lex(src: string): Tok[] {
  const out: Tok[] = []
  let i = 0
  while (i < src.length) {
    const c = src[i]
    if (/\s/.test(c)) { i++; continue }
    if (/[0-9.]/.test(c)) {
      const m = /^(\d+\.?\d*|\.\d+)([eE][-+]?\d+)?/.exec(src.slice(i))
      if (!m) throw new ExprError(`bad number at ${i + 1}`, i)
      out.push({ t: 'num', v: m[0], pos: i }); i += m[0].length; continue
    }
    if (c === '"' || c === "'") {
      let j = i + 1, s = ''
      while (j < src.length && src[j] !== c) {
        if (src[j] === '\\' && j + 1 < src.length) { s += src[j + 1]; j += 2 } else s += src[j++]
      }
      if (j >= src.length) throw new ExprError('unterminated string', i)
      out.push({ t: 'str', v: s, pos: i }); i = j + 1; continue
    }
    if (c === '{') {
      let j = i + 1, raw = ''
      while (j < src.length && src[j] !== '}') {
        if (src[j] === '\\' && j + 1 < src.length) { raw += src[j + 1]; j += 2 } else raw += src[j++]
      }
      if (j >= src.length) throw new ExprError('missing } after tag reference', i)
      const ref = raw.trim()
      if (!ref) throw new ExprError('empty tag reference', i)
      out.push({ t: 'tag', v: ref, pos: i }); i = j + 1; continue
    }
    if (/[A-Za-z_]/.test(c)) {
      const m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(src.slice(i))!
      out.push({ t: 'id', v: m[0], pos: i }); i += m[0].length; continue
    }
    const op = OPS.find(o => src.startsWith(o, i))
    if (!op) throw new ExprError(`unexpected "${c}" at ${i + 1}`, i)
    out.push({ t: 'op', v: op, pos: i }); i += op.length
  }
  out.push({ t: 'end', v: '', pos: src.length })
  return out
}

export const FUNCTIONS: Record<string, { min: number; max: number; doc: string }> = {
  abs: { min: 1, max: 1, doc: 'abs(x)' },
  round: { min: 1, max: 2, doc: 'round(x, decimals?)' },
  floor: { min: 1, max: 1, doc: 'floor(x)' },
  ceil: { min: 1, max: 1, doc: 'ceil(x)' },
  min: { min: 1, max: 16, doc: 'min(a, b, ...)' },
  max: { min: 1, max: 16, doc: 'max(a, b, ...)' },
  clamp: { min: 3, max: 3, doc: 'clamp(x, lo, hi)' },
  if: { min: 3, max: 3, doc: 'if(cond, then, else)' },
  bit: { min: 2, max: 2, doc: 'bit(x, n): bit n (0-52) of an integer, two\'s complement for negatives' },
  scale: { min: 5, max: 5, doc: 'scale(x, inLo, inHi, outLo, outHi)' },
  text: { min: 1, max: 2, doc: 'text(x, decimals?)' },
}

// Precedence climbing: higher binds tighter.
const PREC: Record<string, number> = { '||': 1, '&&': 2, '==': 3, '!=': 3, '>': 4, '<': 4, '>=': 4, '<=': 4, '+': 5, '-': 5, '*': 6, '/': 6, '%': 6 }

/** Longest expression accepted, in characters. */
export const MAX_SOURCE = 2048
/** Most nodes in one expression. */
export const MAX_NODES = 1000
/** Deepest nesting (also bounds the recursion of refsOf and evaluate). */
export const MAX_DEPTH = 100

/** `{ref}` text for a tag ref, escaping the characters the lexer treats specially. */
export const escapeRef = (ref: string) => `{${ref.replace(/[\\}]/g, m => `\\${m}`)}}`

export function parse(src: string): Node {
  if (src.length > MAX_SOURCE) throw new ExprError(`expression is longer than ${MAX_SOURCE} characters`, MAX_SOURCE)
  const toks = lex(src)
  let p = 0
  let nodes = 0
  let nesting = 0
  // Depth of each node built, so a long left-associative chain is caught too.
  const depths = new WeakMap<Node, number>()
  const mk = <T extends Node>(n: T, pos: number, ...kids: Node[]): T => {
    if (++nodes > MAX_NODES) throw new ExprError(`expression has more than ${MAX_NODES} parts`, pos)
    const d = 1 + kids.reduce((m, k) => Math.max(m, depths.get(k) ?? 1), 0)
    if (d > MAX_DEPTH) throw new ExprError(`expression is nested deeper than ${MAX_DEPTH} levels`, pos)
    depths.set(n, d)
    return n
  }
  const enter = (pos: number) => {
    if (++nesting > MAX_DEPTH) throw new ExprError(`expression is nested deeper than ${MAX_DEPTH} levels`, pos)
  }
  const peek = () => toks[p]
  const take = () => toks[p++]
  const expect = (v: string) => {
    const t = take()
    if (t.t !== 'op' || t.v !== v) throw new ExprError(`expected "${v}" at ${t.pos + 1}`, t.pos)
  }

  const primary = (): Node => {
    enter(peek().pos)
    try { return primaryInner() } finally { nesting-- }
  }

  const primaryInner = (): Node => {
    const t = take()
    switch (t.t) {
      case 'num': return mk({ k: 'lit', v: Number(t.v) }, t.pos)
      case 'str': return mk({ k: 'lit', v: t.v }, t.pos)
      case 'tag': return mk({ k: 'tag', ref: t.v }, t.pos)
      case 'id': {
        if (t.v === 'true' || t.v === 'false') return mk({ k: 'lit', v: t.v === 'true' }, t.pos)
        if (t.v === 'null') return mk({ k: 'lit', v: null }, t.pos)
        // Own names only: `toString`, `constructor`, `__proto__`... are not functions here.
        const fn = Object.prototype.hasOwnProperty.call(FUNCTIONS, t.v) ? FUNCTIONS[t.v] : undefined
        if (!fn) throw new ExprError(`unknown name "${t.v}"`, t.pos)
        expect('(')
        const args: Node[] = []
        if (!(peek().t === 'op' && peek().v === ')')) {
          args.push(ternary())
          while (peek().t === 'op' && peek().v === ',') { take(); args.push(ternary()) }
        }
        expect(')')
        if (args.length < fn.min || args.length > fn.max) throw new ExprError(`${fn.doc} takes ${fn.min === fn.max ? fn.min : `${fn.min}-${fn.max}`} arguments`, t.pos)
        return mk({ k: 'call', fn: t.v, args }, t.pos, ...args)
      }
      case 'op':
        if (t.v === '(') { const e = ternary(); expect(')'); return e }
        if (t.v === '-' || t.v === '!') { const a = primary(); return mk({ k: 'un', op: t.v, a }, t.pos, a) }
        throw new ExprError(`unexpected "${t.v}" at ${t.pos + 1}`, t.pos)
      default:
        throw new ExprError('unexpected end of expression', t.pos)
    }
  }

  const binary = (minPrec: number): Node => {
    let left = primary()
    for (;;) {
      const t = peek()
      const prec = t.t === 'op' ? PREC[t.v] : undefined
      if (prec == null || prec < minPrec) return left
      take()
      const right = binary(prec + 1)
      left = mk({ k: 'bin', op: t.v, a: left, b: right }, t.pos, left, right)
    }
  }

  const ternary = (): Node => {
    enter(peek().pos)
    try {
      const c = binary(1)
      if (peek().t === 'op' && peek().v === '?') {
        const q = take()
        const a = ternary()
        expect(':')
        const b = ternary()
        return mk({ k: 'cond', c, a, b }, q.pos, c, a, b)
      }
      return c
    } finally { nesting-- }
  }

  const root = ternary()
  if (peek().t !== 'end') throw new ExprError(`unexpected "${peek().v}" at ${peek().pos + 1}`, peek().pos)
  return root
}

/** Tag refs an expression reads. Recursion is bounded by MAX_DEPTH (parse rejects deeper trees). */
export function refsOf(n: Node, out: Set<string> = new Set()): Set<string> {
  switch (n.k) {
    case 'tag': out.add(n.ref); break
    case 'un': refsOf(n.a, out); break
    case 'bin': refsOf(n.a, out); refsOf(n.b, out); break
    case 'cond': refsOf(n.c, out); refsOf(n.a, out); refsOf(n.b, out); break
    case 'call': n.args.forEach(a => refsOf(a, out)); break
  }
  return out
}

/** Decimals for text() and round(): an integer from 0 to 20 (what toFixed accepts). */
const decimals = (v: number | undefined) => (v == null || !Number.isFinite(v) ? 0 : Math.max(0, Math.min(20, Math.trunc(v))))

/** Bit n of the integer part of x; negatives as two's complement. null for a bit number outside 0-52. */
function bitOf(x: number, n: number): boolean | null {
  if (!Number.isFinite(x) || !Number.isInteger(n) || n < 0 || n > 52) return null
  const q = Math.floor(Math.trunc(x) / 2 ** n)
  return ((q % 2) + 2) % 2 === 1
}

const num = (v: Value): number => (typeof v === 'number' ? v : typeof v === 'boolean' ? (v ? 1 : 0) : v == null ? NaN : Number(v))
export const truthy = (v: Value): boolean => (typeof v === 'number' ? v !== 0 && !Number.isNaN(v) : !!v)

/**
 * Evaluate. `tag(ref)` returns the tag's current value, or null when the tag is
 * unknown or has no value yet; arithmetic on null gives null, so a missing tag
 * shows as "no value" rather than as 0.
 */
export function evaluate(n: Node, tag: (ref: string) => number | null): Value {
  switch (n.k) {
    case 'lit': return n.v
    case 'tag': return tag(n.ref)
    case 'un': {
      const a = evaluate(n.a, tag)
      if (n.op === '!') return !truthy(a)
      return a == null ? null : -num(a)
    }
    case 'cond': return truthy(evaluate(n.c, tag)) ? evaluate(n.a, tag) : evaluate(n.b, tag)
    case 'bin': {
      if (n.op === '&&') return truthy(evaluate(n.a, tag)) && truthy(evaluate(n.b, tag))
      if (n.op === '||') return truthy(evaluate(n.a, tag)) || truthy(evaluate(n.b, tag))
      const a = evaluate(n.a, tag), b = evaluate(n.b, tag)
      if (n.op === '==') return a === b || (a != null && b != null && typeof a !== 'string' && typeof b !== 'string' && num(a) === num(b))
      if (n.op === '!=') return !(a === b || (a != null && b != null && typeof a !== 'string' && typeof b !== 'string' && num(a) === num(b)))
      if (n.op === '+' && (typeof a === 'string' || typeof b === 'string')) return `${a ?? ''}${b ?? ''}`
      if (a == null || b == null) return null
      const x = num(a), y = num(b)
      switch (n.op) {
        case '+': return x + y
        case '-': return x - y
        case '*': return x * y
        case '/': return y === 0 ? null : x / y
        case '%': return y === 0 ? null : x % y
        case '>': return x > y
        case '<': return x < y
        case '>=': return x >= y
        case '<=': return x <= y
      }
      return null
    }
    case 'call': {
      if (n.fn === 'if') return truthy(evaluate(n.args[0], tag)) ? evaluate(n.args[1], tag) : evaluate(n.args[2], tag)
      const args = n.args.map(a => evaluate(a, tag))
      if (n.fn === 'text') {
        const v = args[0]
        if (v == null) return ''
        if (args.length > 1 && typeof v === 'number' && Number.isFinite(v)) return v.toFixed(decimals(args[1] == null ? 0 : num(args[1])))
        return String(v)
      }
      if (args.some(a => a == null)) return null
      const xs = args.map(num)
      switch (n.fn) {
        case 'abs': return Math.abs(xs[0])
        case 'round': { const f = 10 ** decimals(xs[1]); return Math.round(xs[0] * f) / f }
        case 'floor': return Math.floor(xs[0])
        case 'ceil': return Math.ceil(xs[0])
        case 'min': return Math.min(...xs)
        case 'max': return Math.max(...xs)
        case 'clamp': return Math.min(xs[2], Math.max(xs[1], xs[0]))
        case 'bit': return bitOf(xs[0], xs[1])
        case 'scale': return xs[2] === xs[1] ? null : xs[3] + ((xs[0] - xs[1]) * (xs[4] - xs[3])) / (xs[2] - xs[1])
      }
      return null
    }
  }
}

/** Evaluate without ever throwing: anything unexpected gives null. */
export function safeEvaluate(n: Node, tag: (ref: string) => number | null): Value {
  try {
    const v = evaluate(n, tag)
    return typeof v === 'number' && Number.isNaN(v) ? null : v
  } catch {
    return null
  }
}

export const CACHE_LIMIT = 500
// Least recently used first: a Map keeps insertion order, and a hit re-inserts.
const cache = new Map<string, Node | ExprError>()

/** Parse once per distinct source text (a bounded LRU cache; oversized text is not cached). */
export function compile(src: string): Node | ExprError {
  let c = cache.get(src)
  if (c) {
    cache.delete(src)
    cache.set(src, c)
    return c
  }
  try { c = parse(src) } catch (e) { c = e instanceof ExprError ? e : new ExprError(String(e), 0) }
  if (src.length <= MAX_SOURCE) {
    cache.set(src, c)
    if (cache.size > CACHE_LIMIT) cache.delete(cache.keys().next().value!)
  }
  return c
}

/** Entries in the parse cache (tests). */
export const cacheSize = () => cache.size
