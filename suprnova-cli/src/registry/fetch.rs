//! Fetching a library's files at a release tag (REG-009, REG-026), from a
//! forge over HTTPS, from a tree on disk, or from the embedded shipped
//! library, behind one trait so every other step is the same for each.

use std::collections::BTreeMap;

use super::address::LibraryAddress;
use super::{RegistryError, Result};

/// The largest response a fetch accepts (REG-009).
pub const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;

/// How long a fetch may take (REG-009).
pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The commit a tag resolved to; every file of one plan comes from it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Commit(pub String);

/// Where a library's bytes come from.
pub trait Fetcher {
    /// The release versions the library lists, from its `v<semver>` tags,
    /// pre-releases excluded.
    fn versions(&self, library: &LibraryAddress) -> Result<Vec<semver::Version>>;

    /// The commit the tag `v<version>` names.
    fn resolve(&self, library: &LibraryAddress, version: &semver::Version) -> Result<Commit>;

    /// One file of the library at that commit, by path from the library
    /// root (`library.json`, `components/<name>/manifest.json`, ...).
    fn file(&self, library: &LibraryAddress, commit: &Commit, path: &str) -> Result<Vec<u8>>;
}

/// Fetches raw files from a forge over HTTPS (REG-009).
#[derive(Debug, Default)]
pub struct HttpsFetcher;

impl Fetcher for HttpsFetcher {
    fn versions(&self, library: &LibraryAddress) -> Result<Vec<semver::Version>> {
        let _ = library;
        Err(RegistryError::NotBuilt("HTTPS fetching"))
    }

    fn resolve(&self, library: &LibraryAddress, version: &semver::Version) -> Result<Commit> {
        let _ = (library, version);
        Err(RegistryError::NotBuilt("HTTPS fetching"))
    }

    fn file(&self, library: &LibraryAddress, commit: &Commit, path: &str) -> Result<Vec<u8>> {
        let _ = (library, commit, path);
        Err(RegistryError::NotBuilt("HTTPS fetching"))
    }
}

/// Reads a library tree on disk, the author's own (REG-008 path sources);
/// its one version is the one `library.json` names.
#[derive(Debug)]
pub struct PathFetcher {
    /// The library root.
    pub root: std::path::PathBuf,
}

impl Fetcher for PathFetcher {
    fn versions(&self, library: &LibraryAddress) -> Result<Vec<semver::Version>> {
        let _ = library;
        Err(RegistryError::NotBuilt("path fetching"))
    }

    fn resolve(&self, library: &LibraryAddress, version: &semver::Version) -> Result<Commit> {
        let _ = (library, version);
        Err(RegistryError::NotBuilt("path fetching"))
    }

    fn file(&self, library: &LibraryAddress, commit: &Commit, path: &str) -> Result<Vec<u8>> {
        let _ = (library, commit, path);
        Err(RegistryError::NotBuilt("path fetching"))
    }
}

/// A library held in memory, for tests: versions, each with its files by
/// path.
#[derive(Debug, Default)]
pub struct FakeFetcher {
    libraries: BTreeMap<LibraryAddress, BTreeMap<semver::Version, BTreeMap<String, Vec<u8>>>>,
}

impl FakeFetcher {
    /// Adds one version of a library.
    pub fn add_version(
        &mut self,
        library: LibraryAddress,
        version: semver::Version,
        files: BTreeMap<String, Vec<u8>>,
    ) {
        self.libraries
            .entry(library)
            .or_default()
            .insert(version, files);
    }
}

impl Fetcher for FakeFetcher {
    fn versions(&self, library: &LibraryAddress) -> Result<Vec<semver::Version>> {
        Ok(self
            .libraries
            .get(library)
            .map(|versions| {
                versions
                    .keys()
                    .filter(|v| v.pre.is_empty())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
    }

    fn resolve(&self, library: &LibraryAddress, version: &semver::Version) -> Result<Commit> {
        self.libraries
            .get(library)
            .and_then(|versions| versions.get(version))
            .map(|_| Commit(format!("fake-{library}-{version}")))
            .ok_or_else(|| RegistryError::Network(format!("{library} has no tag v{version}")))
    }

    fn file(&self, library: &LibraryAddress, commit: &Commit, path: &str) -> Result<Vec<u8>> {
        let versions = self
            .libraries
            .get(library)
            .ok_or_else(|| RegistryError::Network(format!("{library} is unknown")))?;
        let (_, files) = versions
            .iter()
            .find(|(version, _)| Commit(format!("fake-{library}-{version}")) == *commit)
            .ok_or_else(|| {
                RegistryError::Network(format!("{library} has no commit {}", commit.0))
            })?;
        files
            .get(path)
            .cloned()
            .ok_or_else(|| RegistryError::Network(format!("{library}@{} has no {path}", commit.0)))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{FakeFetcher, Fetcher};
    use crate::registry::address::LibraryAddress;

    #[test]
    fn the_fake_fetcher_lists_release_versions_and_serves_files_at_their_commit() {
        let mut fetcher = FakeFetcher::default();
        let library = LibraryAddress("github.com/acme/acme-ui".to_owned());
        fetcher.add_version(
            library.clone(),
            semver::Version::new(1, 0, 0),
            BTreeMap::from([("library.json".to_owned(), b"{}".to_vec())]),
        );
        fetcher.add_version(
            library.clone(),
            semver::Version::parse("2.0.0-rc.1").expect("semver"),
            BTreeMap::new(),
        );
        assert_eq!(
            fetcher.versions(&library).expect("versions"),
            vec![semver::Version::new(1, 0, 0)]
        );
        let commit = fetcher
            .resolve(&library, &semver::Version::new(1, 0, 0))
            .expect("commit");
        assert_eq!(
            fetcher
                .file(&library, &commit, "library.json")
                .expect("file"),
            b"{}".to_vec()
        );
        assert!(fetcher.file(&library, &commit, "missing").is_err());
    }
}
