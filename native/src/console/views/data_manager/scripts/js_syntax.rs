//! A small JavaScript tokenizer for the script editor's colours: keywords,
//! literals, the sandbox's builtins, function names, strings, template
//! strings (with their `${…}` parts), numbers and comments. It reads one
//! line at a time and carries what spans lines (a block comment, an open
//! template string) in a [`LineState`], so the editor colours only the lines
//! on screen. Not a parser: a `/` that starts a regular expression reads as
//! an operator, and a string never runs past its line, so a mistake stays on
//! one line.

use std::ops::Range;

/// What a token is, for its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Keyword,
    /// `true`, `false`, `null`, `undefined`, `NaN`, `Infinity`, `this`.
    Literal,
    /// The globals a script uses: `Math`, `JSON`, `input`, …
    Builtin,
    /// A name followed by `(`: a call or a declaration.
    Function,
    String,
    /// A template string's text and quotes.
    Template,
    /// `${` and `}` around a template string's expression.
    Interpolation,
    Number,
    Comment,
}

/// One coloured token of a line, by byte range within the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Token {
    pub range: Range<usize>,
    pub kind: Kind,
}

/// Where a template string stands at a line break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frame {
    /// In the template's text.
    Text,
    /// In a `${…}`, with this many `{` open inside it.
    Expr(u16),
}

/// What a line starts inside: a block comment, or open template strings
/// (innermost last). The state at the text's start is the default.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct LineState {
    block_comment: bool,
    templates: Vec<Frame>,
}

const KEYWORDS: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "of",
    "return",
    "static",
    "super",
    "switch",
    "throw",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

const LITERALS: &[&str] = &[
    "true",
    "false",
    "null",
    "undefined",
    "NaN",
    "Infinity",
    "this",
];

/// The sandbox's globals (QuickJS's ES intrinsics) and `input`, the
/// script's argument.
const BUILTINS: &[&str] = &[
    "Array",
    "BigInt",
    "Boolean",
    "Date",
    "Error",
    "JSON",
    "Map",
    "Math",
    "Number",
    "Object",
    "Promise",
    "RangeError",
    "RegExp",
    "Set",
    "String",
    "Symbol",
    "TypeError",
    "WeakMap",
    "WeakSet",
    "console",
    "globalThis",
    "input",
    "isFinite",
    "isNaN",
    "parseFloat",
    "parseInt",
];

/// The coloured tokens of `line` (no line break in it), starting in
/// `state`; `state` becomes the next line's.
pub(crate) fn tokenize_line(line: &str, state: &mut LineState) -> Vec<Token> {
    let mut tokens = Vec::new();
    scan(line, state, |range, kind| {
        // Neighbours of one kind become one token.
        match tokens.last_mut() {
            Some(Token {
                range: last,
                kind: last_kind,
            }) if *last_kind == kind && last.end == range.start => last.end = range.end,
            _ => tokens.push(Token { range, kind }),
        }
    });
    tokens
}

/// Move `state` past `line` without collecting its tokens.
pub(crate) fn advance(line: &str, state: &mut LineState) {
    scan(line, state, |_, _| {});
}

/// `line` cut into spans that cover it end to end: the tokens, and the
/// plain text between them (`None`).
pub(crate) fn spans(len: usize, tokens: &[Token]) -> Vec<(Range<usize>, Option<Kind>)> {
    let mut out = Vec::with_capacity(tokens.len() * 2 + 1);
    let mut at = 0;
    for token in tokens {
        let start = token.range.start.clamp(at, len);
        let end = token.range.end.clamp(start, len);
        if start > at {
            out.push((at..start, None));
        }
        if end > start {
            out.push((start..end, Some(token.kind)));
        }
        at = end;
    }
    if at < len {
        out.push((at..len, None));
    }
    out
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$' || byte >= 0x80
}

fn is_ident(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit()
}

