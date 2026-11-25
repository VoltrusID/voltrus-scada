//! Binding expressions: a port of the web Designer's `client/src/designer/expr.ts`,
//! with the same grammar, the same values and the same error texts, so an
//! expression saved by either editor evaluates alike in both. No `eval`,
//! no property access, no loops: numbers, strings, booleans, tag
//! references, arithmetic, comparison, logic, a ternary and a fixed list
//! of functions.
//!
//! ```text
//! {wtp/raw_tank_level} > 85 ? "HIGH" : "OK"
//! round({wtp/intake_flow} / 3600, 2)
//! {wtp/p101a_run} && !{wtp/p101b_fault}
//! ```
//!
//! A tag is referenced by its project ref, `{device/key}`, so an expression
//! survives export and import. Values follow JavaScript's rules where the
//! web evaluator relies on them (truthiness, `+` joining text, number to
//! text), and a missing tag or value gives "no value" (`Null`), never 0.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

/// A value an expression gives.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Str(String),
    Bool(bool),
    Null,
}

impl Value {
    /// JavaScript's `Number(v)`.
    pub fn num(&self) -> f64 {
        match self {
            Self::Num(x) => *x,
            Self::Bool(b) => f64::from(u8::from(*b)),
            Self::Null => f64::NAN,
            Self::Str(s) => js_string_to_number(s),
        }
    }

    /// JavaScript truthiness (`truthy` in expr.ts).
    pub fn truthy(&self) -> bool {
        match self {
            Self::Num(x) => *x != 0.0 && !x.is_nan(),
            Self::Str(s) => !s.is_empty(),
            Self::Bool(b) => *b,
            Self::Null => false,
        }
    }

    /// JavaScript's `String(v)`; `Null` is empty (the `?? ''` of `+`).
    pub fn text(&self) -> String {
        match self {
            Self::Num(x) => js_number(*x),
            Self::Str(s) => s.clone(),
            Self::Bool(b) => b.to_string(),
            Self::Null => String::new(),
        }
    }

    /// As JSON (non-finite numbers have none: `Null`).
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Num(x) => serde_json::Number::from_f64(*x)
                .map_or(serde_json::Value::Null, serde_json::Value::Number),
            Self::Str(s) => serde_json::Value::String(s.clone()),
            Self::Bool(b) => serde_json::Value::Bool(*b),
            Self::Null => serde_json::Value::Null,
        }
    }

    /// From JSON: arrays and objects have no expression value.
    pub fn from_json(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Number(n) => n.as_f64().map_or(Self::Null, Self::Num),
            serde_json::Value::String(s) => Self::Str(s.clone()),
            serde_json::Value::Bool(b) => Self::Bool(*b),
            _ => Self::Null,
        }
    }
}

/// JavaScript's number to text (`Number::toString`): the shortest digits
/// that read back as the number, the closest such when there are several;
/// integers without a fraction, exponents outside 1e-6..1e21.
pub fn js_number(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if x == 0.0 {
        return "0".into();
    }
    let abs = x.abs();
    // Rust's shortest digits; of the same length, the closest one (`{:.Ne}`
    // rounds the exact value), as JavaScript picks.
    let shortest = format!("{abs:e}");
    let count = shortest.split_once('e').map_or(1, |(mantissa, _)| {
        mantissa.chars().filter(char::is_ascii_digit).count()
    });
    let closest = format!("{abs:.prec$e}", prec = count.saturating_sub(1));
    let chosen = if closest.parse::<f64>().ok() == Some(abs) {
        closest
    } else {
        shortest
    };
    let (mantissa, exponent) = chosen.split_once('e').unwrap_or((&chosen, "0"));
    let mut digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    let k = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    // The point goes after `n` digits: the value is 0.digits × 10^n.
    let n = exponent.parse::<i32>().unwrap_or(0) + 1;
    let sign = if x < 0.0 { "-" } else { "" };
    let body = if k <= n && n <= 21 {
        format!(
            "{digits}{}",
            "0".repeat(usize::try_from(n - k).unwrap_or(0))
        )
    } else if 0 < n && n <= 21 {
        let (int, frac) = digits.split_at(usize::try_from(n).unwrap_or(0));
        format!("{int}.{frac}")
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat(usize::try_from(-n).unwrap_or(0)))
    } else {
        let (first, rest) = digits.split_at(1);
        let point = if rest.is_empty() { "" } else { "." };
        let e = n - 1;
        let e_sign = if e < 0 { "-" } else { "+" };
        format!("{first}{point}{rest}e{e_sign}{}", e.abs())
    };
    format!("{sign}{body}")
}

