# Filesystem & Storage

Suprnova's storage facade gives you a single, named-disk API over local
filesystems, in-memory backends, and the major object stores (S3, Azure Blob,
Google Cloud Storage). Under the hood it is built on
[`opendal`](https://docs.rs/opendal) - but the consumer surface is shaped to
match Laravel's `Storage::disk(...)` calls, so PHP muscle memory translates
straight across.

```rust,no_run
use suprnova::{DiskExt, Storage};

# async fn doc() -> Result<(), suprnova::FrameworkError> {
Storage::register_fs("local", "./storage")?;
let disk = Storage::disk("local")?;

disk.put("notes/hello.txt", b"hello world".to_vec()).await?;
let bytes = disk.get("notes/hello.txt").await?;
assert_eq!(bytes, b"hello world");
# Ok(())
# }
```

## Registering disks

Every disk is registered once at boot via `Storage::register_*` and looked up
by name through `Storage::disk(name)`. There is no "default backend" the
others degrade into - each driver is a peer.

| Constructor                          | Backend                       | Feature             |
|--------------------------------------|-------------------------------|---------------------|
| `Storage::register_fs(name, root)`   | Local filesystem              | `filesystem`        |
| `Storage::register_memory(name)`     | In-process memory (tests)     | `filesystem`        |
| `Storage::register_s3(name, cfg)`    | Amazon S3 or S3-compatible    | `filesystem`        |
| `Storage::register_azblob(name, cfg)`| Azure Blob Storage            | `filesystem-azure`  |
| `Storage::register_gcs(name, cfg)`   | Google Cloud Storage          | `filesystem-gcs`    |
| `Storage::register_read_through(name, cfg)` | Read-through composite | `filesystem` |

`filesystem` is on by default; the Azure and GCS features are not. Turn one
on in your `Cargo.toml`:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.3", features = ["filesystem-gcs"] }
```

Without the feature, `register_azblob` / `register_gcs` and their config
structs do not exist - you get a compile error naming the missing item, not
a runtime failure.

Every constructor has a `_with` variant that hands you the `suprnova::opendal::Operator`
just before it lands in the registry so you can install retry/timeout/logging
layers around it:

```rust,ignore
use std::time::Duration;
use suprnova::opendal::layers::{LoggingLayer, RetryLayer, TimeoutLayer};
use suprnova::Storage;

Storage::register_fs_with("local", "./storage", |op| {
    op.layer(RetryLayer::new().with_max_times(3))
      .layer(TimeoutLayer::new().with_timeout(Duration::from_secs(30)))
      .layer(LoggingLayer::default())
})?;
```

The cloud constructors (`register_s3`, `register_azblob`, `register_gcs`)
apply a `RetryLayer` (3 attempts) by default since transient throttling /
5xx errors are routine on object stores. Use the `_with` variants when you
need full control.

The full set of opendal layers wired in by Suprnova is `RetryLayer`,
`TimeoutLayer`, `LoggingLayer`, `TracingLayer` (bridges to OTel via
`tracing-opentelemetry` when the framework's `otel` feature is on), and
`PrometheusClientLayer` (exports histograms and counters into a
`prometheus_client::registry::Registry` you own). Layer order matters -
the outermost layer wraps everything inside it - and the idiomatic stack
is `RetryLayer → TimeoutLayer → LoggingLayer` so a timed-out attempt
still logs and a retry covers transport failures.

Re-registering the same name replaces the previous operator and emits a
`warn!` log - disks are meant to be registered once at boot, and an
accidental duplicate could swap a production disk for a memory one. The
replacement still happens; the warning just makes the swap audible.

### Why Suprnova diverges

Laravel's `config/filesystems.php` lists every disk driver and you pick one
at runtime; nothing is compiled out. Suprnova gates Azure and GCS behind
features because in Rust the choice has a dependency cost, and this one has
a security dimension: both opendal service crates pull `rsa`, which carries
[RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071) (the
Marvin timing attack) with no fixed release upstream. Making them opt-in
means an app that stores files locally or on S3 never carries that crate.

S3 is deliberately *not* gated - its signer never depended on `rsa`, so
gating it would break the most-used cloud backend and remove nothing.

### Path-traversal guard

Local filesystem disks have a `PathGuardLayer` applied before any user-supplied
layers. A request like `disk.write("../escaped.txt", ..)` is rejected before
it reaches the OS - no `..` component or absolute prefix can escape the disk
root. Object stores and the in-memory backend do not get the guard (a key
like `../foo` is just an ordinary key character on those backends).

After rejecting `..` and absolute components, the guard canonicalizes the
local disk root and the requested on-disk target. Existing targets resolve
every symlink component; for a path that does not exist yet, the guard walks
up to and canonicalizes the nearest existing ancestor. The operation is
rejected if that resolved path lies outside the canonical root, so an in-root
symlink observed during validation cannot redirect a read, write, list, copy,
or rename outside the disk.

This is a canonicalize-then-operate guard, not descriptor-relative filesystem
confinement. It assumes the disk root and its contents are trusted against
concurrent mutation: an attacker who can replace directories or symlinks after
validation but before the backend opens the path may win a time-of-check to
time-of-use race. Use OS-level isolation or a dedicated filesystem when other
principals can mutate the storage tree concurrently.

Streaming writers, listers, and copiers perform this resolved-path check once,
immediately before their first backend I/O. Validation is then fixed for that
stream session so each chunk or item does not block on filesystem
canonicalization. Copier and writer aborts always forward cleanup to their
backends, even before activation or when validation can no longer complete.

## The Laravel-shape disk surface

`Storage::disk(name)` returns a `suprnova::opendal::Operator` directly so you
can use its full streaming surface (`writer`, `reader`, `presign_read`, `list`,
`stat`, ...). On top of that, the [`DiskExt`] trait - blanket-implemented on
`Operator` and re-exported as `suprnova::DiskExt` - adds every Laravel
convenience method you'd reach for through `Storage::disk('local')->...`.

Bring it into scope with `use suprnova::DiskExt;`.

### Existence checks

```rust,ignore
disk.exists("a.txt").await?;        // raw opendal
disk.missing("a.txt").await?;       // negation
disk.file_exists("a.txt").await?;   // file only (not a directory)
disk.file_missing("a.txt").await?;
disk.directory_exists("dir/").await?;
disk.directory_missing("dir/").await?;
```

### Reading and writing

| Laravel name | Rust-native equivalent | Note |
|--------------|------------------------|------|
| `get(path)`  | `read(path)`           | `get` returns `Vec<u8>`; `read` returns opendal's `Buffer`. |
| `put(path, contents)` | `write(path, contents)` | Both accept any `Into<Bytes>`. |
| `json::<T>(path)` | - | Reads + deserializes via serde_json. |
| `put_json(path, &value)` | - | Pretty-prints via serde_json. |
| `prepend(path, data)` | - | Joins with `\n`. Use `prepend_with_separator` for a custom join. |
| `append(path, data)`  | - | Joins with `\n`. Use `append_with_separator` for a custom join. |

`prepend` and `append` create the file if it does not yet exist, so they are
safe as the first write to a log file.

### Metadata

```rust,ignore
let bytes  = disk.size("a.bin").await?;          // u64
let when   = disk.last_modified("a.bin").await?; // Option<DateTime<Utc>>
let mime   = disk.mime_type("a.bin").await?;     // Option<String>
let digest = disk.checksum("a.bin", ChecksumAlgorithm::Sha256).await?;
```

`mime_type` first asks the backend - S3, Azure, and GCS pass the stored
`Content-Type` through. If the backend does not have one, it sniffs the first
16 KiB via the `infer` crate. `Ok(None)` is reserved for unrecognised binary
blobs.

`checksum` supports `Md5`, `Sha1`, and `Sha256` via [`ChecksumAlgorithm`].
MD5 and SHA-1 are included for parity with Laravel and object-store ETags;
choose SHA-256 for any new integrity check.

### Listing

```rust,ignore
let files = disk.files("docs", false).await?;     // top-level files
let all   = disk.all_files("docs").await?;        // recursive
let dirs  = disk.directories("docs", false).await?;
let all   = disk.all_directories("docs").await?;
```

All four return sorted `Vec<String>` so callers can rely on stable ordering
across backends. Directories are filtered out of `files`, and vice versa.
Directory paths are returned **without** a trailing slash (`"docs/sub"`) to
match Laravel's `Storage::directories()` output - opendal's underlying
`list` reports `"docs/sub/"` but we strip the slash for parity.

### Mutating directories and files

| Laravel name           | opendal native        |
|------------------------|-----------------------|
| `make_directory(path)` | `create_dir(path)`    |
| `delete_directory(p)`  | `delete_with(p).recursive(true)` |
| `move_to(from, to)`    | `rename(from, to)`    |

`move_to` falls back to `copy + delete` if the backend doesn't support
rename, and to `read + write + delete` if it doesn't support copy either -
so it works against the in-memory driver used in tests as well as against
production backends.

### Pre-signed URLs

```rust,ignore
let read_url   = disk.temporary_url("uploads/a.pdf", Duration::from_secs(900)).await?;
let upload_url = disk.temporary_upload_url("uploads/new.pdf", Duration::from_secs(900)).await?;
```

`temporary_url` and `temporary_upload_url` return the URL as a `String` for
Laravel parity. They are backed by `Operator::presign_read` /
`presign_write`, so they error with an `Unsupported` message on backends
that do not implement presigning (the in-memory and local-filesystem
drivers fall in this bucket; S3, Azure Blob, and GCS support it).

## Cross-disk streaming copy

`copy_between_disks(src, src_path, dest, dest_path)` streams the source
object into the destination in 64 KiB chunks, regardless of the backend
pair. Source and destination can be backed by *any* opendal driver - local
filesystem to S3, S3 to Azure Blob, in-memory to GCS, and so on.

```rust,ignore
use suprnova::filesystem::streaming::copy_between_disks;

Storage::register_fs("local", "./storage")?;
Storage::register_memory("scratch");
let bytes = copy_between_disks("local", "uploads/big.bin", "scratch", "big.bin").await?;
```

If any step fails mid-copy, the partial destination object is aborted and
deleted before the original error propagates - a failed copy is never
observable as a truncated destination.

## Read-through disks

A read-through disk pairs a fast *primary* with a slower *fallback* and moves
objects from the second to the first as they are read. Point the primary at
the store you are migrating to and the fallback at the one you are migrating
from, and the working set crosses over under real traffic - no maintenance
window, no bulk copy of objects nobody asks for.

```rust,ignore
use suprnova::{ReadThroughConfig, S3Config, Storage};

Storage::register_s3("new-store", S3Config { bucket: "assets-2".into(), ..Default::default() })?;
Storage::register_s3("legacy-store", S3Config { bucket: "assets-1".into(), ..Default::default() })?;

Storage::register_read_through(
    "assets",
    ReadThroughConfig {
        primary: "new-store".into(),
        fallback: "legacy-store".into(),
        ..Default::default()
    },
)?;

let assets = Storage::disk("assets")?;
// Reads `logo.png` from `legacy-store` and writes it to `new-store` on the way
// out. Every later read is served by `new-store`.
let bytes = assets.read("logo.png").await?;
```

`Storage::disk("assets")` returns an ordinary `Operator`, so every method on
it and every `DiskExt` convenience works unchanged.

### Which disk answers which operation

| Operation | Disk |
|---|---|
| `read` | Primary if it holds the object, otherwise the fallback - and the fallback hit is promoted |
| `exists`, `size`, `last_modified`, `mime_type`, `stat` | Primary if it holds the object, otherwise the fallback |
| `write`, `make_directory` | Primary only |
| `files`, `directories`, `list` | Primary only - fallback entries are invisible to a listing |
| `delete` | Both, fallback first |
| `temporary_url` | Primary if it holds the object, otherwise the fallback |
| `temporary_upload_url` | Primary only - an upload has to land where writes land |

Listing is primary-only by design. A union listing would have to reconcile
paging and ordering across two backends, and it would report objects that a
later listing no longer returns once they are promoted. Use
`Storage::disk("legacy-store")` directly when you need to enumerate what is
left on the fallback.

Delete removes the object from both disks. If it only removed the primary
copy, the next read would promote the fallback copy straight back. The
consequence is that a read-through disk over a read-only fallback cannot
delete: the fallback delete fails and the error reaches you.

### When a promotion fails

By default a promotion failure is logged at `warn` and swallowed. You still
receive the bytes you asked for; the disk simply degrades to reading the
fallback every time until the primary is writable again. Set
`throw_on_promotion_failure: true` when a silent loss of promotion would hide
a fault you need to see - a migration you are trying to finish, for instance:

```rust,ignore
Storage::register_read_through(
    "assets",
    ReadThroughConfig {
        primary: "new-store".into(),
        fallback: "legacy-store".into(),
        throw_on_promotion_failure: true,
    },
)?;
```

Registration rejects a configuration that cannot work: an empty `primary` or
`fallback`, a pair that names the same disk twice, a disk that names itself,
or a name that is not registered. Each returns a `FrameworkError` naming the
problem, and no disk is registered.

### Versioned and conditional reads

A read that carries a version or an `If-Match`, `If-None-Match`,
`If-Modified-Since`, or `If-Unmodified-Since` condition is passed on with that
condition intact, so the answer means what you asked it to mean. Such a read
is served but never promoted: writing an old version or a validator-matched
body to the primary would publish it as the live object, and every later
plain read would get it.

Which disk answers one is decided the usual way. The first probe is an
ordinary existence check, so a read-through disk delegates a versioned or
conditional read to the primary whenever the primary holds the path at all;
it reaches the fallback only when the primary does not.

The primary also decides which of these a read-through disk accepts at all,
because the primary's reader is opened first. A versioned read against a
read-through disk whose primary is a local directory is rejected before it
reaches the fallback, since a local directory has no versions.

### Why Suprnova diverges

Laravel builds a read-through disk from a `config/filesystems.php` entry whose
`primary` and `fallback` keys accept either a disk name or an inline driver
config. Suprnova takes disk names only, because disks here are registered by
typed constructors rather than described by arrays - register the inner disk
first, then name it.

Laravel's promotion re-checks the primary after reading the fallback, which
makes a concurrent writer win. Suprnova keeps that check and publishes the
promotion atomically, which Laravel does not. On a local-filesystem primary
the bytes are staged at a temporary sibling and renamed into place; writing
them straight to the target would leave a growing, half-written file visible
for the length of the write, and a read-through disk routes readers by
exactly that existence check. On a primary without a rename - in-memory, S3,
Azure Blob, GCS - a write is already a single indivisible publish, so the
promotion writes the target directly, conditional on the object not already
existing so two concurrent readers do not both promote.

That condition is the part a staged promotion cannot have: the staging path is
unique, so a no-clobber condition on it would be vacuous, and the target is
published by a rename that overwrites. A read-through disk on a
local-filesystem primary therefore trades it away - a write that lands on the
primary in the moment between the promotion's last existence check and its
rename is overwritten by the promoted copy. On a primary without a rename the
condition holds and no such window exists.

The staging object is a real entry on the primary while it lasts, so a listing
taken mid-promotion can show a `.suprnova-promote-<id>.tmp` sibling. A read
that completes, fails, or gives up removes its own sibling, but nothing sweeps
one left by a process that crashed or a read future that was cancelled
mid-promotion: those have to be removed by hand.

A read that resolves from the fallback holds the object in memory until the
promotion write completes, because promotion needs the whole object. That
suits the tiering case a read-through disk is for. For very large cold
objects, read the fallback disk directly or use
[`copy_between_disks`](#cross-disk-streaming-copy) instead.

## Registry hygiene

```rust,ignore
let removed = Storage::forget("local");  // bool: was it present?
Storage::purge();                        // drop every disk
let names = Storage::disks();            // Vec<String>, sorted
```

These mirror Laravel's `FilesystemManager::forgetDisk` / `purge` and are
useful for configuration reloads and admin dashboards. They are not
test-only: production code occasionally needs to drop and re-register a
disk at runtime (e.g. after a secrets rotation).

## Testing

`Storage::fake()` returns a guard that:

1. Acquires a process-global mutex so concurrent `#[tokio::test]` cases do
   not race on the shared registry, and
2. Resets the registry on construction and on drop, leaving the suite in a
   clean state for whichever test runs next.

A `"default"` memory disk is pre-registered for convenience.

```rust,ignore
use suprnova::filesystem::testing::DiskAssertExt;
use suprnova::{DiskExt, Storage};

#[tokio::test]
async fn stores_and_asserts() {
    let _guard = Storage::fake();
    Storage::register_memory("uploads");
    let disk = Storage::disk("uploads").unwrap();

    disk.put("a.txt", b"hello".to_vec()).await.unwrap();

    disk.assert_exists("a.txt").await;
    disk.assert_contents("a.txt", b"hello").await;
    disk.assert_missing("not-here.txt").await;
    disk.assert_count("", 1, false).await;
    disk.assert_directory_empty("docs/").await;
}
```

The five assertion helpers - `assert_exists`, `assert_contents`,
`assert_missing`, `assert_count`, `assert_directory_empty` - are exposed via
the [`DiskAssertExt`] trait, gated on `#[cfg(any(test, feature = "testing"))]`
so production code cannot reach for them.

## Parity quick reference

| Laravel `Storage::disk(...)->...`     | Suprnova                                                 |
|---------------------------------------|----------------------------------------------------------|
| `exists($path)`                       | `disk.exists(path)`                                      |
| `missing($path)`                      | `disk.missing(path)`                                     |
| `fileExists($path)` / `fileMissing`   | `disk.file_exists(path)` / `file_missing(path)`          |
| `directoryExists($p)` / `directoryMissing` | `disk.directory_exists(p)` / `directory_missing(p)` |
| `get($path)`                          | `disk.get(path)` (`Vec<u8>`)                             |
| `json($path)`                         | `disk.json::<T>(path)`                                   |
| `put($path, $contents)`               | `disk.put(path, bytes)`                                  |
| `prepend($path, $data)`               | `disk.prepend(path, data)`                               |
| `append($path, $data)`                | `disk.append(path, data)`                                |
| `size($path)`                         | `disk.size(path)`                                        |
| `lastModified($path)`                 | `disk.last_modified(path)`                               |
| `mimeType($path)`                     | `disk.mime_type(path)`                                   |
| `checksum($path, ['checksum_algo' => 'sha256'])` | `disk.checksum(path, ChecksumAlgorithm::Sha256)` |
| `files($dir, $recursive)`             | `disk.files(dir, recursive)`                             |
| `allFiles($dir)`                      | `disk.all_files(dir)`                                    |
| `directories($dir, $recursive)`       | `disk.directories(dir, recursive)`                       |
| `allDirectories($dir)`                | `disk.all_directories(dir)`                              |
| `makeDirectory($path)`                | `disk.make_directory(path)`                              |
| `deleteDirectory($path)`              | `disk.delete_directory(path)`                            |
| `move($from, $to)`                    | `disk.move_to(from, to)` (or opendal-native `rename`)    |
| `copy($from, $to)`                    | `disk.copy(from, to)` (opendal-native)                   |
| `delete($path)`                       | `disk.delete(path)` (opendal-native)                     |
| `temporaryUrl($path, $expiry)`        | `disk.temporary_url(path, expire)` (or opendal-native `presign_read`) |
| `temporaryUploadUrl($path, $expiry)`  | `disk.temporary_upload_url(path, expire)` (or opendal-native `presign_write`) |
| `Storage::fake()`                     | `Storage::fake()`                                        |
| `Storage::disk()->assertExists()`     | `disk.assert_exists(path).await`                         |
| `FilesystemManager::forgetDisk($n)`   | `Storage::forget(name)`                                  |
| `FilesystemManager::purge()`          | `Storage::purge()`                                       |

## Configuration

Storage configuration lives entirely in Rust code, not in `.env`. Disks
are registered by name in `bootstrap()` via `Storage::register_*` and
addressed by name at the call site (`Storage::disk("public")`). There is
no `FILESYSTEM_DISK` env var the framework reads and no implicit default
disk - each driver is a peer. Apps decide which disk name a given upload
or download targets, and pass any URLs / keys / credentials the chosen
driver needs as their own env vars.

See [Configuration](configuration.md) for the wider rule on where the
framework reads from the environment versus where it expects code-side
registration.

## Next

- [Configuration](configuration.md) - what the framework reads from
  `.env` (and why storage isn't on that list)
- [Requests](requests.md) - file uploads land on a disk via
  `UploadedFile::store_as`
- [Responses](responses.md) - streaming bytes back out of a disk
- [Cache](cache.md) - the other named-driver registry, same shape
- [Testing](testing.md) - the wider fake-everything testing surface

[`DiskExt`]: https://docs.rs/suprnova/latest/suprnova/trait.DiskExt.html
[`DiskAssertExt`]: https://docs.rs/suprnova/latest/suprnova/filesystem/testing/trait.DiskAssertExt.html
[`ChecksumAlgorithm`]: https://docs.rs/suprnova/latest/suprnova/enum.ChecksumAlgorithm.html