#[allow(clippy::too_many_lines)]
fn scan(line: &str, state: &mut LineState, mut emit: impl FnMut(Range<usize>, Kind)) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if state.block_comment {
            match line[i..].find("*/") {
                Some(end) => {
                    emit(i..i + end + 2, Kind::Comment);
                    i += end + 2;
                    state.block_comment = false;
                }
                None => {
                    emit(i..len, Kind::Comment);
                    i = len;
                }
            }
            continue;
        }
        if state.templates.last() == Some(&Frame::Text) {
            let start = i;
            let mut closed = false;
            let mut interpolation = false;
            while i < len {
                match bytes[i] {
                    b'\\' => i = (i + 2).min(len),
                    b'`' => {
                        i += 1;
                        closed = true;
                        break;
                    }
                    b'$' if bytes.get(i + 1) == Some(&b'{') => {
                        interpolation = true;
                        break;
                    }
                    _ => i += 1,
                }
            }
            if i > start {
                emit(start..i, Kind::Template);
            }
            if closed {
                state.templates.pop();
            } else if interpolation {
                emit(i..i + 2, Kind::Interpolation);
                i += 2;
                if let Some(top) = state.templates.last_mut() {
                    *top = Frame::Expr(0);
                }
            }
            continue;
        }
        let byte = bytes[i];
        match byte {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                emit(i..len, Kind::Comment);
                i = len;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => match line[i + 2..].find("*/") {
                Some(end) => {
                    emit(i..i + 2 + end + 2, Kind::Comment);
                    i += 2 + end + 2;
                }
                None => {
                    emit(i..len, Kind::Comment);
                    i = len;
                    state.block_comment = true;
                }
            },
            b'"' | b'\'' => {
                let start = i;
                i += 1;
                while i < len {
                    match bytes[i] {
                        b'\\' => i = (i + 2).min(len),
                        quote if quote == byte => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
                emit(start..i, Kind::String);
            }
            b'`' => {
                emit(i..i + 1, Kind::Template);
                i += 1;
                state.templates.push(Frame::Text);
            }
            b'{' => {
                if let Some(Frame::Expr(depth)) = state.templates.last_mut() {
                    *depth += 1;
                }
                i += 1;
            }
            b'}' => {
                match state.templates.last_mut() {
                    Some(top @ Frame::Expr(0)) => {
                        emit(i..i + 1, Kind::Interpolation);
                        *top = Frame::Text;
                    }
                    Some(Frame::Expr(depth)) => *depth -= 1,
                    _ => {}
                }
                i += 1;
            }
            b'0'..=b'9' => {
                let start = i;
                i = number_end(bytes, i);
                emit(start..i, Kind::Number);
            }
            b'.' if bytes.get(i + 1).is_some_and(u8::is_ascii_digit) => {
                let start = i;
                i = number_end(bytes, i + 1);
                emit(start..i, Kind::Number);
            }
            byte if is_ident_start(byte) => {
                let start = i;
                while i < len && is_ident(bytes[i]) {
                    i += 1;
                }
                let word = &line[start..i];
                let after_dot = line[..start].trim_end().ends_with('.')
                    && !line[..start].trim_end().ends_with("..");
                let called = line[i..].trim_start().starts_with('(');
                let kind = if after_dot {
                    // A property: only a call stands out.
                    called.then_some(Kind::Function)
                } else if KEYWORDS.contains(&word) {
                    Some(Kind::Keyword)
                } else if LITERALS.contains(&word) {
                    Some(Kind::Literal)
                } else if BUILTINS.contains(&word) {
                    Some(Kind::Builtin)
                } else if called {
                    Some(Kind::Function)
                } else {
                    None
                };
                if let Some(kind) = kind {
                    emit(start..i, kind);
                }
            }
            _ => i += 1,
        }
    }
}

