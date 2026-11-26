//! A structural check of a y-sync update before `yrs` decodes it. `yrs`
//! decodes nested values recursively without a depth limit, so a few hundred
//! kilobytes of `[[[[…]]]]` overflow the stack and abort the process; Yjs in
//! the browser has the same weakness. [`check_update`] walks the lib0 v1
//! encoding iteratively and refuses what a screen document never holds:
//!
//! * values nested deeper than [`MAX_DEPTH`];
//! * content other than plain values, deletions and nested maps or arrays
//!   (no text, XML, embeds, formats, binary, JSON strings or subdocuments);
//! * top-level types other than [`WIDGETS`] and [`CONNECTIONS`];
//! * counts larger than the bytes that could hold them, and clocks that
//!   overflow.
//!
//! [`WIDGETS`]: crate::document::WIDGETS
//! [`CONNECTIONS`]: crate::document::CONNECTIONS

use crate::document::{CONNECTIONS, WIDGETS};

/// Deepest nesting of a value (an array in an array … counts one per level).
pub const MAX_DEPTH: usize = 32;
/// Longest map key or string of an update's structure (not values), bytes.
const MAX_KEY: usize = 1024;
/// Highest clock an update may reach.
const MAX_CLOCK: u64 = 1 << 31;

/// Why an update was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

fn refuse<T>(why: impl Into<String>) -> Result<T, Refused> {
    Err(Refused(why.into()))
}

struct Reader<'a> {
    buf: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn left(&self) -> usize {
        self.buf.len() - self.at
    }

    fn u8(&mut self) -> Result<u8, Refused> {
        let b = *self
            .buf
            .get(self.at)
            .ok_or_else(|| Refused("update ends early".into()))?;
        self.at += 1;
        Ok(b)
    }

    fn var(&mut self) -> Result<u64, Refused> {
        let mut n: u64 = 0;
        let mut shift = 0;
        loop {
            let b = self.u8()?;
            if shift >= 64 {
                return refuse("variable-length number too long");
            }
            n |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(n);
            }
            shift += 7;
        }
    }

    /// Skip a signed variable-length number.
    fn skip_var(&mut self) -> Result<(), Refused> {
        for _ in 0..10 {
            if self.u8()? & 0x80 == 0 {
                return Ok(());
            }
        }
        refuse("variable-length number too long")
    }

    /// A count of things each at least `min` bytes long.
    fn count(&mut self, min: usize) -> Result<usize, Refused> {
        let n = self.var()?;
        if n > (self.left() / min.max(1)) as u64 {
            return refuse("a count is larger than the update");
        }
        Ok(n as usize)
    }

    fn bytes(&mut self) -> Result<&'a [u8], Refused> {
        let n = self.count(1)?;
        let s = &self.buf[self.at..self.at + n];
        self.at += n;
        Ok(s)
    }

    fn skip(&mut self, n: usize) -> Result<(), Refused> {
        if n > self.left() {
            return refuse("update ends early");
        }
        self.at += n;
        Ok(())
    }

    fn string(&mut self) -> Result<&'a str, Refused> {
        std::str::from_utf8(self.bytes()?).map_err(|_| Refused("a string is not UTF-8".into()))
    }

    fn key(&mut self) -> Result<&'a str, Refused> {
        let s = self.string()?;
        if s.len() > MAX_KEY {
            return refuse("a key is too long");
        }
        Ok(s)
    }

    /// Skip one lib0 `Any`, iteratively, refusing nesting over [`MAX_DEPTH`].
    fn any(&mut self) -> Result<(), Refused> {
        // Per open level: values still to read, and whether they have keys.
        let mut stack: Vec<(usize, bool)> = vec![(1, false)];
        while let Some(top) = stack.last_mut() {
            if top.0 == 0 {
                stack.pop();
                continue;
            }
            top.0 -= 1;
            if top.1 {
                self.string()?;
            }
            match self.u8()? {
                127 | 126 | 121 | 120 => {}
                125 => self.skip_var()?,
                124 => self.skip(4)?,
                123 | 122 => self.skip(8)?,
                119 => {
                    self.string()?;
                }
                116 => {
                    self.bytes()?;
                }
                t @ (118 | 117) => {
                    let is_map = t == 118;
                    let n = self.count(if is_map { 2 } else { 1 })?;
                    if stack.len() > MAX_DEPTH {
                        return refuse(format!("values nested deeper than {MAX_DEPTH}"));
                    }
                    stack.push((n, is_map));
                }
                t => return refuse(format!("unknown value type {t}")),
            }
        }
        Ok(())
    }

    fn id(&mut self) -> Result<(), Refused> {
        self.var()?;
        let clock = self.var()?;
        if clock >= MAX_CLOCK {
            return refuse("clock out of range");
        }
        Ok(())
    }
}

