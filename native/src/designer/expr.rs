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

use std::collections::{BTreeSet, HashMap};
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
            Self::Str(s) => {
                let s = s.trim();
                if s.is_empty() {
                    0.0
                } else {
                    s.parse().unwrap_or(f64::NAN)
                }
            }
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

/// JavaScript's number to text: integers without a fraction, the shortest
/// round-trip digits, exponents outside 1e-6..1e21.
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
    if (1e-6..1e21).contains(&abs) {
        return format!("{x}");
    }
    // `1e21` → `1e+21`, `1.5e-7` stays.
    let text = format!("{x:e}");
    match text.split_once('e') {
        Some((mantissa, exp)) if !exp.starts_with('-') => format!("{mantissa}e+{exp}"),
        _ => text,
    }
}

/// JavaScript's `x.toFixed(digits)`: ties round away from zero.
pub fn to_fixed(x: f64, digits: f64) -> String {
    let digits = if digits.is_nan() {
        0
    } else {
        digits.clamp(0.0, 100.0) as usize
    };
    if !x.is_finite() || x.abs() >= 1e21 {
        return js_number(x);
    }
    let scale = 10f64.powi(digits as i32);
    let scaled = x.abs() * scale;
    if scaled.fract() == 0.5 {
        let rounded = scaled.round() / scale;
        let sign = if x < 0.0 { "-" } else { "" };
        return format!("{sign}{rounded:.digits$}");
    }
    format!("{x:.digits$}")
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
    let rest = |i: usize| chars[i..].iter().collect::<String>();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || c == '.' {
            let len = number_len(&chars[i..]);
            if len == 0 {
                return Err(error(format!("bad number at {}", i + 1), i));
            }
            out.push((Tok::Num(chars[i..i + len].iter().collect()), i));
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
                return Err(error("unterminated string", i));
            }
            out.push((Tok::Str(s), i));
            i = j + 1;
            continue;
        }
        if c == '{' {
            let Some(end) = chars[i..].iter().position(|&ch| ch == '}') else {
                return Err(error("missing } after tag reference", i));
            };
            let reference: String = chars[i + 1..i + end].iter().collect();
            let reference = reference.trim();
            if reference.is_empty() {
                return Err(error("empty tag reference", i));
            }
            out.push((Tok::Tag(reference.to_owned()), i));
            i += end + 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let len = chars[i..]
                .iter()
                .take_while(|ch| ch.is_ascii_alphanumeric() || **ch == '_')
                .count();
            out.push((Tok::Id(chars[i..i + len].iter().collect()), i));
            i += len;
            continue;
        }
        let tail = rest(i);
        let Some(op) = OPS.iter().find(|op| tail.starts_with(**op)) else {
            return Err(error(format!("unexpected \"{c}\" at {}", i + 1), i));
        };
        out.push((Tok::Op(op), i));
        i += op.chars().count();
    }
    out.push((Tok::End, chars.len()));
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
    ("bit", 2, 2, "bit(x, n): bit n of an integer"),
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

struct Parser {
    toks: Vec<(Tok, usize)>,
    at: usize,
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

    fn primary(&mut self) -> Result<Node, ExprError> {
        let (tok, pos) = self.take();
        match tok {
            Tok::Num(text) => Ok(Node::Lit(Value::Num(text.parse().unwrap_or(f64::NAN)))),
            Tok::Str(text) => Ok(Node::Lit(Value::Str(text))),
            Tok::Tag(reference) => Ok(Node::Tag(reference)),
            Tok::Id(name) => {
                match name.as_str() {
                    "true" => return Ok(Node::Lit(Value::Bool(true))),
                    "false" => return Ok(Node::Lit(Value::Bool(false))),
                    "null" => return Ok(Node::Lit(Value::Null)),
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
                Ok(Node::Call(function, args))
            }
            Tok::Op("(") => {
                let inner = self.ternary()?;
                self.expect(")")?;
                Ok(inner)
            }
            Tok::Op("-") => Ok(Node::Neg(Box::new(self.primary()?))),
            Tok::Op("!") => Ok(Node::Not(Box::new(self.primary()?))),
            Tok::Op(op) => Err(error(format!("unexpected \"{op}\" at {}", pos + 1), pos)),
            Tok::End => Err(error("unexpected end of expression", pos)),
        }
    }

    fn binary(&mut self, least: u8) -> Result<Node, ExprError> {
        let mut left = self.primary()?;
        loop {
            let op = match &self.peek().0 {
                Tok::Op(op) => *op,
                _ => return Ok(left),
            };
            let Some(prec) = precedence(op).filter(|prec| *prec >= least) else {
                return Ok(left);
            };
            self.take();
            let right = self.binary(prec + 1)?;
            left = Node::Bin(op, Box::new(left), Box::new(right));
        }
    }

    fn ternary(&mut self) -> Result<Node, ExprError> {
        let cond = self.binary(1)?;
        if self.peek_op("?") {
            self.take();
            let then = self.ternary()?;
            self.expect(":")?;
            let otherwise = self.ternary()?;
            return Ok(Node::Cond(
                Box::new(cond),
                Box::new(then),
                Box::new(otherwise),
            ));
        }
        Ok(cond)
    }
}

/// Parse an expression.
pub fn parse(src: &str) -> Result<Node, ExprError> {
    let mut parser = Parser {
        toks: lex(src)?,
        at: 0,
    };
    let root = parser.ternary()?;
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

/// `Math.round`: halves toward +∞.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
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
            Value::Num(x) if values.len() > 1 => Value::Str(to_fixed(*x, values[1].num())),
            value => Value::Str(value.text()),
        };
    }
    if values.contains(&Value::Null) {
        return Value::Null;
    }
    let xs: Vec<f64> = values.iter().map(Value::num).collect();
    let nan_or = |fold: fn(f64, f64) -> f64, start: f64| {
        if xs.iter().any(|x| x.is_nan()) {
            f64::NAN
        } else {
            xs.iter().copied().fold(start, fold)
        }
    };
    match function {
        "abs" => Value::Num(xs[0].abs()),
        "round" => {
            let factor = 10f64.powf(xs.get(1).copied().unwrap_or(0.0));
            Value::Num(js_round(xs[0] * factor) / factor)
        }
        "floor" => Value::Num(xs[0].floor()),
        "ceil" => Value::Num(xs[0].ceil()),
        "min" => Value::Num(nan_or(f64::min, f64::INFINITY)),
        "max" => Value::Num(nan_or(f64::max, f64::NEG_INFINITY)),
        "clamp" => {
            if xs.iter().any(|x| x.is_nan()) {
                Value::Num(f64::NAN)
            } else {
                Value::Num(xs[2].min(xs[1].max(xs[0])))
            }
        }
        "bit" => Value::Bool((xs[0] / 2f64.powf(xs[1])).floor() % 2.0 == 1.0),
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

/// Parses once per distinct text (the web's `compile`).
#[derive(Debug, Default)]
pub struct Cache {
    parsed: HashMap<String, Result<Node, ExprError>>,
}

impl Cache {
    /// Entries kept before the cache starts over.
    pub const LIMIT: usize = 500;

    pub fn compile(&mut self, src: &str) -> &Result<Node, ExprError> {
        if !self.parsed.contains_key(src) {
            if self.parsed.len() > Self::LIMIT {
                self.parsed.clear();
            }
            self.parsed.insert(src.to_owned(), parse(src));
        }
        &self.parsed[src]
    }
}
