#![no_main]
use libfuzzer_sys::fuzz_target;
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{KeyId, UnixMillis};
use suprnova_live::render_cache::entry::{EntryLimits, decode, inspect};

fn keys() -> SnapshotKeyRing {
    let active = KeyRecord::new(
        KeyId::parse("fuzz").expect("key id"),
        RootKey::new(vec![1; 32]).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(u64::MAX / 2),
        UnixMillis::new(u64::MAX),
    )
    .expect("key record");
    SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
}

fuzz_target!(|data: &[u8]| {
    let bytes = bytes::Bytes::copy_from_slice(data);
    let limits = EntryLimits::default();
    let _ = inspect(&bytes, &limits);
    let _ = decode(&bytes, &keys(), &limits);
});
