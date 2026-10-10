//! The query string normalisation Laravel's `Request::fullUrl` applies,
//! through Symfony's `Request::normalizeQueryString` (PAR-056).
//!
//! Symfony parses the query as PHP fills `$_GET` (`HeaderUtils::parseQuery`
//! around `parse_str`), sorts the top-level keys with `ksort`, and writes
//! the result back with `http_build_query(..., PHP_QUERY_RFC3986)`. Laravel's
//! Inertia adapter puts that string in the page object's `url`, so the
//! client's comparisons of page URLs (a reload, a prefetch, history) see the
//! string Laravel sends: `/s?b=2&a=1%20x` becomes `/s?a=1%20x&b=2`.
//!
//! The parse keeps PHP's rules, since they decide the output: a repeated
//! key keeps its last value, a key without `=` gets an empty value
//! (`flag` becomes `flag=`), bracketed keys build nested arrays (`a[]=1`
//! becomes `a%5B0%5D=1`), and `+` decodes to a space that re-encodes as
//! `%20`.

use std::cmp::Ordering;

use indexmap::IndexMap;

/// PHP's default `max_input_vars`: `parse_str` stops reading after this
/// many pairs.
const MAX_INPUT_VARS: usize = 1000;

/// PHP's default `max_input_nesting_level`: a key nested deeper drops its
/// whole top-level entry.
const MAX_INPUT_NESTING_LEVEL: usize = 64;

/// The query part of `path_and_query`, normalised; the path is left as it
/// is.
///
/// The `String` comes back unchanged, the same buffer, when its query is
/// already normalised or there is none, so the page `url` costs no copy in
/// the common case (MEM-003). A query that normalises to nothing (`?` or
/// `?&`) loses its `?`, as Laravel's `fullUrl()` omits it.
pub(crate) fn normalize_path_and_query(mut path_and_query: String) -> String {
    let Some(at) = path_and_query.find('?') else {
        return path_and_query;
    };
    let query = &path_and_query[at + 1..];
    let normalized = normalize_query_string(query);
    if normalized == query {
        if normalized.is_empty() {
            path_and_query.truncate(at);
        }
        return path_and_query;
    }
    path_and_query.truncate(at);
    if !normalized.is_empty() {
        path_and_query.push('?');
        path_and_query.push_str(&normalized);
    }
    path_and_query
}

/// Symfony's `Request::normalizeQueryString`: parse `query` as PHP does,
/// sort its top-level keys, and encode it per RFC 3986.
pub(crate) fn normalize_query_string(query: &str) -> String {
    if query.is_empty() {
        return String::new();
    }
    let mut root = PhpArray::default();
    let mut counted = 0usize;
    for piece in query.split('&') {
        // `HeaderUtils::parseQuery` cuts a piece at a NUL byte.
        let piece = piece.split('\0').next().unwrap_or_default();
        let (raw_key, raw_value) = match piece.split_once('=') {
            Some((key, value)) => (key, Some(value)),
            None => (piece, None),
        };
        // The name is decoded, cut at a NUL, and stripped of leading
        // spaces, as Symfony and `parse_str` both do.
        let mut key = url_decode(raw_key.as_bytes());
        if let Some(nul) = key.iter().position(|byte| *byte == 0) {
            key.truncate(nul);
        }
        let spaces = key.iter().take_while(|byte| **byte == b' ').count();
        key.drain(..spaces);
        // An empty piece never reaches `parse_str`; every other one counts
        // toward `max_input_vars`, a nameless one included.
        if key.is_empty() && raw_value.is_none() {
            continue;
        }
        counted += 1;
        if counted > MAX_INPUT_VARS {
            break;
        }
        let value = raw_value
            .map(|value| url_decode(value.as_bytes()))
            .unwrap_or_default();
        register(&mut root, &key, value);
    }

    let mut entries: Vec<(Key, Node)> = root.entries.into_iter().collect();
    // `ksort` is stable since PHP 8.
    entries.sort_by(|(a, _), (b, _)| compare_keys(a, b));
    let mut out = String::new();
    for (key, node) in &entries {
        build_query(&mut out, None, key, node);
    }
    out
}

/// A PHP array key: an integer, or any other string.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    Int(i64),
    Str(Vec<u8>),
}

