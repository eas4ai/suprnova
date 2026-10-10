//! Laravel infrastructure gaps owned by the eloquent suite: the
//! descending sort for `Ord` items and the ordered `map_with_keys`.

use std::cmp::Ordering;

use suprnova::eloquent::Collection;
use suprnova::indexmap::IndexMap;

/// An item whose order reads only its rank, so two items of one rank
/// compare equal while their labels still tell them apart.
#[derive(Debug, Clone)]
struct Ranked {
    rank: u32,
    label: &'static str,
}

impl PartialEq for Ranked {
    fn eq(&self, other: &Self) -> bool {
        self.rank == other.rank
    }
}

impl Eq for Ranked {}

impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ranked {
    fn cmp(&self, other: &Self) -> Ordering {
        self.rank.cmp(&other.rank)
    }
}

fn labels(items: Collection<Ranked>) -> Vec<&'static str> {
    items.into_iter().map(|item| item.label).collect()
}

#[test]
fn sort_desc_orders_greatest_first_and_keeps_equal_items_in_order() {
    let items = Collection::from_vec(vec![
        Ranked {
            rank: 1,
            label: "a",
        },
        Ranked {
            rank: 1,
            label: "b",
        },
        Ranked {
            rank: 2,
            label: "c",
        },
    ]);

    assert_eq!(labels(items.sort_desc()), vec!["c", "a", "b"]);
}

#[test]
fn sort_desc_is_stable_when_every_item_is_equal() {
    let items = Collection::from_vec(vec![
        Ranked {
            rank: 4,
            label: "first",
        },
        Ranked {
            rank: 4,
            label: "second",
        },
        Ranked {
            rank: 4,
            label: "third",
        },
    ]);

    assert_eq!(
        labels(items.sort_desc()),
        vec!["first", "second", "third"],
        "sort-then-reverse would reverse equal items"
    );
}

#[test]
fn sort_desc_on_plain_numbers_and_an_empty_collection() {
    let numbers = Collection::from_vec(vec![3, 1, 4, 1, 5, 9, 2, 6]);
    assert_eq!(numbers.sort_desc().into_vec(), vec![9, 6, 5, 4, 3, 2, 1, 1]);

    let empty: Collection<i32> = Collection::new();
    assert!(empty.sort_desc().is_empty());
}

#[test]
fn sort_with_keeps_its_comparator_meaning() {
    let numbers = Collection::from_vec(vec![2, 3, 1]);
    assert_eq!(numbers.sort_with(|a, b| a.cmp(b)).into_vec(), vec![1, 2, 3]);
}

#[test]
fn map_with_keys_keeps_the_collection_order() {
    let numbers = Collection::from_vec((0..32).collect::<Vec<u32>>());
    let mapped: IndexMap<String, u32> = numbers.map_with_keys(|n| (format!("key-{n}"), n * 10));

    let keys: Vec<String> = mapped.keys().cloned().collect();
    let expected: Vec<String> = (0..32).map(|n| format!("key-{n}")).collect();
    assert_eq!(keys, expected);
    assert_eq!(mapped.get("key-31"), Some(&310));
}

#[test]
fn map_with_keys_keeps_the_first_position_and_the_last_value_of_a_repeated_key() {
    let pairs = Collection::from_vec(vec![("a", 1), ("b", 2), ("a", 3)]);
    let mapped = pairs.map_with_keys(|(key, value)| (key, value));

    let keys: Vec<&str> = mapped.keys().copied().collect();
    assert_eq!(keys, vec!["a", "b"]);
    assert_eq!(mapped.get("a"), Some(&3));
    assert_eq!(mapped.get("b"), Some(&2));
}

#[test]
fn map_with_keys_on_an_empty_collection_is_empty() {
    let empty: Collection<u8> = Collection::new();
    assert!(empty.map_with_keys(|n| (n, n)).is_empty());
}

#[test]
fn map_to_map_keeps_collecting_into_a_hash_map() {
    let words = Collection::from_vec(vec!["a", "bb"]);
    let mapped: std::collections::HashMap<usize, &str> = words.map_to_map(|w| (w.len(), w));
    assert_eq!(mapped.get(&2), Some(&"bb"));
}
