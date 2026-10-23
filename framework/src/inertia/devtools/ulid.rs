//! Entry ids: ULIDs, as Laravel's `Str::ulid()` mints them.
//!
//! The extension sorts and links entries by id, and the store lists them
//! newest first by comparing ids, so an id must sort in the order its
//! entry was recorded. A ULID's first ten characters are its millisecond,
//! and within one millisecond this generator adds one to the random part
//! of the id before it, as Symfony's monotonic `Ulid` does, so two entries
//! recorded in the same millisecond still sort in order.

use std::sync::Mutex;

use rand::RngExt;

use crate::eloquent::unique_id::{encode_ulid_lowercase, is_valid_ulid};

/// The last id minted in this process: its millisecond and its 80 random
/// bits.
static LAST: Mutex<(u64, u128)> = Mutex::new((0, 0));

/// The largest value 80 bits hold.
const RANDOM_MAX: u128 = (1 << 80) - 1;

/// A new id for an entry recorded at `now_ms` milliseconds since the Unix
/// epoch, in upper case as Laravel writes it.
///
/// Later than every id this process minted before: a millisecond that is
/// not past the last one reuses it with the random part one higher, and a
/// random part that would overflow moves on to the next millisecond.
pub(crate) fn new_id(now_ms: u64) -> String {
    let fresh = rand::rng().random::<u128>() & RANDOM_MAX;
    let (ms, random) = {
        let mut last = crate::lock::recover(&LAST);
        let next = if now_ms > last.0 {
            (now_ms, fresh)
        } else if last.1 < RANDOM_MAX {
            (last.0, last.1 + 1)
        } else {
            (last.0 + 1, fresh)
        };
        *last = next;
        next
    };
    encode(ms, random)
}

/// The ULID of millisecond `ms` and random part `random`: 48 bits of the
/// one, then 80 of the other, in Crockford base32, upper case.
fn encode(ms: u64, random: u128) -> String {
    let mut bytes = [0u8; 16];
    let ts = ms & ((1u64 << 48) - 1);
    bytes[..6].copy_from_slice(&ts.to_be_bytes()[2..]);
    bytes[6..].copy_from_slice(&(random & RANDOM_MAX).to_be_bytes()[6..]);
    encode_ulid_lowercase(&bytes).to_ascii_uppercase()
}

/// Whether `value` is a ULID in either case, Laravel's `Str::isUlid`: the
/// entry endpoint answers `404` for anything else without touching the
/// disk, so an id can never name a path outside the store.
pub(crate) fn is_ulid(value: &str) -> bool {
    is_valid_ulid(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indt_ids_are_ulids_and_sort_in_the_order_they_were_minted() {
        let ids: Vec<String> = (0..50).map(|_| new_id(1_700_000_000_000)).collect();
        for id in &ids {
            assert!(is_ulid(id), "{id} is not a ULID");
            assert_eq!(id.len(), 26);
            assert_eq!(id, &id.to_ascii_uppercase(), "ids are upper case");
        }
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted, "ids minted in one millisecond sort in order");
        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "no two ids are the same");
    }

    #[test]
    fn indt_an_id_carries_its_millisecond_then_its_random_part() {
        assert_eq!(encode(1 << 47, 0), "40000000000000000000000000");
        assert_eq!(encode(0, RANDOM_MAX), "0000000000ZZZZZZZZZZZZZZZZ");
    }

    #[test]
    fn indt_is_ulid_refuses_what_is_not_one() {
        assert!(!is_ulid("not-a-ulid"));
        assert!(!is_ulid("../../../../etc/passwd"));
        assert!(!is_ulid("8ZZZZZZZZZZZZZZZZZZZZZZZZZ"), "past 2^128");
        assert!(is_ulid("01ARZ3NDEKTSV4RRFFQ69G5FAV"));
        assert!(is_ulid("01arz3ndektsv4rrffq69g5fav"));
    }
}