// Item info bits (yrs `block.rs`).
const HAS_ORIGIN: u8 = 0b1000_0000;
const HAS_RIGHT_ORIGIN: u8 = 0b0100_0000;
const HAS_PARENT_SUB: u8 = 0b0010_0000;
const GC: u8 = 0;
const SKIP: u8 = 10;
// Content refs.
const DELETED: u8 = 1;
const TYPE: u8 = 7;
const ANY: u8 = 8;
// Type refs.
const ARRAY: u8 = 0;
const MAP: u8 = 1;

/// Check an update (lib0 v1, as in a sync step 2 or update message) before
/// it is decoded. See the module documentation for what is refused.
pub fn check_update(update: &[u8]) -> Result<(), Refused> {
    let mut r = Reader { buf: update, at: 0 };
    // Block runs: count, then per client its blocks.
    let clients = r.count(3)?;
    for _ in 0..clients {
        let blocks = r.count(1)?;
        r.var()?;
        let mut clock = r.var()?;
        for _ in 0..blocks {
            let info = r.u8()?;
            let len = match info {
                GC | SKIP => {
                    let len = r.var()?;
                    if len == 0 {
                        return refuse("an empty block");
                    }
                    len
                }
                info => item(&mut r, info)?,
            };
            clock = clock.saturating_add(len);
            if clock >= MAX_CLOCK {
                return refuse("clock out of range");
            }
        }
    }
    // Delete set: per client, ranges of (clock, len).
    let clients = r.count(2)?;
    for _ in 0..clients {
        r.var()?;
        let ranges = r.count(2)?;
        for _ in 0..ranges {
            let clock = r.var()?;
            let len = r.var()?;
            if clock.saturating_add(len) >= MAX_CLOCK {
                return refuse("deleted range out of range");
            }
        }
    }
    Ok(())
}

