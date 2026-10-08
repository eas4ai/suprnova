//! Where entries are kept: Laravel's `EntriesRepository` and the circuit
//! breaker of its `EntryStore`.
//!
//! One JSON file per entry, `<id>.json`, written to a temporary file and
//! renamed into place so a reader never sees half an entry. Beside them
//! `_meta.json` holds every entry's `__meta`, newest first, so the list
//! endpoint reads one file instead of all of them; it is rewritten under an
//! exclusive file lock, since every request of every process writes it,
//! and rebuilt from the entry files when it is missing or unreadable.
//! `_last_prune` holds the second of the last prune, and a `.gitignore`
//! keeps the directory out of the application's repository.
//!
//! Every function here does blocking file I/O; the middleware calls them
//! on the blocking pool.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

use super::ulid::is_ulid;

/// The index of every entry's metadata.
const INDEX_FILE: &str = "_meta.json";

/// The second the store was last pruned.
const LAST_PRUNE_FILE: &str = "_last_prune";

/// How long one write failure keeps recording off, Laravel's
/// `EntryStore::SUPPRESS_SECONDS`.
pub(crate) const SUPPRESS_MS: i64 = 30_000;

/// The entry files and index under one directory.
#[derive(Debug, Clone)]
pub(crate) struct EntriesRepository {
    path: PathBuf,
}

