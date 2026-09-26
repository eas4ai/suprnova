# Feature map: `manual/filesystem.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 53 checked.

## Rust API: suprnova

### Re-exported from other crates

- [ ] module `suprnova::opendal` re-exports `opendal` (feature: `filesystem`)

### `suprnova::filesystem::disk` (private module; items are public through re-exports)

- [ ] enum `suprnova::ChecksumAlgorithm` · framework/src/filesystem/disk.rs:46 (feature: `filesystem`; also `suprnova::filesystem::ChecksumAlgorithm`)
  - Variants: `Md5`, `Sha1`, `Sha256`
- [ ] trait `suprnova::DiskExt` · framework/src/filesystem/disk.rs:69 (feature: `filesystem`; also `suprnova::filesystem::DiskExt`)
  - Implemented here by: `opendal::Operator`
  - [ ] fn `suprnova::DiskExt::missing` · framework/src/filesystem/disk.rs:75 (required)
  - [ ] fn `suprnova::DiskExt::file_exists` · framework/src/filesystem/disk.rs:82 (required)
  - [ ] fn `suprnova::DiskExt::file_missing` · framework/src/filesystem/disk.rs:85 (required)
  - [ ] fn `suprnova::DiskExt::directory_exists` · framework/src/filesystem/disk.rs:94 (required)
  - [ ] fn `suprnova::DiskExt::directory_missing` · framework/src/filesystem/disk.rs:100 (required)
  - [ ] fn `suprnova::DiskExt::get` · framework/src/filesystem/disk.rs:109 (required)
  - [ ] fn `suprnova::DiskExt::put` · framework/src/filesystem/disk.rs:112 (required)
  - [ ] fn `suprnova::DiskExt::json` · framework/src/filesystem/disk.rs:122 (required)
  - [ ] fn `suprnova::DiskExt::put_json` · framework/src/filesystem/disk.rs:131 (required)
  - [ ] fn `suprnova::DiskExt::prepend` · framework/src/filesystem/disk.rs:140 (required)
  - [ ] fn `suprnova::DiskExt::prepend_with_separator` · framework/src/filesystem/disk.rs:148 (required)
  - [ ] fn `suprnova::DiskExt::append` · framework/src/filesystem/disk.rs:156 (required)
  - [ ] fn `suprnova::DiskExt::append_with_separator` · framework/src/filesystem/disk.rs:164 (required)
  - [ ] fn `suprnova::DiskExt::size` · framework/src/filesystem/disk.rs:175 (required)
  - [ ] fn `suprnova::DiskExt::last_modified` · framework/src/filesystem/disk.rs:179 (required)
  - [ ] fn `suprnova::DiskExt::mime_type` · framework/src/filesystem/disk.rs:192 (required)
  - [ ] fn `suprnova::DiskExt::checksum` · framework/src/filesystem/disk.rs:200 (required)
  - [ ] fn `suprnova::DiskExt::files` · framework/src/filesystem/disk.rs:211 (required)
  - [ ] fn `suprnova::DiskExt::all_files` · framework/src/filesystem/disk.rs:218 (required)
  - [ ] fn `suprnova::DiskExt::directories` · framework/src/filesystem/disk.rs:225 (required)
  - [ ] fn `suprnova::DiskExt::all_directories` · framework/src/filesystem/disk.rs:232 (required)
  - [ ] fn `suprnova::DiskExt::make_directory` · framework/src/filesystem/disk.rs:238 (required)
  - [ ] fn `suprnova::DiskExt::delete_directory` · framework/src/filesystem/disk.rs:243 (required)
  - [ ] fn `suprnova::DiskExt::move_to` · framework/src/filesystem/disk.rs:253 (required)
  - [ ] fn `suprnova::DiskExt::temporary_url` · framework/src/filesystem/disk.rs:266 (required)
  - [ ] fn `suprnova::DiskExt::temporary_upload_url` · framework/src/filesystem/disk.rs:277 (required)

### `suprnova::filesystem::streaming` (feature: `filesystem`)

- [ ] fn `suprnova::copy_between_disks` · framework/src/filesystem/streaming.rs:57 (also `suprnova::filesystem::copy_between_disks`, `suprnova::filesystem::streaming::copy_between_disks`)

### `suprnova::filesystem` (feature: `filesystem`)

- [ ] struct `suprnova::AzBlobConfig` · framework/src/filesystem/mod.rs:177 (feature: `filesystem-azure`, off by default; also `suprnova::filesystem::AzBlobConfig`)
  - Public fields: `container`, `account_name`, `account_key`, `endpoint`, `root`
- [ ] struct `suprnova::GcsConfig` · framework/src/filesystem/mod.rs:223 (feature: `filesystem-gcs`, off by default; also `suprnova::filesystem::GcsConfig`)
  - Public fields: `bucket`, `credential`, `credential_path`, `endpoint`, `root`
- [ ] struct `suprnova::ReadThroughConfig` · framework/src/filesystem/mod.rs:320 (also `suprnova::filesystem::ReadThroughConfig`)
  - Public fields: `primary`, `fallback`, `copy`, `throw_on_promotion_failure`
- [ ] struct `suprnova::S3Config` · framework/src/filesystem/mod.rs:137 (also `suprnova::filesystem::S3Config`)
  - Public fields: `bucket`, `region`, `endpoint`, `access_key_id`, `secret_access_key`, `root`
- [ ] struct `suprnova::Storage` · framework/src/filesystem/mod.rs:124 (also `suprnova::filesystem::Storage`)
  - [ ] fn `suprnova::Storage::disk` · framework/src/filesystem/mod.rs:387
  - [ ] fn `suprnova::Storage::register_fs` · framework/src/filesystem/mod.rs:427
  - [ ] fn `suprnova::Storage::register_fs_with` · framework/src/filesystem/mod.rs:479
  - [ ] fn `suprnova::Storage::register_memory` · framework/src/filesystem/mod.rs:514
  - [ ] fn `suprnova::Storage::register_memory_with` · framework/src/filesystem/mod.rs:537
  - [ ] fn `suprnova::Storage::register_s3` · framework/src/filesystem/mod.rs:560
  - [ ] fn `suprnova::Storage::register_s3_with` · framework/src/filesystem/mod.rs:599
  - [ ] fn `suprnova::Storage::register_azblob` · framework/src/filesystem/mod.rs:648
  - [ ] fn `suprnova::Storage::register_azblob_with` · framework/src/filesystem/mod.rs:664
  - [ ] fn `suprnova::Storage::register_gcs` · framework/src/filesystem/mod.rs:716
  - [ ] fn `suprnova::Storage::register_gcs_with` · framework/src/filesystem/mod.rs:729
  - [ ] fn `suprnova::Storage::register_read_through` · framework/src/filesystem/mod.rs:803
  - [ ] fn `suprnova::Storage::register_read_through_with` · framework/src/filesystem/mod.rs:821
  - [ ] fn `suprnova::Storage::forget` · framework/src/filesystem/mod.rs:874
  - [ ] fn `suprnova::Storage::purge` · framework/src/filesystem/mod.rs:884
  - [ ] fn `suprnova::Storage::disks` · framework/src/filesystem/mod.rs:892
  - [ ] fn `suprnova::Storage::fake` · framework/src/filesystem/mod.rs:908
- [ ] const `suprnova::ATOMIC_STAGING_DIR` · framework/src/filesystem/mod.rs:72 (also `suprnova::filesystem::ATOMIC_STAGING_DIR`)