impl Key {
    /// The key PHP stores for `bytes`: an integer for a canonical decimal
    /// that fits an `i64` (`"5"`, `"-3"`, not `"05"` or `"-0"`), a string
    /// otherwise, as `ZEND_HANDLE_NUMERIC_STR` decides.
    fn from_bytes(bytes: &[u8]) -> Self {
        integer_key(bytes).map_or_else(|| Self::Str(bytes.to_vec()), Self::Int)
    }

    /// The key as the text PHP compares and encodes.
    fn text(&self) -> std::borrow::Cow<'_, [u8]> {
        match self {
            Self::Int(value) => std::borrow::Cow::Owned(value.to_string().into_bytes()),
            Self::Str(bytes) => std::borrow::Cow::Borrowed(bytes),
        }
    }
}

/// `ZEND_HANDLE_NUMERIC_STR`: whether `bytes` is a canonical decimal
/// integer PHP stores as an integer key.
fn integer_key(bytes: &[u8]) -> Option<i64> {
    let (negative, digits) = match bytes.split_first() {
        Some((b'-', rest)) => (true, rest),
        _ => (false, bytes),
    };
    let leading_zero = digits.first() == Some(&b'0') && bytes.len() > 1;
    if digits.is_empty()
        || leading_zero
        || digits.len() > 19
        || !digits.iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let magnitude = digits.iter().try_fold(0u64, |acc, digit| {
        acc.checked_mul(10)?.checked_add(u64::from(digit - b'0'))
    })?;
    if negative {
        0i64.checked_sub_unsigned(magnitude)
    } else {
        i64::try_from(magnitude).ok()
    }
}

/// A value `parse_str` stores: a string, or a nested array.
#[derive(Debug)]
enum Node {
    Scalar(Vec<u8>),
    Array(PhpArray),
}

/// An ordered PHP array: insertion order, a key updated in place keeps its
/// position, and `[]` appends at the next integer key.
#[derive(Debug, Default)]
struct PhpArray {
    entries: IndexMap<Key, Node>,
    /// `nNextFreeElement`: `None` until an integer key is stored.
    next_index: Option<i64>,
}

impl PhpArray {
    /// Store `node` at `key`, replacing a value already there in place.
    fn update(&mut self, key: Key, node: Node) {
        if let Key::Int(index) = key {
            self.note_index(index);
        }
        self.entries.insert(key, node);
    }

    /// Store `node` at the next integer key. `None` when that key is taken
    /// (the array already holds `i64::MAX`), where PHP drops the value.
    fn append(&mut self, node: Node) -> Option<&mut Node> {
        let index = self.next_index.unwrap_or(0);
        let key = Key::Int(index);
        if self.entries.contains_key(&key) {
            return None;
        }
        self.note_index(index);
        let (at, _) = self.entries.insert_full(key, node);
        self.entries.get_index_mut(at).map(|(_, node)| node)
    }

    fn note_index(&mut self, index: i64) {
        if self.next_index.is_none_or(|next| index >= next) {
            self.next_index = Some(index.saturating_add(1));
        }
    }
}

/// Store one decoded `name` and `value` in `root`, as
/// `php_register_variable_ex` does for `parse_str`.
fn register(root: &mut PhpArray, name: &[u8], value: Vec<u8>) {
    // A nameless pair (`=value`) is no variable.
    if name.is_empty() {
        return;
    }
    let Some(open) = name.iter().position(|byte| *byte == b'[') else {
        root.update(Key::from_bytes(name), Node::Scalar(value));
        return;
    };
    let base = &name[..open];
    if base.is_empty() {
        return;
    }

    // Read the `[index]` chain. `None` is `[]`, an append.
    let mut indexes: Vec<Option<Vec<u8>>> = Vec::new();
    let mut at = open;
    loop {
        if indexes.len() + 1 > MAX_INPUT_NESTING_LEVEL {
            root.entries.shift_remove(&Key::from_bytes(base));
            return;
        }
        let start = at + 1;
        let Some(close) = name[start..]
            .iter()
            .position(|byte| *byte == b']')
            .map(|offset| start + offset)
        else {
            if indexes.is_empty() {
                // An unterminated first bracket makes the whole name a
                // plain key, its `[` and the spaces, dots and brackets
                // after it written as `_` - except that Symfony puts the
                // first `[` back.
                let mut key = base.to_vec();
                key.push(b'[');
                key.extend(name[start..].iter().map(|byte| match byte {
                    b' ' | b'.' | b'[' => b'_',
                    other => *other,
                }));
                root.update(Key::from_bytes(&key), Node::Scalar(value));
                return;
            }
            // A later unterminated bracket ends the chain where it stands.
            break;
        };
        indexes.push((close > start).then(|| name[start..close].to_vec()));
        at = close + 1;
        if name.get(at) != Some(&b'[') {
            break;
        }
    }

    let mut path = Vec::with_capacity(indexes.len() + 1);
    path.push(Some(base.to_vec()));
    path.extend(indexes);
    insert_at(root, &path, value);
}