impl EntriesRepository {
    /// The repository at `path`. Nothing is created until the first save.
    pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The directory the entries live in.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Store `entry` under `id` and list its `__meta` in the index.
    ///
    /// # Errors
    ///
    /// An id that is not a ULID, which would name a file outside the
    /// directory, and any failure to create the directory or write a file.
    pub(crate) fn save(&self, id: &str, entry: &Value) -> io::Result<()> {
        if !is_ulid(id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid Inertia DevTools entry id",
            ));
        }
        let encoded = serde_json::to_vec(entry).map_err(io::Error::other)?;
        self.ensure_directory()?;
        let target = self.entry_path(id);
        let temp = self.path.join(format!(".{id}.json.tmp"));
        fs::write(&temp, &encoded)?;
        fs::rename(&temp, &target)?;
        let meta = normalize_meta(entry.get("__meta").cloned().unwrap_or(Value::Null));
        self.mutate_index(|index| {
            index.retain(|existing| meta_id(existing) != Some(id));
            index.push(meta);
        })
    }

    /// The entry stored under `id`, or `None` for an id that is not a ULID,
    /// names no entry, or names a file that is not JSON.
    #[cfg(test)]
    pub(crate) fn get(&self, id: &str) -> Option<Value> {
        if !is_ulid(id) {
            return None;
        }
        let bytes = fs::read(self.entry_path(id)).ok()?;
        serde_json::from_slice::<Value>(&bytes)
            .ok()
            .filter(Value::is_object)
    }

    /// Every entry's metadata, newest first.
    #[cfg(test)]
    pub(crate) fn all(&self) -> Vec<Value> {
        let mut index = self.read_index();
        sort_newest_first(&mut index);
        index
    }

    /// Delete every entry recorded before `cutoff`, in seconds since the
    /// Unix epoch.
    ///
    /// # Errors
    ///
    /// A failure to rewrite the index.
    pub(crate) fn prune(&self, cutoff: f64) -> io::Result<()> {
        if !self.path.is_dir() {
            return Ok(());
        }
        let expired: Vec<String> = self
            .read_index()
            .iter()
            .filter(|meta| meta_utime(meta) < cutoff)
            .filter_map(|meta| meta_id(meta).map(str::to_string))
            .collect();
        self.delete_entries(&expired)
    }

    /// Prune entries older than `ttl_hours` when the last prune is at least
    /// `interval_secs` before `now_secs`, or always when the interval is
    /// `0`, and note the prune.
    ///
    /// # Errors
    ///
    /// A failure to create the directory, prune, or note the prune.
    pub(crate) fn prune_if_due(
        &self,
        now_secs: i64,
        interval_secs: u64,
        ttl_hours: u64,
    ) -> io::Result<()> {
        let cutoff = now_secs as f64 - (ttl_hours as f64 * 3600.0);
        if interval_secs == 0 {
            return self.prune(cutoff);
        }
        self.ensure_directory()?;
        if let Some(last) = self.read_last_prune() {
            let since = now_secs.saturating_sub(last);
            if since >= 0 && (since as u64) < interval_secs {
                return Ok(());
            }
        }
        self.prune(cutoff)?;
        fs::write(self.path.join(LAST_PRUNE_FILE), now_secs.to_string())
    }

    /// Keep the newest `limit` entries of the tab `tab`, deleting the
    /// older ones. `0` keeps every entry.
    ///
    /// # Errors
    ///
    /// A failure to rewrite the index.
    pub(crate) fn enforce_tab_limit(&self, tab: &str, limit: usize) -> io::Result<()> {
        if limit == 0 || !self.path.is_dir() {
            return Ok(());
        }
        let mut of_tab: Vec<Value> = self
            .read_index()
            .into_iter()
            .filter(|meta| meta.get("tabUuid").and_then(Value::as_str) == Some(tab))
            .collect();
        sort_newest_first(&mut of_tab);
        let drop: Vec<String> = of_tab
            .iter()
            .skip(limit)
            .filter_map(|meta| meta_id(meta).map(str::to_string))
            .collect();
        self.delete_entries(&drop)
    }

    /// The file of the entry `id`.
    fn entry_path(&self, id: &str) -> PathBuf {
        self.path.join(format!("{id}.json"))
    }

    /// Create the directory, readable by its owner only, with a
    /// `.gitignore` that ignores everything in it.
    fn ensure_directory(&self) -> io::Result<()> {
        if !self.path.is_dir() {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&self.path)?;
        }
        let gitignore = self.path.join(".gitignore");
        if !gitignore.exists() {
            fs::write(gitignore, "*\n")?;
        }
        Ok(())
    }

    /// The second noted by the last prune, if one is noted.
    fn read_last_prune(&self) -> Option<i64> {
        fs::read_to_string(self.path.join(LAST_PRUNE_FILE))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// The index as stored, or rebuilt from the entry files when it is
    /// missing or not a JSON list.
    fn read_index(&self) -> Vec<Value> {
        let index_path = self.path.join(INDEX_FILE);
        let read = File::open(&index_path).and_then(|mut file| {
            file.lock_shared()?;
            let mut contents = String::new();
            file.read_to_string(&mut contents)?;
            Ok(contents)
        });
        match read
            .ok()
            .and_then(|contents| serde_json::from_str::<Value>(&contents).ok())
        {
            Some(Value::Array(index)) => index.into_iter().map(normalize_meta).collect(),
            _ => {
                let rebuilt = self.meta_from_files();
                if !rebuilt.is_empty() {
                    let snapshot = rebuilt.clone();
                    let _ = self.mutate_index(move |index| *index = snapshot);
                }
                rebuilt
            }
        }
    }

    /// The `__meta` of every entry file in the directory.
    fn meta_from_files(&self) -> Vec<Value> {
        let Ok(dir) = fs::read_dir(&self.path) else {
            return Vec::new();
        };
        let mut index: Vec<Value> = dir
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().is_some_and(|ext| ext == "json")
                    && path.file_name().is_some_and(|name| name != INDEX_FILE)
            })
            .filter_map(|path| fs::read(path).ok())
            .filter_map(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .filter_map(|entry| entry.get("__meta").cloned())
            .filter(|meta| meta_id(meta).is_some_and(|id| !id.is_empty()))
            .map(normalize_meta)
            .collect();
        sort_newest_first(&mut index);
        index
    }

    /// Apply `change` to the index under an exclusive lock on the index
    /// file, then write it back newest first. An index that does not read
    /// as a JSON list is rebuilt from the entry files first, so a rewrite
    /// never drops the metadata of the entries already stored.
    fn mutate_index(&self, change: impl FnOnce(&mut Vec<Value>)) -> io::Result<()> {
        self.ensure_directory()?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.path.join(INDEX_FILE))?;
        file.lock()?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        let mut index = if contents.trim().is_empty() {
            Vec::new()
        } else {
            match serde_json::from_str::<Value>(&contents) {
                Ok(Value::Array(index)) => index,
                _ => self.meta_from_files(),
            }
        };
        change(&mut index);
        sort_newest_first(&mut index);
        let encoded = serde_json::to_vec(&index).map_err(io::Error::other)?;
        file.seek(SeekFrom::Start(0))?;
        file.set_len(0)?;
        file.write_all(&encoded)?;
        file.flush()
    }

    /// Delete the files of `ids` and drop them from the index in one
    /// rewrite.
    fn delete_entries(&self, ids: &[String]) -> io::Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        for id in ids {
            if is_ulid(id) {
                match fs::remove_file(self.entry_path(id)) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
        self.mutate_index(|index| {
            index.retain(|meta| meta_id(meta).is_none_or(|id| !ids.iter().any(|gone| gone == id)));
        })
    }
}