/// JavaScript's `x.toFixed(digits)`: the decimal nearest the exact binary
/// value, an exact tie rounding away from zero; `-0` (and anything that
/// rounds to zero from a zero) shows without a sign, a negative that rounds
/// to zero keeps it (`(-0.0001).toFixed(2)` is `-0.00`).
pub fn to_fixed(x: f64, digits: f64) -> String {
    let digits = if digits.is_nan() {
        0
    } else {
        digits.clamp(0.0, 100.0) as usize
    };
    if !x.is_finite() || x.abs() >= 1e21 {
        return js_number(x);
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let abs = x.abs();
    // Rust's `{:.N}` is exact, ties to even: only an exact tie differs.
    let body = if exact_tie(abs, digits) {
        round_up_last(&format!("{abs:.prec$}", prec = digits + 1))
    } else {
        format!("{abs:.digits$}")
    };
    format!("{sign}{body}")
}

/// Whether `x` (finite, not negative) lies exactly halfway between two
/// multiples of 10^-digits: x·10^digits has a fraction of exactly ½, which
/// for x = m·2^e (m odd) is e + digits = -1.
fn exact_tie(x: f64, digits: usize) -> bool {
    if x == 0.0 {
        return false;
    }
    let bits = x.to_bits();
    let exponent = i64::try_from((bits >> 52) & 0x7ff).unwrap_or(0);
    let fraction = bits & ((1 << 52) - 1);
    let (mantissa, e) = if exponent == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), exponent - 1075)
    };
    let digits = i64::try_from(digits).unwrap_or(i64::MAX / 2);
    e + i64::from(mantissa.trailing_zeros()) + digits == -1
}

/// `"2.5"` → `"3"`, `"0.125"` → `"0.13"`, `"9.95"` → `"10.0"`: drop the
/// last digit (a 5) and add one to the one before.
fn round_up_last(text: &str) -> String {
    let mut digits: Vec<u8> = text.as_bytes()[..text.len() - 1].to_vec();
    if digits.last() == Some(&b'.') {
        digits.pop();
    }
    let mut carry = true;
    for d in digits.iter_mut().rev() {
        if !carry {
            break;
        }
        match *d {
            b'.' => {}
            b'9' => *d = b'0',
            _ => {
                *d += 1;
                carry = false;
            }
        }
    }
    if carry {
        digits.insert(0, b'1');
    }
    String::from_utf8(digits).unwrap_or_default()
}

/// `Math.round`: the nearest integer, halves toward +∞, the sign of a zero
/// kept (`Math.round(-0.4)` is `-0`). Exact: `0.49999999999999994` gives 0
/// and integers above 2^52 stay.
pub fn js_round(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let floor = x.floor();
    let rounded = if x - floor >= 0.5 { floor + 1.0 } else { floor };
    if rounded == 0.0 && x.is_sign_negative() {
        -0.0
    } else {
        rounded
    }
}

