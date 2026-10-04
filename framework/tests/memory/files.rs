//! MEM-003 on files. Disks exist only with the framework's `filesystem`
//! feature, and the fake disk only with `testing`, so this test needs both.

#![cfg(all(feature = "filesystem", feature = "testing"))]

use suprnova::{DiskExt, Storage};

use crate::support::{Heap, exclusive};

/// MEM-003: appending to a file copies the file's bytes once at most.
#[tokio::test]
async fn mem_audit_appending_does_not_copy_the_file_twice() {
    let _lock = exclusive().await;
    const SIZE: usize = 16 * 1024 * 1024;
    let _storage = Storage::fake();
    Storage::register_memory("mem-audit");
    let disk = Storage::disk("mem-audit").expect("a disk");
    disk.put("f", vec![b'a'; SIZE]).await.expect("put");
    disk.append("f", "w").await.expect("a warm-up");

    let heap = Heap::start();
    let before = heap.bytes();
    disk.append("f", "x").await.expect("append");
    let used = heap.bytes() - before;
    drop(heap);
    assert!(
        used < (SIZE as u64) * 3 / 2,
        "appending one byte to {SIZE} bytes allocated {used} bytes"
    );
    let contents = disk.get("f").await.expect("get");
    assert_eq!(contents.len(), SIZE + 4);
    assert!(contents.ends_with(b"\nw\nx"));
}
