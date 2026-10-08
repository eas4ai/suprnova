//! Nested form data: bracketed and indexed names read as PHP reads them.
//!
//! The Inertia client and HTML forms send nested data under flat names:
//! `user[name]`, `tags[]`, `photos[0]`. PHP's `parse_str` turns such names
//! into nested arrays before Laravel reads them, and Symfony's
//! `HeaderUtils::parseQuery` keeps the part before the first bracket as it
//! was sent. The names here are read the same way, into a tree that the
//! form deserializer walks:
//!
//! - The base, the text before the first `[`, is a name as sent, less its
//!   leading spaces. A name whose base is empty is no variable.
//! - `[key]` names a member and `[]` (or `[ ]`) appends one. A key that is
//!   an integer as PHP writes one (`0`, `12`, `-3`, never `01`) is an index.
//! - `[]` takes the index after the highest index its array holds, `0` in
//!   an array that holds none.
//! - Text after a `]` that no `[` follows is dropped, and a first `[` that
//!   never closes is part of a plain name.
//! - A name nested more than 64 levels deep removes its base and everything
//!   read under it, as PHP's default `max_input_nesting_level` does.
//! - A later name replaces what an earlier one put at the same place, and a
//!   member under a place that holds a value replaces the value with an
//!   array. A name keeps the place in its array where it was first sent.

use std::borrow::Cow;
use std::collections::HashSet;
use std::ops::Range;

use indexmap::IndexMap;
use url::form_urlencoded;

/// How many bracketed levels a name may have, PHP's default
/// `max_input_nesting_level`. It also bounds the depth the deserializer
/// recurses to, whatever the body.
const MAX_DEPTH: usize = 64;

/// The value one name holds.
pub(super) enum Node<'a> {
    /// A value as sent. An empty one is `null`.
    Text(Cow<'a, str>),
    /// The members of bracketed names.
    Array(Box<Array<'a>>),
}

/// The members bracketed names put under one place, as a PHP array holds
/// them: by key, in the order each key was first sent.
#[derive(Default)]
pub(super) struct Array<'a> {
    entries: IndexMap<Key<'a>, Node<'a>>,
    /// The index `[]` takes next: one past the highest index the array has
    /// held, `None` before it held one.
    next: Option<i64>,
}

/// A member's key.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum Key<'a> {
    /// An integer key, from `[]` or a key written as an integer.
    Index(i64),
    /// Any other key.
    Name(Cow<'a, str>),
}

impl<'a> Array<'a> {
    /// Whether every key is an index, which makes the array a list.
    pub(super) fn is_list(&self) -> bool {
        self.entries.keys().all(|key| matches!(key, Key::Index(_)))
    }

    /// The members, in the order their keys were first sent.
    pub(super) fn into_members(self) -> indexmap::map::IntoIter<Key<'a>, Node<'a>> {
        self.entries.into_iter()
    }

    /// The elements of a list in index order, each with its index; `None`
    /// when a key is no index.
    pub(super) fn into_list(self) -> Option<Vec<(i64, Node<'a>)>> {
        let mut items = Vec::with_capacity(self.entries.len());
        for (key, node) in self.entries {
            match key {
                Key::Index(index) => items.push((index, node)),
                Key::Name(_) => return None,
            }
        }
        items.sort_by_key(|(index, _)| *index);
        Some(items)
    }

    /// The key `key` names here: the next index for `[]`, which is `None`
    /// when the array cannot take another (its next index is the highest
    /// an `i64` holds and is taken), as PHP refuses it.
    fn place(&self, key: Option<Cow<'a, str>>) -> Option<Key<'a>> {
        match key {
            Some(text) => Some(key_from(text)),
            None => {
                let index = self.next.unwrap_or(0);
                let key = Key::Index(index);
                (!self.entries.contains_key(&key)).then_some(key)
            }
        }
    }

    /// Count `key` toward the index `[]` takes next.
    fn note(&mut self, key: &Key<'a>) {
        if let Key::Index(index) = *key
            && self.next.is_none_or(|next| index >= next)
        {
            self.next = Some(index.saturating_add(1));
        }
    }

    /// Put `value` under `key`, replacing what the key held.
    fn set(&mut self, key: Option<Cow<'a, str>>, value: Node<'a>) {
        if let Some(key) = self.place(key) {
            self.note(&key);
            self.entries.insert(key, value);
        }
    }

    /// The place under `key`, made an empty array when the key is new.
    fn child(&mut self, key: Option<Cow<'a, str>>) -> Option<&mut Node<'a>> {
        let key = self.place(key)?;
        self.note(&key);
        Some(
            self.entries
                .entry(key)
                .or_insert_with(|| Node::Array(Box::default())),
        )
    }
}

impl<'a> Node<'a> {
    /// The array this place holds, made one when it holds a value.
    fn make_array(&mut self) -> Option<&mut Array<'a>> {
        if !matches!(self, Self::Array(_)) {
            *self = Self::Array(Box::default());
        }
        match self {
            Self::Array(array) => Some(array),
            Self::Text(_) => None,
        }
    }
}

/// Form data read from bracketed names, by top-level name.
pub(super) struct Nested<'a> {
    /// The top-level names a read uses, or `None` for every name.
    tracked: Option<HashSet<&'static str>>,
    /// Each tracked top-level name with its value, in the order it was
    /// first sent.
    pub(super) names: IndexMap<Cow<'a, str>, Node<'a>>,
    /// The first top-level name sent that the read does not use. A struct
    /// ignores it, or refuses it when it denies unknown fields, so one is
    /// all a read needs, however many the client sends.
    pub(super) untracked: Option<Cow<'a, str>>,
}