/// JavaScript's white space and line terminators (`/\s/`, and what
/// `Number()` trims): not U+0085, but U+FEFF.
pub fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// JavaScript's `Number(text)`: trimmed of [`js_space`]; empty is 0;
/// `0x`, `0o`, `0b` integers; `Infinity` with a sign; otherwise a decimal
/// with an optional sign and exponent (no `inf`, `nan`, `1_0`).
pub fn js_string_to_number(text: &str) -> f64 {
    let s = text.trim_matches(js_space);
    if s.is_empty() {
        return 0.0;
    }
    let radix = match s.get(..2) {
        Some("0x" | "0X") => 16,
        Some("0o" | "0O") => 8,
        Some("0b" | "0B") => 2,
        _ => 0,
    };
    if radix != 0 {
        let digits = &s[2..];
        if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
            return f64::NAN;
        }
        return match u128::from_str_radix(digits, radix) {
            #[allow(clippy::cast_precision_loss)]
            Ok(n) => n as f64,
            Err(_) => digits.chars().fold(0.0, |acc, c| {
                acc * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0))
            }),
        };
    }
    let unsigned = s.strip_prefix(['+', '-']).unwrap_or(s);
    if unsigned == "Infinity" {
        return if s.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    // StrUnsignedDecimalLiteral: digits, a point, digits (one side at
    // least), then an optional exponent with digits.
    let chars: Vec<char> = unsigned.chars().collect();
    let len = number_len(&chars);
    if len == 0 || len != chars.len() {
        return f64::NAN;
    }
    s.parse().unwrap_or(f64::NAN)
}

