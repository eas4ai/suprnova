//! The record frame's stored format from outside the engine.
//!
//! The record store port itself - opaque bytes, a version that only the
//! state it was read at can replace, store-time expiry, and a promotion key
//! space of its own - is proven by the unit tests over `MemoryRecordStore`
//! in `src/ledger/distributed.rs`, which reach the same reference
//! implementation through the same trait. What only an outside caller can
//! show is that the two numbers a stored record is written and read under
//! are part of the crate's public surface, and what they are.

use suprnova_live::ledger::{MAX_RECORD_BYTES, RECORD_VERSION};

#[test]
fn the_record_frame_is_versioned_and_bounded() {
    assert_eq!(
        RECORD_VERSION, 1,
        "the record version is part of the stored format and changes deliberately"
    );
    assert_eq!(
        MAX_RECORD_BYTES, 32_768,
        "the record bound is part of the stored format and changes deliberately"
    );
}