/// Store `value` under the chain of `path` keys in `array`, `None` being
/// `[]`. Every key but the last holds an array: one is created where the
/// key is missing, and replaces a string where one is there, in place.
/// An append PHP cannot make drops the value.
fn insert_at(array: &mut PhpArray, path: &[Option<Vec<u8>>], value: Vec<u8>) {
    let Some((first, rest)) = path.split_first() else {
        return;
    };
    if rest.is_empty() {
        match first {
            Some(key) => array.update(Key::from_bytes(key), Node::Scalar(value)),
            None => {
                array.append(Node::Scalar(value));
            }
        }
        return;
    }
    let node = match first {
        Some(key) => {
            let key = Key::from_bytes(key);
            if let Key::Int(index) = key {
                array.note_index(index);
            }
            array
                .entries
                .entry(key)
                .or_insert_with(|| Node::Array(PhpArray::default()))
        }
        None => match array.append(Node::Array(PhpArray::default())) {
            Some(node) => node,
            None => return,
        },
    };
    if let Node::Scalar(_) = node {
        *node = Node::Array(PhpArray::default());
    }
    if let Node::Array(nested) = node {
        insert_at(nested, rest, value);
    }
}

/// PHP's `urldecode`: `+` is a space, and `%` with two hex digits is that
/// byte; anything else is kept.
fn url_decode(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        if byte == b'%'
            && let Some(&[high, low]) = bytes.get(at + 1..at + 3)
            && let Some(decoded) = hex_pair(high, low)
        {
            out.push(decoded);
            at += 3;
            continue;
        }
        out.push(if byte == b'+' { b' ' } else { byte });
        at += 1;
    }
    out
}

fn hex_pair(high: u8, low: u8) -> Option<u8> {
    let digit = |byte: u8| {
        (byte as char)
            .to_digit(16)
            .and_then(|d| u8::try_from(d).ok())
    };
    Some(digit(high)? * 16 + digit(low)?)
}