impl<'a> Nested<'a> {
    /// An empty read that keeps the top-level names in `fields`, or every
    /// name when `fields` is `None`.
    ///
    /// Only the names a struct reads are kept, so a client cannot make the
    /// read hold names the struct never asks for.
    pub(super) fn new(fields: Option<&'static [&'static str]>) -> Self {
        Self {
            tracked: fields.map(|fields| fields.iter().copied().collect()),
            names: IndexMap::new(),
            untracked: None,
        }
    }

    /// The pairs of a url-encoded body or query string.
    pub(super) fn from_urlencoded(
        bytes: &'a [u8],
        fields: Option<&'static [&'static str]>,
    ) -> Self {
        let mut nested = Self::new(fields);
        for (name, value) in form_urlencoded::parse(bytes) {
            nested.insert(name, Node::Text(value));
        }
        nested
    }

    /// Which of `fields` the read holds a value for.
    pub(super) fn present(&self, fields: &'static [&'static str]) -> Vec<&'static str> {
        fields
            .iter()
            .copied()
            .filter(|field| self.names.contains_key(*field))
            .collect()
    }

    fn tracks(&self, name: &str) -> bool {
        self.tracked
            .as_ref()
            .is_none_or(|tracked| tracked.contains(name))
    }

    fn note_untracked(&mut self, name: Cow<'a, str>) {
        if self.untracked.is_none() {
            self.untracked = Some(name);
        }
    }

    /// Put `value` where `name` says.
    pub(super) fn insert(&mut self, name: Cow<'a, str>, value: Node<'a>) {
        match split(name) {
            Split::Dropped => {}
            Split::Plain(name) => {
                if self.tracks(&name) {
                    self.names.insert(name, value);
                } else {
                    self.note_untracked(name);
                }
            }
            // PHP removes the whole variable. Which name takes the removed
            // one's place in the order does not matter to a read, and
            // `swap_remove` keeps a run of such names linear.
            Split::TooDeep(base) => {
                self.names.swap_remove(base.as_ref());
            }
            Split::Nested { base, keys } => {
                if !self.tracks(&base) {
                    self.note_untracked(base);
                    return;
                }
                let place = self
                    .names
                    .entry(base)
                    .or_insert_with(|| Node::Array(Box::default()));
                set_path(place, keys, value);
            }
        }
    }
}

/// Put `value` at the end of `keys`, starting at `place`.
fn set_path<'a>(place: &mut Node<'a>, keys: Vec<Option<Cow<'a, str>>>, value: Node<'a>) {
    let mut place = place;
    let mut keys = keys.into_iter().peekable();
    while let Some(key) = keys.next() {
        let Some(array) = place.make_array() else {
            return;
        };
        if keys.peek().is_none() {
            array.set(key, value);
            return;
        }
        let Some(child) = array.child(key) else {
            return;
        };
        place = child;
    }
}

/// What a name says about where its value goes.
enum Split<'a> {
    /// No variable: the name, or the part before its first bracket, is
    /// empty.
    Dropped,
    /// A top-level name.
    Plain(Cow<'a, str>),
    /// A name nested too deep: its base is removed.
    TooDeep(Cow<'a, str>),
    /// A base and its keys, `None` for `[]`.
    Nested {
        base: Cow<'a, str>,
        keys: Vec<Option<Cow<'a, str>>>,
    },
}

/// Read `name` as PHP's `parse_str` reads a variable name, keeping the base
/// as sent, as Symfony does.
fn split(name: Cow<'_, str>) -> Split<'_> {
    let text: &str = &name;
    let start = text.len() - text.trim_start_matches(' ').len();
    let Some(open) = text[start..].find('[').map(|open| start + open) else {
        return if start == text.len() {
            Split::Dropped
        } else {
            Split::Plain(slice(&name, start..text.len()))
        };
    };
    if open == start {
        return Split::Dropped;
    }
    let bytes = text.as_bytes();
    let mut keys: Vec<Option<Range<usize>>> = Vec::new();
    let mut at = open;
    loop {
        if keys.len() == MAX_DEPTH {
            return Split::TooDeep(slice(&name, start..open));
        }
        let content = at + 1;
        // PHP reads `[ ]` as `[]`.
        let probe = if bytes.get(content) == Some(&b' ') {
            content + 1
        } else {
            content
        };
        if bytes.get(probe) == Some(&b']') {
            keys.push(None);
            at = probe + 1;
        } else {
            match text.get(probe..).and_then(|rest| rest.find(']')) {
                Some(close) => {
                    keys.push(Some(content..probe + close));
                    at = probe + close + 1;
                }
                None if keys.is_empty() => return Split::Plain(unclosed(&name, start, open)),
                None => break,
            }
        }
        if bytes.get(at) != Some(&b'[') {
            break;
        }
    }
    Split::Nested {
        base: slice(&name, start..open),
        keys: keys
            .into_iter()
            .map(|key| key.map(|range| slice(&name, range)))
            .collect(),
    }
}

/// The plain name a first `[` that never closes leaves: PHP writes `_` for
/// each space, `.` and `[` after it, and Symfony gives the bracket back.
fn unclosed<'a>(name: &Cow<'a, str>, start: usize, open: usize) -> Cow<'a, str> {
    let rest = &name[open + 1..];
    if !rest.contains([' ', '.', '[']) {
        return slice(name, start..name.len());
    }
    let mut plain = String::with_capacity(name.len() - start);
    plain.push_str(&name[start..=open]);
    plain.extend(rest.chars().map(|c| match c {
        ' ' | '.' | '[' => '_',
        c => c,
    }));
    Cow::Owned(plain)
}

