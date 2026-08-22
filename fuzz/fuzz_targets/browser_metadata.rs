#![no_main]

use libfuzzer_sys::fuzz_target;
use suprnova_live::identity::{ComponentName, IslandSlot};
use suprnova_live::mount::{DocumentMountKey, MountFlags};
use suprnova_live::view::{MountMetadata, MountSnapshotKind};

fuzz_target!(|data: &[u8]| {
    if data.len() > 4_096 {
        return;
    }
    let first = data.len() / 3;
    let second = first.saturating_mul(2);
    let document_key = String::from_utf8_lossy(&data[..first]);
    let flag_name = String::from_utf8_lossy(&data[first..second]);
    let flag_value = String::from_utf8_lossy(&data[second..]);
    let _ = DocumentMountKey::parse(&document_key);
    let _ = MountFlags::new([(flag_name.into_owned(), flag_value.into_owned())]);

    let Ok(slot) = IslandSlot::parse("fuzz-root") else {
        return;
    };
    let Ok(component) = ComponentName::parse("fuzz.component") else {
        return;
    };
    if let Ok(metadata) = MountMetadata::new(
        slot,
        component,
        MountSnapshotKind::Instance,
        data.to_vec().into(),
    ) {
        assert_eq!(format!("{metadata:?}"), "<MountMetadata:redacted>");
        assert_eq!(metadata.signed_snapshot(), data);
    }
});