/// PHP's `rawurlencode` (RFC 3986): every byte but `A-Z a-z 0-9 - . _ ~`
/// as `%XX`, upper-case.
fn raw_url_encode(out: &mut String, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(*byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
}

/// `http_build_query` with `PHP_QUERY_RFC3986` for one entry: a string as
/// `key=value`, an array as each of its entries under `key[index]`, the
/// brackets encoded.
fn build_query(out: &mut String, prefix: Option<&str>, key: &Key, node: &Node) {
    let mut name = String::new();
    match prefix {
        Some(prefix) => {
            name.push_str(prefix);
            raw_url_encode(&mut name, &key.text());
        }
        None => raw_url_encode(&mut name, &key.text()),
    }
    match node {
        Node::Scalar(value) => {
            if !out.is_empty() {
                out.push('&');
            }
            out.push_str(&name);
            if prefix.is_some() {
                out.push_str("%5D");
            }
            out.push('=');
            raw_url_encode(out, value);
        }
        Node::Array(array) => {
            let nested = if prefix.is_some() {
                format!("{name}%5D%5B")
            } else {
                format!("{name}%5B")
            };
            for (key, node) in &array.entries {
                build_query(out, Some(&nested), key, node);
            }
        }
    }
}

/// `ksort`'s comparison of two keys (PHP 8): two numeric keys compare as
/// numbers, any other pair as bytes.
fn compare_keys(a: &Key, b: &Key) -> Ordering {
    match (a, b) {
        (Key::Int(a), Key::Int(b)) => a.cmp(b),
        (Key::Int(a), Key::Str(b)) => compare_long_to_string(*a, b),
        (Key::Str(a), Key::Int(b)) => compare_long_to_string(*b, a).reverse(),
        // `zendi_smart_strcmp`.
        (Key::Str(a), Key::Str(b)) => match (numeric(a), numeric(b)) {
            (Some(x), Some(y)) => compare_numbers(x, y).unwrap_or_else(|| a.cmp(b)),
            _ => a.cmp(b),
        },
    }
}

/// `compare_longs_to_string`: an integer key against a string one, as a
/// number when the string is numeric and as bytes otherwise.
fn compare_long_to_string(long: i64, text: &[u8]) -> Ordering {
    match numeric(text) {
        Some(Numeric::Long(value)) => long.cmp(&value),
        Some(Numeric::Double { value, .. }) => three_way(long as f64, value),
        None => long.to_string().as_bytes().cmp(text),
    }
}

/// A numeric string's value as PHP reads it: an integer, or a float with
/// the sign of an integer that overflowed (`oflow`).
#[derive(Clone, Copy, Debug)]
enum Numeric {
    Long(i64),
    Double { value: f64, overflow: i8 },
}

/// `is_numeric_string` without errors allowed: optional leading and
/// trailing whitespace, a sign, digits with an optional fraction and
/// exponent.
fn numeric(bytes: &[u8]) -> Option<Numeric> {
    let is_space = |byte: &u8| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c');
    let start = bytes.iter().position(|byte| !is_space(byte))?;
    let end = bytes.iter().rposition(|byte| !is_space(byte))? + 1;
    let text = std::str::from_utf8(&bytes[start..end]).ok()?;
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(at) => (&unsigned[..at], Some(&unsigned[at + 1..])),
        None => (unsigned, None),
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (mantissa, None),
    };
    let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    let has_digits = !whole.is_empty() || fraction.is_some_and(|f| !f.is_empty());
    if !has_digits || !digits(whole) || !fraction.is_none_or(digits) {
        return None;
    }
    if let Some(exponent) = exponent {
        let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if exponent.is_empty() || !digits(exponent) {
            return None;
        }
    }
    let negative = text.starts_with('-');
    if fraction.is_none() && exponent.is_none() {
        if let Ok(value) = text.trim_start_matches('+').parse::<i64>() {
            return Some(Numeric::Long(value));
        }
        let value: f64 = text.parse().ok()?;
        return Some(Numeric::Double {
            value,
            overflow: if negative { -1 } else { 1 },
        });
    }
    Some(Numeric::Double {
        value: text.parse().ok()?,
        overflow: 0,
    })
}

/// `zendi_smart_strcmp`'s numeric branch; `None` where PHP falls back to
/// comparing bytes (two equal infinities).
fn compare_numbers(a: Numeric, b: Numeric) -> Option<Ordering> {
    match (a, b) {
        (Numeric::Long(a), Numeric::Long(b)) => Some(a.cmp(&b)),
        (Numeric::Long(a), Numeric::Double { value, overflow }) => {
            if overflow != 0 {
                return Some(0.cmp(&overflow));
            }
            Some(three_way(a as f64, value))
        }
        (Numeric::Double { value, overflow }, Numeric::Long(b)) => {
            if overflow != 0 {
                return Some(overflow.cmp(&0));
            }
            Some(three_way(value, b as f64))
        }
        (Numeric::Double { value: a, .. }, Numeric::Double { value: b, .. }) => {
            if a == b && !a.is_finite() {
                return None;
            }
            Some(three_way(a, b))
        }
    }
}