/// A parsed expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Lit(Value),
    Tag(String),
    Neg(Box<Node>),
    Not(Box<Node>),
    Bin(&'static str, Box<Node>, Box<Node>),
    Cond(Box<Node>, Box<Node>, Box<Node>),
    Call(&'static str, Vec<Node>),
}

/// A parse error: the web editor's message and its position (in
/// characters).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprError {
    pub message: String,
    pub pos: usize,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ExprError {}

fn error(message: impl Into<String>, pos: usize) -> ExprError {
    ExprError {
        message: message.into(),
        pos,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(String),
    Str(String),
    Tag(String),
    Id(String),
    Op(&'static str),
    End,
}

const OPS: [&str; 19] = [
    "&&", "||", "==", "!=", ">=", "<=", "?", ":", "(", ")", ",", "+", "-", "*", "/", "%", ">", "<",
    "!",
];

fn lex(src: &str) -> Result<Vec<(Tok, usize)>, ExprError> {
    let chars: Vec<char> = src.chars().collect();
    // Positions are in UTF-16 units, as the web counts them.
    let mut at = Vec::with_capacity(chars.len() + 1);
    let mut units = 0;
    for c in &chars {
        at.push(units);
        units += c.len_utf16();
    }
    at.push(units);
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let pos = at[i];
        if js_space(c) {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || c == '.' {
            let len = number_len(&chars[i..]);
            if len == 0 {
                return Err(error(format!("bad number at {}", pos + 1), pos));
            }
            out.push((Tok::Num(chars[i..i + len].iter().collect()), pos));
            i += len;
            continue;
        }
        if c == '"' || c == '\'' {
            let mut j = i + 1;
            let mut s = String::new();
            while j < chars.len() && chars[j] != c {
                if chars[j] == '\\' && j + 1 < chars.len() {
                    s.push(chars[j + 1]);
                    j += 2;
                } else {
                    s.push(chars[j]);
                    j += 1;
                }
            }
            if j >= chars.len() {
                return Err(error("unterminated string", pos));
            }
            out.push((Tok::Str(s), pos));
            i = j + 1;
            continue;
        }
        if c == '{' {
            // A backslash escapes the next character: `{dev/a\}b}` is `dev/a}b`.
            let mut j = i + 1;
            let mut raw = String::new();
            while j < chars.len() && chars[j] != '}' {
                if chars[j] == '\\' && j + 1 < chars.len() {
                    raw.push(chars[j + 1]);
                    j += 2;
                } else {
                    raw.push(chars[j]);
                    j += 1;
                }
            }
            if j >= chars.len() {
                return Err(error("missing } after tag reference", pos));
            }
            // `String.trim` trims JavaScript's white space.
            let reference = raw.trim_matches(js_space);
            if reference.is_empty() {
                return Err(error("empty tag reference", pos));
            }
            out.push((Tok::Tag(reference.to_owned()), pos));
            i = j + 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let len = chars[i..]
                .iter()
                .take_while(|ch| ch.is_ascii_alphanumeric() || **ch == '_')
                .count();
            out.push((Tok::Id(chars[i..i + len].iter().collect()), pos));
            i += len;
            continue;
        }
        let rest = &chars[i..];
        let Some(op) = OPS
            .iter()
            .find(|op| op.len() <= rest.len() && op.chars().zip(rest).all(|(a, b)| a == *b))
        else {
            return Err(error(format!("unexpected \"{c}\" at {}", pos + 1), pos));
        };
        out.push((Tok::Op(op), pos));
        i += op.len();
    }
    out.push((Tok::End, units));
    Ok(out)
}

/// The length of `^(\d+\.?\d*|\.\d+)([eE][-+]?\d+)?`, 0 when it does not
/// match.
fn number_len(chars: &[char]) -> usize {
    let digits = |from: usize| {
        chars[from..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count()
    };
    let mut i = digits(0);
    if i > 0 {
        if chars.get(i) == Some(&'.') {
            i += 1;
            i += digits(i);
        }
    } else if chars.first() == Some(&'.') && digits(1) > 0 {
        i = 1 + digits(1);
    } else {
        return 0;
    }
    if matches!(chars.get(i), Some('e' | 'E')) {
        let mut j = i + 1;
        if matches!(chars.get(j), Some('+' | '-')) {
            j += 1;
        }
        let exp = digits(j);
        if exp > 0 {
            i = j + exp;
        }
    }
    i
}

/// The functions: name, least and most arguments, and the signature the
/// errors show.
pub const FUNCTIONS: [(&str, usize, usize, &str); 11] = [
    ("abs", 1, 1, "abs(x)"),
    ("round", 1, 2, "round(x, decimals?)"),
    ("floor", 1, 1, "floor(x)"),
    ("ceil", 1, 1, "ceil(x)"),
    ("min", 1, 16, "min(a, b, ...)"),
    ("max", 1, 16, "max(a, b, ...)"),
    ("clamp", 3, 3, "clamp(x, lo, hi)"),
    ("if", 3, 3, "if(cond, then, else)"),
    (
        "bit",
        2,
        2,
        "bit(x, n): bit n (0-52) of an integer, two's complement for negatives",
    ),
    ("scale", 5, 5, "scale(x, inLo, inHi, outLo, outHi)"),
    ("text", 1, 2, "text(x, decimals?)"),
];

/// Binary operators and how tightly they bind.
fn precedence(op: &str) -> Option<u8> {
    Some(match op {
        "||" => 1,
        "&&" => 2,
        "==" | "!=" => 3,
        ">" | "<" | ">=" | "<=" => 4,
        "+" | "-" => 5,
        "*" | "/" | "%" => 6,
        _ => return None,
    })
}

/// Longest expression accepted, in characters (UTF-16 units, as the web
/// counts them).
pub const MAX_SOURCE: usize = 2048;
/// Most nodes in one expression.
pub const MAX_NODES: usize = 1000;
/// Deepest nesting (also bounds the recursion of [`refs`] and
/// [`evaluate`]).
pub const MAX_DEPTH: usize = 100;

/// `{ref}` text for a tag ref, escaping what the lexer treats specially.
pub fn escape_ref(reference: &str) -> String {
    let mut out = String::with_capacity(reference.len() + 2);
    out.push('{');
    for c in reference.chars() {
        if c == '\\' || c == '}' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('}');
    out
}

/// A node the parser built and its depth (1 for a leaf).
type Built = (Node, usize);

struct Parser {
    toks: Vec<(Tok, usize)>,
    at: usize,
    nodes: usize,
    nesting: usize,
}

impl Parser {
    fn peek(&self) -> &(Tok, usize) {
        &self.toks[self.at.min(self.toks.len() - 1)]
    }

    fn take(&mut self) -> (Tok, usize) {
        let tok = self.peek().clone();
        self.at += 1;
        tok
    }

    fn peek_op(&self, op: &str) -> bool {
        matches!(&self.peek().0, Tok::Op(o) if *o == op)
    }

    fn expect(&mut self, op: &str) -> Result<(), ExprError> {
        let (tok, pos) = self.take();
        if tok == Tok::Op(OPS.iter().find(|o| **o == op).copied().unwrap_or("")) {
            Ok(())
        } else {
            Err(error(format!("expected \"{op}\" at {}", pos + 1), pos))
        }
    }

    /// A node built from children of depths `kids`: counted, and its depth
    /// checked (a long left-associative chain is caught too).
    fn mk(&mut self, node: Node, pos: usize, kids: &[usize]) -> Result<Built, ExprError> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(error(
                format!("expression has more than {MAX_NODES} parts"),
                pos,
            ));
        }
        let depth = 1 + kids.iter().copied().max().unwrap_or(0);
        if depth > MAX_DEPTH {
            return Err(error(
                format!("expression is nested deeper than {MAX_DEPTH} levels"),
                pos,
            ));
        }
        Ok((node, depth))
    }

    fn enter(&mut self) -> Result<(), ExprError> {
        self.nesting += 1;
        if self.nesting > MAX_DEPTH {
            let pos = self.peek().1;
            return Err(error(
                format!("expression is nested deeper than {MAX_DEPTH} levels"),
                pos,
            ));
        }
        Ok(())
    }

    fn primary(&mut self) -> Result<Built, ExprError> {
        self.enter()?;
        let node = self.primary_inner();
        self.nesting -= 1;
        node
    }

    fn primary_inner(&mut self) -> Result<Built, ExprError> {
        let (tok, pos) = self.take();
        match tok {
            Tok::Num(text) => self.mk(
                Node::Lit(Value::Num(text.parse().unwrap_or(f64::NAN))),
                pos,
                &[],
            ),
            Tok::Str(text) => self.mk(Node::Lit(Value::Str(text)), pos, &[]),
            Tok::Tag(reference) => self.mk(Node::Tag(reference), pos, &[]),
            Tok::Id(name) => {
                match name.as_str() {
                    "true" => return self.mk(Node::Lit(Value::Bool(true)), pos, &[]),
                    "false" => return self.mk(Node::Lit(Value::Bool(false)), pos, &[]),
                    "null" => return self.mk(Node::Lit(Value::Null), pos, &[]),
                    _ => {}
                }
                let Some(&(function, least, most, doc)) =
                    FUNCTIONS.iter().find(|(n, ..)| *n == name)
                else {
                    return Err(error(format!("unknown name \"{name}\""), pos));
                };
                self.expect("(")?;
                let mut args = Vec::new();
                if !self.peek_op(")") {
                    args.push(self.ternary()?);
                    while self.peek_op(",") {
                        self.take();
                        args.push(self.ternary()?);
                    }
                }
                self.expect(")")?;
                if args.len() < least || args.len() > most {
                    let count = if least == most {
                        least.to_string()
                    } else {
                        format!("{least}-{most}")
                    };
                    return Err(error(format!("{doc} takes {count} arguments"), pos));
                }
                let depths: Vec<usize> = args.iter().map(|(_, d)| *d).collect();
                let args = args.into_iter().map(|(node, _)| node).collect();
                self.mk(Node::Call(function, args), pos, &depths)
            }
            Tok::Op("(") => {
                let inner = self.ternary()?;
                self.expect(")")?;
                Ok(inner)
            }
            Tok::Op("-") => {
                let (a, d) = self.primary()?;
                self.mk(Node::Neg(Box::new(a)), pos, &[d])
            }
            Tok::Op("!") => {
                let (a, d) = self.primary()?;
                self.mk(Node::Not(Box::new(a)), pos, &[d])
            }
            Tok::Op(op) => Err(error(format!("unexpected \"{op}\" at {}", pos + 1), pos)),
            Tok::End => Err(error("unexpected end of expression", pos)),
        }
    }

    fn binary(&mut self, least: u8) -> Result<Built, ExprError> {
        let mut left = self.primary()?;
        loop {
            let op = match &self.peek().0 {
                Tok::Op(op) => *op,
                _ => return Ok(left),
            };
            let Some(prec) = precedence(op).filter(|prec| *prec >= least) else {
                return Ok(left);
            };
            let (_, pos) = self.take();
            let (right, dr) = self.binary(prec + 1)?;
            let (left_node, dl) = left;
            left = self.mk(
                Node::Bin(op, Box::new(left_node), Box::new(right)),
                pos,
                &[dl, dr],
            )?;
        }
    }

    fn ternary(&mut self) -> Result<Built, ExprError> {
        self.enter()?;
        let node = self.ternary_inner();
        self.nesting -= 1;
        node
    }

    fn ternary_inner(&mut self) -> Result<Built, ExprError> {
        let cond = self.binary(1)?;
        if self.peek_op("?") {
            let (_, pos) = self.take();
            let (then, dt) = self.ternary()?;
            self.expect(":")?;
            let (otherwise, de) = self.ternary()?;
            let (cond, dc) = cond;
            return self.mk(
                Node::Cond(Box::new(cond), Box::new(then), Box::new(otherwise)),
                pos,
                &[dc, dt, de],
            );
        }
        Ok(cond)
    }
}

/// Parse an expression. Text longer than [`MAX_SOURCE`], with more than
/// [`MAX_NODES`] parts or nested deeper than [`MAX_DEPTH`] fails.
pub fn parse(src: &str) -> Result<Node, ExprError> {
    if src.encode_utf16().count() > MAX_SOURCE {
        return Err(error(
            format!("expression is longer than {MAX_SOURCE} characters"),
            MAX_SOURCE,
        ));
    }
    let mut parser = Parser {
        toks: lex(src)?,
        at: 0,
        nodes: 0,
        nesting: 0,
    };
    let (root, _) = parser.ternary()?;
    let (tok, pos) = parser.peek().clone();
    if tok != Tok::End {
        let text = match tok {
            Tok::Num(s) | Tok::Str(s) | Tok::Tag(s) | Tok::Id(s) => s,
            Tok::Op(op) => op.to_owned(),
            Tok::End => String::new(),
        };
        return Err(error(format!("unexpected \"{text}\" at {}", pos + 1), pos));
    }
    Ok(root)
}

/// The tag refs an expression reads, sorted.
pub fn refs(node: &Node) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    collect_refs(node, &mut out);
    out
}

fn collect_refs(node: &Node, out: &mut BTreeSet<String>) {
    match node {
        Node::Lit(_) => {}
        Node::Tag(reference) => {
            out.insert(reference.clone());
        }
        Node::Neg(a) | Node::Not(a) => collect_refs(a, out),
        Node::Bin(_, a, b) => {
            collect_refs(a, out);
            collect_refs(b, out);
        }
        Node::Cond(c, a, b) => {
            collect_refs(c, out);
            collect_refs(a, out);
            collect_refs(b, out);
        }
        Node::Call(_, args) => args.iter().for_each(|arg| collect_refs(arg, out)),
    }
}

/// JavaScript's `a === b || (both non-null, neither text, equal as numbers)`.
fn loosely_equal(a: &Value, b: &Value) -> bool {
    let strict = match (a, b) {
        (Value::Num(x), Value::Num(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Null, Value::Null) => true,
        _ => false,
    };
    strict
        || (!matches!(a, Value::Null | Value::Str(_))
            && !matches!(b, Value::Null | Value::Str(_))
            && a.num() == b.num())
}

/// Decimals for `text()` and `round()`: an integer from 0 to 20 (what
/// `toFixed` accepts).
fn decimals(x: f64) -> f64 {
    if x.is_finite() {
        x.trunc().clamp(0.0, 20.0)
    } else {
        0.0
    }
}

/// Bit `n` of the integer part of `x`, negatives as two's complement;
/// `None` for a bit number outside 0-52.
#[allow(clippy::float_cmp)]
fn bit_of(x: f64, n: f64) -> Option<bool> {
    if !x.is_finite() || n.fract() != 0.0 || !(0.0..=52.0).contains(&n) {
        return None;
    }
    let q = (x.trunc() / 2f64.powf(n)).floor();
    Some(((q % 2.0) + 2.0) % 2.0 == 1.0)
}

/// `Math.min` of two: `NaN` wins, `-0` is less than `0`.
fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if b < a || (b == 0.0 && a == 0.0 && b.is_sign_negative()) {
        b
    } else {
        a
    }
}

/// `Math.max` of two: `NaN` wins, `0` is more than `-0`.
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if b > a || (b == 0.0 && a == 0.0 && a.is_sign_negative()) {
        b
    } else {
        a
    }
}

/// Evaluate. `tag(ref)` gives the tag's current value, or `None` when the
/// tag is unknown or has no value yet; arithmetic on no value gives no
/// value.
pub fn evaluate(node: &Node, tag: &dyn Fn(&str) -> Option<f64>) -> Value {
    match node {
        Node::Lit(value) => value.clone(),
        Node::Tag(reference) => tag(reference).map_or(Value::Null, Value::Num),
        Node::Not(a) => Value::Bool(!evaluate(a, tag).truthy()),
        Node::Neg(a) => match evaluate(a, tag) {
            Value::Null => Value::Null,
            value => Value::Num(-value.num()),
        },
        Node::Cond(c, a, b) => {
            if evaluate(c, tag).truthy() {
                evaluate(a, tag)
            } else {
                evaluate(b, tag)
            }
        }
        Node::Bin(op, a, b) => {
            if *op == "&&" {
                return Value::Bool(evaluate(a, tag).truthy() && evaluate(b, tag).truthy());
            }
            if *op == "||" {
                return Value::Bool(evaluate(a, tag).truthy() || evaluate(b, tag).truthy());
            }
            let (a, b) = (evaluate(a, tag), evaluate(b, tag));
            match *op {
                "==" => return Value::Bool(loosely_equal(&a, &b)),
                "!=" => return Value::Bool(!loosely_equal(&a, &b)),
                "+" if matches!(a, Value::Str(_)) || matches!(b, Value::Str(_)) => {
                    return Value::Str(format!("{}{}", a.text(), b.text()));
                }
                _ => {}
            }
            if a == Value::Null || b == Value::Null {
                return Value::Null;
            }
            let (x, y) = (a.num(), b.num());
            match *op {
                "+" => Value::Num(x + y),
                "-" => Value::Num(x - y),
                "*" => Value::Num(x * y),
                "/" if y == 0.0 => Value::Null,
                "/" => Value::Num(x / y),
                "%" if y == 0.0 => Value::Null,
                "%" => Value::Num(x % y),
                ">" => Value::Bool(x > y),
                "<" => Value::Bool(x < y),
                ">=" => Value::Bool(x >= y),
                "<=" => Value::Bool(x <= y),
                _ => Value::Null,
            }
        }
        Node::Call(function, args) => call(function, args, tag),
    }
}

fn call(function: &str, args: &[Node], tag: &dyn Fn(&str) -> Option<f64>) -> Value {
    if function == "if" {
        return if evaluate(&args[0], tag).truthy() {
            evaluate(&args[1], tag)
        } else {
            evaluate(&args[2], tag)
        };
    }
    let values: Vec<Value> = args.iter().map(|arg| evaluate(arg, tag)).collect();
    if function == "text" {
        return match &values[0] {
            Value::Null => Value::Str(String::new()),
            Value::Num(x) if values.len() > 1 && x.is_finite() => {
                let digits = match &values[1] {
                    Value::Null => 0.0,
                    value => decimals(value.num()),
                };
                Value::Str(to_fixed(*x, digits))
            }
            value => Value::Str(value.text()),
        };
    }
    if values.contains(&Value::Null) {
        return Value::Null;
    }
    let xs: Vec<f64> = values.iter().map(Value::num).collect();
    match function {
        "abs" => Value::Num(xs[0].abs()),
        "round" => {
            let factor = 10f64.powf(xs.get(1).copied().map_or(0.0, decimals));
            Value::Num(js_round(xs[0] * factor) / factor)
        }
        "floor" => Value::Num(xs[0].floor()),
        "ceil" => Value::Num(xs[0].ceil()),
        "min" => Value::Num(xs.iter().copied().fold(f64::INFINITY, js_min)),
        "max" => Value::Num(xs.iter().copied().fold(f64::NEG_INFINITY, js_max)),
        "clamp" => Value::Num(js_min(xs[2], js_max(xs[1], xs[0]))),
        "bit" => bit_of(xs[0], xs[1]).map_or(Value::Null, Value::Bool),
        "scale" => {
            if xs[2] == xs[1] {
                Value::Null
            } else {
                Value::Num(xs[3] + (xs[0] - xs[1]) * (xs[4] - xs[3]) / (xs[2] - xs[1]))
            }
        }
        _ => Value::Null,
    }
}

/// Evaluate, giving `Null` for anything that is not a number (`NaN`), as
/// the web's `safeEvaluate` does. Bindings evaluate through this.
pub fn safe_evaluate(node: &Node, tag: &dyn Fn(&str) -> Option<f64>) -> Value {
    match evaluate(node, tag) {
        Value::Num(x) if x.is_nan() => Value::Null,
        value => value,
    }
}

/// Parses once per distinct text (the web's `compile`): a least recently
/// used cache, [`Cache::LIMIT`] texts by default or as many as
/// [`Cache::fit`] asks for (a screen's binding texts); text longer than
/// [`MAX_SOURCE`] is not kept.
#[derive(Debug)]
pub struct Cache {
    /// Each text's parse and when it was last asked for.
    parsed: HashMap<String, (u64, Result<Node, ExprError>)>,
    /// The texts by when they were last asked for: the first goes first.
    order: BTreeMap<u64, String>,
    clock: u64,
    limit: usize,
    /// The last oversized text's answer (always the length error).
    oversized: Option<Result<Node, ExprError>>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            parsed: HashMap::new(),
            order: BTreeMap::new(),
            clock: 0,
            limit: Self::LIMIT,
            oversized: None,
        }
    }
}