/// The `id` of an index record.
fn meta_id(meta: &Value) -> Option<&str> {
    meta.get("id").and_then(Value::as_str)
}

/// The `utime` of an index record, `0` when it has none.
fn meta_utime(meta: &Value) -> f64 {
    meta.get("utime").and_then(Value::as_f64).unwrap_or(0.0)
}

/// An index record with the fields the store relies on in their types,
/// Laravel's `normalizeIndexMeta`: an empty `tabUuid` is `null`.
fn normalize_meta(meta: Value) -> Value {
    let mut meta = match meta {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    let blank_tab = meta
        .get("tabUuid")
        .is_some_and(|tab| tab.as_str().is_none_or(str::is_empty));
    if blank_tab {
        meta.insert("tabUuid".to_string(), Value::Null);
    }
    Value::Object(meta)
}

/// Sort index records newest first: by id, which sorts by the moment the
/// entry was recorded.
fn sort_newest_first(index: &mut [Value]) {
    index.sort_by(|a, b| meta_id(b).cmp(&meta_id(a)));
}

/// The breaker of every store this process wrote to: per directory, the
/// millisecond recording resumes, and whether a failure has been reported
/// since the last successful write.
static BREAKER: Mutex<Option<HashMap<PathBuf, Breaker>>> = Mutex::new(None);

/// One directory's breaker, Laravel's `EntryStore::$suppressedUntil`.
#[derive(Debug, Clone, Copy)]
struct Breaker {
    /// Recording stays off until this millisecond.
    until_ms: i64,
}

/// Whether a write failure to `path` keeps recording off at `now_ms`.
pub(crate) fn is_suppressed(path: &Path, now_ms: i64) -> bool {
    let breakers = crate::lock::recover(&BREAKER);
    breakers
        .as_ref()
        .and_then(|map| map.get(path))
        .is_some_and(|breaker| now_ms < breaker.until_ms)
}

/// Note a write failure to `path` at `now_ms`: recording to it stays off
/// for 30 seconds. `true` when the failure is the first since the last
/// successful write, the one failure that is logged.
pub(crate) fn note_failure(path: &Path, now_ms: i64) -> bool {
    let mut breakers = crate::lock::recover(&BREAKER);
    let map = breakers.get_or_insert_with(HashMap::new);
    let first = !map.contains_key(path);
    map.insert(
        path.to_path_buf(),
        Breaker {
            until_ms: now_ms.saturating_add(SUPPRESS_MS),
        },
    );
    first
}

/// Note a successful write to `path`: the next failure is logged again.
pub(crate) fn note_success(path: &Path) {
    let mut breakers = crate::lock::recover(&BREAKER);
    if let Some(map) = breakers.as_mut() {
        map.remove(path);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::inertia::devtools::ulid::new_id;

    fn entry(id: &str, tab: Option<&str>, utime: f64) -> Value {
        json!({
            "__meta": {"id": id, "tabUuid": tab, "utime": utime, "component": "Home", "requestType": "navigate"},
            "http": {},
            "props": {},
        })
    }

    #[test]
    fn indt_an_entry_is_one_json_file_listed_in_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let repo = EntriesRepository::new(dir.path().join("devtools"));
        let first = new_id(1_000);
        let second = new_id(2_000);
        repo.save(&first, &entry(&first, Some("tab"), 1.0)).unwrap();
        repo.save(&second, &entry(&second, None, 2.0)).unwrap();

        let file = dir.path().join("devtools").join(format!("{first}.json"));
        let stored: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        assert_eq!(stored["__meta"]["id"], first.as_str());
        assert_eq!(repo.get(&first).unwrap()["__meta"]["component"], "Home");

        let ids: Vec<String> = repo
            .all()
            .iter()
            .map(|meta| meta["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, vec![second.clone(), first.clone()], "newest first");
        let index: Value =
            serde_json::from_slice(&fs::read(dir.path().join("devtools/_meta.json")).unwrap())
                .unwrap();
        assert_eq!(index[0]["id"], second.as_str());
        assert_eq!(
            fs::read_to_string(dir.path().join("devtools/.gitignore")).unwrap(),
            "*\n"
        );
    }

    #[test]
    fn indt_get_refuses_an_id_that_is_not_a_ulid_or_names_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let repo = EntriesRepository::new(dir.path());
        assert!(repo.get("../escape").is_none());
        assert!(repo.get(&new_id(5)).is_none());
        assert!(repo.save("../escape", &json!({})).is_err());
    }

    #[test]
    fn indt_a_lost_index_is_rebuilt_from_the_entry_files() {
        let dir = tempfile::tempdir().unwrap();
        let repo = EntriesRepository::new(dir.path());
        let id = new_id(3_000);
        repo.save(&id, &entry(&id, None, 3.0)).unwrap();
        fs::write(dir.path().join(INDEX_FILE), "{not json").unwrap();
        let all = repo.all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0]["id"], id.as_str());
    }

    #[test]
    fn indt_prune_drops_entries_older_than_the_ttl_when_due() {
        let dir = tempfile::tempdir().unwrap();
        let repo = EntriesRepository::new(dir.path());
        let now: i64 = 1_800_000_000;
        let old = new_id(4_000);
        let fresh = new_id(5_000);
        repo.save(&old, &entry(&old, None, (now - 25 * 3600) as f64))
            .unwrap();
        repo.save(&fresh, &entry(&fresh, None, (now - 3600) as f64))
            .unwrap();
        fs::write(dir.path().join(LAST_PRUNE_FILE), (now - 10).to_string()).unwrap();

        repo.prune_if_due(now, 300, 24).unwrap();
        assert!(repo.get(&old).is_some(), "the last prune is 10 s old: not due");

        repo.prune_if_due(now + 300, 300, 24).unwrap();
        assert!(repo.get(&old).is_none(), "25 hours old: pruned");
        assert!(repo.get(&fresh).is_some(), "1 hour old: kept");
        assert_eq!(repo.all().len(), 1);
        assert_eq!(
            fs::read_to_string(dir.path().join(LAST_PRUNE_FILE)).unwrap(),
            (now + 300).to_string()
        );
    }

    #[test]
    fn indt_a_tab_keeps_its_newest_entries_up_to_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let repo = EntriesRepository::new(dir.path());
        let ids: Vec<String> = (0..4).map(|i| new_id(6_000 + i)).collect();
        for id in &ids {
            repo.save(id, &entry(id, Some("tab-a"), 1.0)).unwrap();
        }
        let other = new_id(7_000);
        repo.save(&other, &entry(&other, Some("tab-b"), 1.0)).unwrap();

        repo.enforce_tab_limit("tab-a", 3).unwrap();
        assert!(repo.get(&ids[0]).is_none(), "the oldest of the tab goes");
        for id in &ids[1..] {
            assert!(repo.get(id).is_some());
        }
        assert!(repo.get(&other).is_some(), "another tab is untouched");

        repo.enforce_tab_limit("tab-a", 0).unwrap();
        assert_eq!(repo.all().len(), 4, "0 keeps everything");
    }

    #[test]
    fn indt_the_breaker_reports_the_first_failure_and_holds_for_30_seconds() {
        let path = PathBuf::from("/indt-breaker-test/unique-path");
        assert!(!is_suppressed(&path, 0));
        assert!(note_failure(&path, 1_000), "the first failure is reported");
        assert!(is_suppressed(&path, 1_000 + SUPPRESS_MS - 1));
        assert!(!is_suppressed(&path, 1_000 + SUPPRESS_MS));
        assert!(
            !note_failure(&path, 1_000 + SUPPRESS_MS),
            "a failure before any success is not reported again"
        );
        note_success(&path);
        assert!(!is_suppressed(&path, 0));
        assert!(note_failure(&path, 50_000), "reported again after a success");
        note_success(&path);
    }
}
