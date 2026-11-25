//! Fractional indexes: strings that sort between any two others, so a widget
//! can be put between two neighbours without renumbering anything. The
//! stacking order of a screen document is its widgets sorted by `(zi, id)`.
//!
//! Keys are base-62 digits (`0-9A-Za-z`, in ASCII order) that never end in
//! `0`, so there is always a key between two different keys. The algorithm is
//! the midpoint of rocicorp's `fractional-indexing` without the integer part;
//! `client/src/designer/collab/fractional.ts` is the same code.

const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

fn digit(c: u8) -> Option<usize> {
    DIGITS.iter().position(|d| *d == c)
}

/// Whether `key` is a valid index: base-62 digits, not empty, not ending in `0`.
pub fn is_valid(key: &str) -> bool {
    !key.is_empty() && !key.ends_with('0') && key.bytes().all(|c| digit(c).is_some())
}

/// A key between `a` and `b` (`a < b`), either end open. Invalid or unordered
/// bounds are treated as open, so a damaged document still gets a key.
pub fn key_between(a: Option<&str>, b: Option<&str>) -> String {
    let a = a.filter(|k| is_valid(k));
    let b = b.filter(|k| is_valid(k));
    let (a, b) = match (a, b) {
        (Some(x), Some(y)) if x >= y => (Some(x), None),
        other => other,
    };
    midpoint(a.unwrap_or("").as_bytes(), b.map(str::as_bytes))
}

fn midpoint(a: &[u8], b: Option<&[u8]>) -> String {
    if let Some(b) = b {
        let mut n = 0;
        while n < b.len() && a.get(n).copied().unwrap_or(b'0') == b[n] {
            n += 1;
        }
        if n > 0 {
            let head = String::from_utf8_lossy(&b[..n]).into_owned();
            let rest_a = if n < a.len() { &a[n..] } else { &[] };
            return head + &midpoint(rest_a, Some(&b[n..]));
        }
    }
    let da = a.first().and_then(|c| digit(*c)).unwrap_or(0);
    let db = match b {
        Some(b) => b.first().and_then(|c| digit(*c)).unwrap_or(62),
        None => 62,
    };
    if db > da + 1 {
        let mid = (da + db).div_ceil(2);
        return (DIGITS[mid] as char).to_string();
    }
    // Consecutive digits.
    if let Some(b) = b
        && b.len() > 1
    {
        return (b[0] as char).to_string();
    }
    let rest = if a.len() > 1 { &a[1..] } else { &[] };
    let mut out = (DIGITS[da] as char).to_string();
    out.push_str(&midpoint(rest, None));
    out
}

/// `n` keys between `a` and `b`, in order, kept short by bisecting.
pub fn n_keys_between(a: Option<&str>, b: Option<&str>, n: usize) -> Vec<String> {
    if n == 0 {
        return Vec::new();
    }
    let mid = key_between(a, b);
    let left = n / 2;
    let mut out = n_keys_between(a, Some(&mid), left);
    let right = n_keys_between(Some(&mid), b, n - left - 1);
    out.push(mid);
    out.extend(right);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_sort_between_their_bounds() {
        let a = key_between(None, None);
        assert!(is_valid(&a));
        let b = key_between(Some(&a), None);
        assert!(a < b);
        let c = key_between(Some(&a), Some(&b));
        assert!(a < c && c < b, "{a} < {c} < {b}");
        let d = key_between(None, Some(&a));
        assert!(d < a);
    }

    #[test]
    fn repeated_inserts_at_one_place_stay_ordered() {
        let mut lo = key_between(None, None);
        let hi = key_between(Some(&lo), None);
        for _ in 0..200 {
            let k = key_between(Some(&lo), Some(&hi));
            assert!(lo < k && k < hi && is_valid(&k));
            lo = k;
        }
        let mut hi2 = hi.clone();
        for _ in 0..200 {
            let k = key_between(None, Some(&hi2));
            assert!(k < hi2 && is_valid(&k));
            hi2 = k;
        }
    }

    #[test]
    fn many_keys_are_ordered_and_short() {
        let keys = n_keys_between(None, None, 1000);
        assert_eq!(keys.len(), 1000);
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
        assert!(
            keys.iter().all(|k| is_valid(k) && k.len() <= 4),
            "{:?}",
            keys.iter().map(String::len).max()
        );
        let inner = n_keys_between(Some(&keys[3]), Some(&keys[4]), 10);
        assert!(inner.windows(2).all(|w| w[0] < w[1]));
        assert!(keys[3] < inner[0] && inner[9] < keys[4]);
    }

    #[test]
    fn bad_bounds_still_give_a_key() {
        assert!(is_valid(&key_between(Some("b"), Some("a"))));
        assert!(is_valid(&key_between(Some("a0"), None)));
        assert!(is_valid(&key_between(Some("!"), Some(""))));
    }

    /// The same keys as `fractional.ts` (client/tests/collabFractional.test.ts).
    #[test]
    fn known_keys() {
        assert_eq!(key_between(None, None), "V");
        assert_eq!(key_between(Some("V"), None), "l");
        assert_eq!(key_between(None, Some("V")), "G");
        assert_eq!(key_between(Some("V"), Some("W")), "VV");
        assert_eq!(key_between(Some("z"), None), "zV");
        assert_eq!(n_keys_between(None, None, 3), vec!["G", "V", "l"]);
    }
}