impl Cache {
    /// Entries kept by default (the web's `CACHE_LIMIT`).
    pub const LIMIT: usize = 500;

    /// Keep at least `texts` entries (never fewer than [`Self::LIMIT`]),
    /// so a screen with more binding texts than that does not parse each
    /// of them again every frame.
    pub fn fit(&mut self, texts: usize) {
        self.limit = texts.max(Self::LIMIT);
        self.evict();
    }

    /// Entries kept now.
    pub fn limit(&self) -> usize {
        self.limit
    }

    fn evict(&mut self) {
        while self.parsed.len() > self.limit {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            self.parsed.remove(&oldest);
        }
    }

    pub fn compile(&mut self, src: &str) -> &Result<Node, ExprError> {
        if src.encode_utf16().count() > MAX_SOURCE {
            return self.oversized.insert(parse(src));
        }
        self.clock += 1;
        let now = self.clock;
        if let Some(entry) = self.parsed.get_mut(src) {
            if let Some(key) = self.order.remove(&entry.0) {
                self.order.insert(now, key);
            }
            entry.0 = now;
        } else {
            self.parsed.insert(src.to_owned(), (now, parse(src)));
            self.order.insert(now, src.to_owned());
            self.evict();
        }
        &self.parsed[src].1
    }

    /// Texts kept.
    pub fn len(&self) -> usize {
        self.parsed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parsed.is_empty()
    }
}