fn three_way(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normalized(query: &str) -> String {
        normalize_query_string(query)
    }

    #[test]
    fn inp_the_query_is_sorted_by_key_and_reencoded_per_rfc_3986() {
        assert_eq!(normalized("b=2&a=1%20x"), "a=1%20x&b=2");
        assert_eq!(normalized("a=%2Fx"), "a=%2Fx");
        assert_eq!(normalized("q=a+b"), "q=a%20b");
        assert_eq!(normalized("q=a b"), "q=a%20b");
        assert_eq!(normalized("q=%7e~"), "q=~~");
        assert_eq!(
            normalized("q=/?:@!$'()*,;"),
            "q=%2F%3F%3A%40%21%24%27%28%29%2A%2C%3B"
        );
        assert_eq!(normalized("q=caf%C3%A9"), "q=caf%C3%A9");
        assert_eq!(normalized("a=1=2"), "a=1%3D2");
    }

    #[test]
    fn inp_php_parse_rules_decide_repeated_and_bare_keys() {
        // The last value wins, in the first value's place.
        assert_eq!(normalized("a=1&b=2&a=3"), "a=3&b=2");
        // A key without `=` is an empty value.
        assert_eq!(normalized("flag&x=1"), "flag=&x=1");
        // Empty pieces and nameless pairs are dropped.
        assert_eq!(normalized("&&x=1&=y&"), "x=1");
        assert_eq!(normalized("&"), "");
        // Leading spaces leave a name; dots and inner spaces stay.
        assert_eq!(normalized("%20a.b%20c=1"), "a.b%20c=1");
    }

    #[test]
    fn inp_brackets_build_nested_arrays() {
        assert_eq!(normalized("a[]=1&a[]=2"), "a%5B0%5D=1&a%5B1%5D=2");
        assert_eq!(
            normalized("a[x]=1&a[y][]=2"),
            "a%5Bx%5D=1&a%5By%5D%5B0%5D=2"
        );
        assert_eq!(normalized("a[5]=x&a[]=y"), "a%5B5%5D=x&a%5B6%5D=y");
        // A later string replaces the array, and a later array the string.
        assert_eq!(normalized("a[x]=1&a=2"), "a=2");
        assert_eq!(normalized("a=2&a[x]=1"), "a%5Bx%5D=1");
        // Text after a closing bracket that opens no new one is ignored.
        assert_eq!(normalized("a[b]c=1"), "a%5Bb%5D=1");
        // An unterminated first bracket is part of a plain key.
        assert_eq!(normalized("a[b.c=1"), "a%5Bb_c=1");
        // A later one ends the chain.
        assert_eq!(normalized("a[b][c=1"), "a%5Bb%5D=1");
        // A nameless bracket is dropped.
        assert_eq!(normalized("[a]=1&x=2"), "x=2");
    }

    #[test]
    fn inp_numeric_keys_sort_as_numbers() {
        assert_eq!(normalized("10=a&2=b&b=c&1.5=d"), "1.5=d&2=b&10=a&b=c");
        assert_eq!(normalized("-1=a&-10=b"), "-10=b&-1=a");
        // `02` stays a string key, and still compares as the number 2.
        assert_eq!(normalized("02=a&1=b"), "1=b&02=a");
    }

    #[test]
    fn inp_parse_stops_at_the_input_limits() {
        let many: Vec<String> = (0..1001).map(|n| format!("k{n:04}=v")).collect();
        let out = normalized(&many.join("&"));
        assert_eq!(out.matches('&').count(), 999, "1000 pairs are read");
        assert!(!out.contains("k1000="));

        let deep = format!("a{}=1&b=2", "[x]".repeat(65));
        assert_eq!(normalized(&deep), "b=2", "too deep drops the key");
        let deep_enough = format!("a{}=1", "[x]".repeat(64));
        assert!(normalized(&deep_enough).starts_with("a%5Bx%5D"));
    }

    #[test]
    fn inp_the_path_keeps_its_buffer_when_nothing_changes() {
        let path = "/page?a=1&b=2".to_string();
        let address = path.as_ptr();
        let same = normalize_path_and_query(path);
        assert_eq!(same, "/page?a=1&b=2");
        assert_eq!(
            same.as_ptr(),
            address,
            "an already normal query is not copied"
        );

        assert_eq!(normalize_path_and_query("/s?b=2&a=1".into()), "/s?a=1&b=2");
        assert_eq!(normalize_path_and_query("/s?".into()), "/s");
        assert_eq!(normalize_path_and_query("/s?&".into()), "/s");
        assert_eq!(normalize_path_and_query("/s".into()), "/s");
    }
}
