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
// project format uses), so an expression survives export and import.

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
      const j = src.indexOf('}', i)
      if (j < 0) throw new ExprError('missing } after tag reference', i)
      const ref = src.slice(i + 1, j).trim()
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
  bit: { min: 2, max: 2, doc: 'bit(x, n): bit n of an integer' },
  scale: { min: 5, max: 5, doc: 'scale(x, inLo, inHi, outLo, outHi)' },
  text: { min: 1, max: 2, doc: 'text(x, decimals?)' },
}

// Precedence climbing: higher binds tighter.
const PREC: Record<string, number> = { '||': 1, '&&': 2, '==': 3, '!=': 3, '>': 4, '<': 4, '>=': 4, '<=': 4, '+': 5, '-': 5, '*': 6, '/': 6, '%': 6 }

export function parse(src: string): Node {
  const toks = lex(src)
  let p = 0
  const peek = () => toks[p]
  const take = () => toks[p++]
  const expect = (v: string) => {
    const t = take()
    if (t.t !== 'op' || t.v !== v) throw new ExprError(`expected "${v}" at ${t.pos + 1}`, t.pos)
  }

  const primary = (): Node => {
    const t = take()
    switch (t.t) {
      case 'num': return { k: 'lit', v: Number(t.v) }
      case 'str': return { k: 'lit', v: t.v }
      case 'tag': return { k: 'tag', ref: t.v }
      case 'id': {
        if (t.v === 'true' || t.v === 'false') return { k: 'lit', v: t.v === 'true' }
        if (t.v === 'null') return { k: 'lit', v: null }
        const fn = FUNCTIONS[t.v]
        if (!fn) throw new ExprError(`unknown name "${t.v}"`, t.pos)
        expect('(')
        const args: Node[] = []
        if (!(peek().t === 'op' && peek().v === ')')) {
          args.push(ternary())
          while (peek().t === 'op' && peek().v === ',') { take(); args.push(ternary()) }
        }
        expect(')')
        if (args.length < fn.min || args.length > fn.max) throw new ExprError(`${fn.doc} takes ${fn.min === fn.max ? fn.min : `${fn.min}-${fn.max}`} arguments`, t.pos)
        return { k: 'call', fn: t.v, args }
      }
      case 'op':
        if (t.v === '(') { const e = ternary(); expect(')'); return e }
        if (t.v === '-' || t.v === '!') return { k: 'un', op: t.v, a: primary() }
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
      left = { k: 'bin', op: t.v, a: left, b: binary(prec + 1) }
    }
  }

  const ternary = (): Node => {
    const c = binary(1)
    if (peek().t === 'op' && peek().v === '?') {
      take()
      const a = ternary()
      expect(':')
      return { k: 'cond', c, a, b: ternary() }
    }
    return c
  }

  const root = ternary()
  if (peek().t !== 'end') throw new ExprError(`unexpected "${peek().v}" at ${peek().pos + 1}`, peek().pos)
  return root
}

/** Tag refs an expression reads. */
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
        return args.length > 1 && typeof v === 'number' ? v.toFixed(num(args[1])) : String(v)
      }
      if (args.some(a => a == null)) return null
      const xs = args.map(num)
      switch (n.fn) {
        case 'abs': return Math.abs(xs[0])
        case 'round': { const f = 10 ** (xs[1] ?? 0); return Math.round(xs[0] * f) / f }
        case 'floor': return Math.floor(xs[0])
        case 'ceil': return Math.ceil(xs[0])
        case 'min': return Math.min(...xs)
        case 'max': return Math.max(...xs)
        case 'clamp': return Math.min(xs[2], Math.max(xs[1], xs[0]))
        case 'bit': return Math.floor(xs[0] / 2 ** xs[1]) % 2 === 1
        case 'scale': return xs[2] === xs[1] ? null : xs[3] + ((xs[0] - xs[1]) * (xs[4] - xs[3])) / (xs[2] - xs[1])
      }
      return null
    }
  }
}

const cache = new Map<string, Node | ExprError>()

/** Parse once per distinct source text. */
export function compile(src: string): Node | ExprError {
  let c = cache.get(src)
  if (!c) {
    try { c = parse(src) } catch (e) { c = e instanceof ExprError ? e : new ExprError(String(e), 0) }
    if (cache.size > 500) cache.clear()
    cache.set(src, c)
  }
  return c
}
