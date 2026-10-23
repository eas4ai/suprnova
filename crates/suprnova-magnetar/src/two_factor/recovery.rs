//! Single-use recovery codes, ported from the deployed flow.
//!
//! Codes are `NNNNNN-NNNNNN` (twelve decimal digits, the Fortify shape),
//! persisted as an encrypted newline-joined blob. Consumption decrypts,
//! locates the code in constant time, and rewrites the blob through the
//! store's compare-and-swap so two concurrent consumes of one code have
//! exactly one winner; the final code leaves `None` behind.

use rand::Rng;
use subtle::{Choice, ConditionallySelectable, ConstantTimeEq};

/// How many codes one enrollment carries.
pub const RECOVERY_CODE_COUNT: usize = 10;

/// Generate `count` fresh recovery codes.
#[must_use]
pub fn generate(count: usize) -> Vec<String> {
    let mut rng = rand::thread_rng();
    (0..count)
        .map(|_| {
            let a: u32 = rng.gen_range(0..1_000_000);
            let b: u32 = rng.gen_range(0..1_000_000);
            format!("{a:06}-{b:06}")
        })
        .collect()
}

/// Locate `code` inside `codes` without short-circuiting.
///
/// Every entry is visited and compared with [`ConstantTimeEq`], folding the
/// result, so run time depends only on `codes.len()` - never on whether or
/// where a match exists. Entries of a different length are a structural
/// reject (codes are a fixed 13-byte shape), not a timing oracle for a
/// same-length attacker.
#[must_use]
pub fn find_constant_time(codes: &[String], code: &str) -> Option<usize> {
    let candidate = code.as_bytes();
    let mut found = Choice::from(0_u8);
    let mut index: u32 = 0;
    for (position, stored) in codes.iter().enumerate() {
        let stored = stored.as_bytes();
        if stored.len() != candidate.len() {
            continue;
        }
        let here = stored.ct_eq(candidate);
        index.conditional_assign(&(position as u32), here & !found);
        found |= here;
    }
    if bool::from(found) {
        Some(index as usize)
    } else {
        None
    }
}