/// `range` of `name`, borrowed when `name` is.
fn slice<'a>(name: &Cow<'a, str>, range: Range<usize>) -> Cow<'a, str> {
    match name {
        Cow::Borrowed(name) => Cow::Borrowed(&name[range]),
        Cow::Owned(name) => Cow::Owned(name[range].to_string()),
    }
}

/// A key as PHP keeps it: an integer when the text is one as PHP writes an
/// integer, else the text.
fn key_from(text: Cow<'_, str>) -> Key<'_> {
    match php_integer(&text) {
        Some(index) => Key::Index(index),
        None => Key::Name(text),
    }
}

/// `text` as an integer when PHP reads an array key as one: decimal digits
/// with an optional `-`, no leading zero but in `0` itself, no `-0`, and
/// within an `i64`.
fn php_integer(text: &str) -> Option<i64> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let canonical = match digits.as_bytes() {
        [] => false,
        [b'0'] => digits.len() == text.len(),
        [b'0', ..] => false,
        bytes => bytes.iter().all(u8::is_ascii_digit),
    };
    if canonical { text.parse().ok() } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(name: &str) -> Option<(String, Vec<Option<String>>)> {
        match split(Cow::Borrowed(name)) {
            Split::Nested { base, keys } => Some((
                base.into_owned(),
                keys.into_iter()
                    .map(|key| key.map(Cow::into_owned))
                    .collect(),
            )),
            Split::Plain(name) => Some((name.into_owned(), Vec::new())),
            Split::Dropped | Split::TooDeep(_) => None,
        }
    }

    #[test]
    fn inp_a_name_splits_into_its_base_and_keys() {
        let some = |key: &str| Some(key.to_string());
        assert_eq!(keys("tags"), Some(("tags".into(), vec![])));
        assert_eq!(keys("  tags"), Some(("tags".into(), vec![])));
        assert_eq!(keys("a.b c"), Some(("a.b c".into(), vec![])));
        assert_eq!(keys("tags[]"), Some(("tags".into(), vec![None])));
        assert_eq!(keys("tags[ ]"), Some(("tags".into(), vec![None])));
        assert_eq!(
            keys("user[ name][0]"),
            Some(("user".into(), vec![some(" name"), some("0")]))
        );
        assert_eq!(keys("a[b[c]"), Some(("a".into(), vec![some("b[c")])));
        assert_eq!(keys("a[b]c[d]"), Some(("a".into(), vec![some("b")])));
        assert_eq!(keys("a[b][c"), Some(("a".into(), vec![some("b")])));
        assert_eq!(keys("a[b.c d"), Some(("a[b_c_d".into(), vec![])));
        assert_eq!(keys("a[bc"), Some(("a[bc".into(), vec![])));
        assert_eq!(keys("[a]"), None);
        assert_eq!(keys("   "), None);
        assert_eq!(keys(""), None);
    }

    #[test]
    fn inp_a_key_is_an_index_when_php_writes_it_as_an_integer() {
        for (text, index) in [
            ("0", Some(0)),
            ("12", Some(12)),
            ("-3", Some(-3)),
            ("01", None),
            ("-0", None),
            ("-", None),
            ("", None),
            ("1a", None),
            (" 1", None),
            ("9223372036854775807", Some(i64::MAX)),
            ("9223372036854775808", None),
        ] {
            assert_eq!(php_integer(text), index, "{text:?}");
        }
    }

    #[test]
    fn inp_an_append_after_the_highest_index_is_refused() {
        let mut nested = Nested::new(None);
        nested.insert(
            Cow::Borrowed("a[9223372036854775807]"),
            Node::Text(Cow::Borrowed("x")),
        );
        nested.insert(Cow::Borrowed("a[]"), Node::Text(Cow::Borrowed("y")));
        let Some(Node::Array(array)) = nested.names.swap_remove("a") else {
            panic!("`a` is an array");
        };
        let list = array.into_list().expect("a list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, i64::MAX);
    }
}