/// A number starting at `start`: digits, `0x…`/`0b…`/`0o…`, a fraction, an
/// exponent with its sign, `_` separators and a `n` suffix.
fn number_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() {
        let byte = bytes[i];
        let exponent_sign = (byte == b'+' || byte == b'-')
            && i > start
            && matches!(bytes[i - 1], b'e' | b'E')
            && !matches!(bytes.get(start + 1), Some(b'x' | b'X'));
        if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.' || exponent_sign {
            i += 1;
        } else {
            break;
        }
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tokens of `line` as (text, kind), from the start state.
    fn kinds(line: &str) -> Vec<(&str, Kind)> {
        let mut state = LineState::default();
        tokenize_line(line, &mut state)
            .into_iter()
            .map(|token| (&line[token.range], token.kind))
            .collect()
    }

    #[test]
    fn keywords_literals_builtins_and_calls_are_told_apart() {
        assert_eq!(
            kinds("function main(input) { return Math.max(input.value, null); }"),
            vec![
                ("function", Kind::Keyword),
                ("main", Kind::Function),
                ("input", Kind::Builtin),
                ("return", Kind::Keyword),
                ("Math", Kind::Builtin),
                ("max", Kind::Function),
                ("input", Kind::Builtin),
                ("null", Kind::Literal),
            ]
        );
        // A property named like a keyword is a property.
        assert_eq!(kinds("x.default = 1"), vec![("1", Kind::Number)]);
        assert_eq!(
            kinds("const value = 3"),
            vec![("const", Kind::Keyword), ("3", Kind::Number)]
        );
    }

    #[test]
    fn strings_keep_escapes_and_stop_at_their_quote_or_the_line_end() {
        assert_eq!(
            kinds(r#"let a = "say \"hi\"", b = 'it\'s';"#),
            vec![
                ("let", Kind::Keyword),
                (r#""say \"hi\"""#, Kind::String),
                (r"'it\'s'", Kind::String),
            ]
        );
        assert_eq!(
            kinds("x = 'open"),
            vec![("'open", Kind::String)],
            "an unterminated string ends with its line"
        );
        // A `//` inside a string is no comment.
        assert_eq!(
            kinds("url = 'http://x' // note"),
            vec![("'http://x'", Kind::String), ("// note", Kind::Comment)]
        );
    }

    #[test]
    fn numbers_in_every_notation() {
        for number in [
            "42", "3.14", ".5", "1e-5", "2E+10", "0xFF", "0b1010", "1_000", "10n",
        ] {
            assert_eq!(kinds(number), vec![(number, Kind::Number)], "{number}");
        }
        assert_eq!(
            kinds("a - 1"),
            vec![("1", Kind::Number)],
            "a minus is not part of the number"
        );
        assert_eq!(
            kinds("x1 = 2"),
            vec![("2", Kind::Number)],
            "digits in a name"
        );
    }

    #[test]
    fn comments_run_across_lines() {
        let mut state = LineState::default();
        let first = "x = 1; /* starts";
        let tokens = tokenize_line(first, &mut state);
        assert_eq!(&first[tokens[1].range.clone()], "/* starts");
        assert_eq!(tokens[1].kind, Kind::Comment);
        let second = "still a comment */ return x";
        let tokens = tokenize_line(second, &mut state);
        assert_eq!(&second[tokens[0].range.clone()], "still a comment */");
        assert_eq!(tokens[0].kind, Kind::Comment);
        assert_eq!(tokens[1].kind, Kind::Keyword);
        assert_eq!(state, LineState::default());
        assert_eq!(kinds("/* one */ x"), vec![("/* one */", Kind::Comment)]);
    }

    #[test]
    fn template_strings_span_lines_and_colour_their_expressions() {
        let mut state = LineState::default();
        let first = "const s = `value ${input.value + 1} and";
        let got: Vec<_> = tokenize_line(first, &mut state)
            .into_iter()
            .map(|token| (&first[token.range], token.kind))
            .collect();
        assert_eq!(
            got,
            vec![
                ("const", Kind::Keyword),
                ("`value ", Kind::Template),
                ("${", Kind::Interpolation),
                ("input", Kind::Builtin),
                ("1", Kind::Number),
                ("}", Kind::Interpolation),
                (" and", Kind::Template),
            ]
        );
        let second = "more ${ {a: 1}.a } end` + 2";
        let got: Vec<_> = tokenize_line(second, &mut state)
            .into_iter()
            .map(|token| (&second[token.range], token.kind))
            .collect();
        assert_eq!(
            got,
            vec![
                ("more ", Kind::Template),
                ("${", Kind::Interpolation),
                ("1", Kind::Number),
                ("}", Kind::Interpolation),
                (" end`", Kind::Template),
                ("2", Kind::Number),
            ],
            "braces inside the expression do not close it"
        );
        assert_eq!(state, LineState::default());
    }

    #[test]
    fn nested_templates_and_advance_agree_with_tokenize() {
        let lines = ["a = `x ${ `inner ${b}` } y", "`", "/* c", "*/ d"];
        let (mut left, mut right) = (LineState::default(), LineState::default());
        for line in lines {
            tokenize_line(line, &mut left);
            advance(line, &mut right);
            assert_eq!(left, right, "{line}");
        }
        assert_eq!(left, LineState::default());
    }

    #[test]
    fn spans_cover_the_line_with_plain_gaps() {
        let line = "let x = 'a';";
        let mut state = LineState::default();
        let tokens = tokenize_line(line, &mut state);
        let spans = spans(line.len(), &tokens);
        assert_eq!(spans.first().map(|span| span.0.start), Some(0));
        assert_eq!(spans.last().map(|span| span.0.end), Some(line.len()));
        assert!(
            spans
                .windows(2)
                .all(|pair| pair[0].0.end == pair[1].0.start)
        );
        assert_eq!(
            spans
                .iter()
                .map(|(range, kind)| (&line[range.clone()], *kind))
                .collect::<Vec<_>>(),
            vec![
                ("let", Some(Kind::Keyword)),
                (" x = ", None),
                ("'a'", Some(Kind::String)),
                (";", None),
            ]
        );
        assert!(super::spans(0, &[]).is_empty());
    }

    #[test]
    fn non_ascii_text_keeps_character_boundaries() {
        let line = "const naïve = 'é' // ünïcode";
        let mut state = LineState::default();
        for token in tokenize_line(line, &mut state) {
            assert!(line.is_char_boundary(token.range.start));
            assert!(line.is_char_boundary(token.range.end));
        }
    }
}