/// Check one item; its length in clock ticks.
fn item(r: &mut Reader<'_>, info: u8) -> Result<u64, Refused> {
    if info & HAS_ORIGIN != 0 {
        r.id()?;
    }
    if info & HAS_RIGHT_ORIGIN != 0 {
        r.id()?;
    }
    if info & (HAS_ORIGIN | HAS_RIGHT_ORIGIN) == 0 {
        // The parent: a named root or the id of a nested type.
        if r.var()? == 1 {
            let root = r.string()?;
            if root != WIDGETS && root != CONNECTIONS {
                return refuse(format!(
                    "unknown top-level type '{}'",
                    root.chars().take(64).collect::<String>()
                ));
            }
        } else {
            r.id()?;
        }
        if info & HAS_PARENT_SUB != 0 {
            r.key()?;
        }
    }
    match info & 0b1111 {
        DELETED => {
            let len = r.var()?;
            if len == 0 {
                return refuse("an empty deletion");
            }
            Ok(len)
        }
        ANY => {
            let n = r.count(1)?;
            if n == 0 {
                return refuse("an empty value list");
            }
            for _ in 0..n {
                r.any()?;
            }
            Ok(n as u64)
        }
        TYPE => match r.u8()? {
            ARRAY | MAP => Ok(1),
            t => refuse(format!("type {t} is not used by screen documents")),
        },
        c => refuse(format!("content {c} is not used by screen documents")),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::document::{self, Rows};
    use yrs::{Any, Array, Doc, Map, MapPrelim, ReadTxn, StateVector, Transact, WriteTxn};

    /// The exact payload of the review: one item `widgets.w` whose value is
    /// `[117, 1]` repeated 120 000 times (an array of an array of …).
    pub(crate) fn stack_bomb() -> Vec<u8> {
        // 1 client, 1 block, client 5, clock 0; an item of plain values with
        // a key and no origins; parent: the root "widgets"; key "w"; 1 value.
        let mut u = vec![1u8, 1, 5, 0, ANY | HAS_PARENT_SUB, 1, WIDGETS.len() as u8];
        u.extend(WIDGETS.as_bytes());
        u.extend([1, b'w', 1]);
        for _ in 0..120_000 {
            u.extend([117, 1]);
        }
        // The innermost value (integer 1), then an empty delete set.
        u.extend([125, 1, 0]);
        u
    }

    fn nested(depth: usize) -> Vec<u8> {
        // An update setting widgets.w = [[…[1]…]] (`depth` arrays).
        let doc = Doc::new();
        let mut v = Any::from(1);
        for _ in 0..depth {
            v = Any::Array(vec![v].into());
        }
        {
            let mut t = doc.transact_mut();
            let m = t.get_or_insert_map(WIDGETS);
            m.insert(&mut t, "w", v);
        }
        doc.transact()
            .encode_state_as_update_v1(&StateVector::default())
    }

    #[test]
    fn documents_pass() {
        let doc = Doc::new();
        {
            let mut t = doc.transact_mut();
            document::load(
                &mut t,
                &Rows {
                    widgets: vec![document::tests::wrow(
                        "a",
                        1.0,
                        r#"{"bindings":{"label":{"kind":"tag","tag":"d/k"}},"z":1,"deep":[[[1,{"a":[2]}]]]}"#,
                    )],
                    connections: vec![document::tests::crow("c", "a", "a")],
                },
            );
        }
        {
            let mut t = doc.transact_mut();
            let w = t.get_or_insert_map(WIDGETS);
            w.remove(&mut t, "a");
        }
        let u = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        check_update(&u).unwrap();
        check_update(&nested(MAX_DEPTH - 1)).unwrap();
    }

    #[test]
    fn deep_nesting_is_refused() {
        assert!(check_update(&nested(MAX_DEPTH + 1)).is_err());
    }

    #[test]
    fn the_stack_bomb_is_refused() {
        let u = stack_bomb();
        assert!(u.len() > 200_000 && u.len() < 256 * 1024);
        let e = check_update(&u).unwrap_err();
        assert!(e.0.contains("nested"), "{e}");
    }

    #[test]
    fn unknown_roots_and_types_are_refused() {
        let doc = Doc::new();
        {
            let mut t = doc.transact_mut();
            let m = t.get_or_insert_map("junk");
            m.insert(&mut t, "k", "v");
        }
        let u = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        assert!(check_update(&u).unwrap_err().0.contains("top-level"));

        let doc = Doc::new();
        {
            let mut t = doc.transact_mut();
            let m = t.get_or_insert_map(WIDGETS);
            m.insert(&mut t, "t", yrs::TextPrelim::new("hi"));
        }
        let u = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        assert!(check_update(&u).is_err());

        let doc = Doc::new();
        {
            let mut t = doc.transact_mut();
            let m = t.get_or_insert_map(WIDGETS);
            let inner = m.insert(&mut t, "w", MapPrelim::default());
            let arr = inner.insert(&mut t, "a", yrs::ArrayPrelim::default());
            arr.push_back(&mut t, 1);
        }
        let u = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        check_update(&u).unwrap();
        let _ = doc
            .transact()
            .get_map(WIDGETS)
            .map(|m| m.len(&doc.transact()));
    }

    #[test]
    fn truncated_and_oversized_counts_are_refused() {
        let u = nested(3);
        for cut in 1..u.len() {
            let _ = check_update(&u[..cut]);
        }
        assert!(check_update(&[0xff, 0xff, 0xff, 0xff, 0x0f]).is_err());
        assert!(check_update(&[1, 1, 5, 0xff, 0xff, 0xff, 0xff, 0x0f, SKIP, 1, 0]).is_err());
    }
}
