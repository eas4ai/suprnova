//! The author's signing key on disk (REG-018, REG-020): where
//! `live:registry new` writes it, where `live:registry sign` reads it, and
//! the file's format. The key never sits inside a library, so a published
//! repository never carries it.
//!
//! A key file is three lines: the format, `public ed25519:<base64>`, and
//! `secret <base64>` of the 32 secret bytes. It is written once, with mode
//! 0600 on Unix, and never replaced: losing or overwriting it strands every
//! application that pinned the library's key. The default file is named by
//! the public key's fingerprint, so two libraries never share one and
//! `sign` finds a library's key from its `library.json` alone.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::signing::{PublicKey, SecretKey};
use super::{RegistryError, Result};

/// The first line of a key file.
pub const KEY_FILE_FORMAT: &str = "suprnova-library-key/1";

/// The directory under the configuration directory that holds key files.
const KEY_DIRECTORY: &str = "suprnova/library-keys";

/// The largest key file read; a real one is under 200 bytes.
const MAX_KEY_FILE_BYTES: u64 = 4096;

/// The user's configuration directory, as `live:registry` reads it from the
/// environment of this process.
pub fn config_dir() -> Result<PathBuf> {
    config_dir_from(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
        std::env::var_os("APPDATA"),
    )
}

/// The configuration directory the given environment names, as the `dirs`
/// crate resolves it: on Windows `%APPDATA%`; on macOS
/// `$HOME/Library/Application Support`; on other Unix `$XDG_CONFIG_HOME`
/// when it is absolute, else `$HOME/.config`. A relative value is ignored,
/// as the XDG specification says, so a key never lands relative to
/// wherever the command happens to run.
pub fn config_dir_from(
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
    app_data: Option<OsString>,
) -> Result<PathBuf> {
    let absolute =
        |value: Option<OsString>| value.map(PathBuf::from).filter(|path| path.is_absolute());
    let missing = || {
        RegistryError::Invalid(
            "cannot find your configuration directory: set HOME (or APPDATA on Windows) to an absolute path, or name the key file with SUPRNOVA_LIBRARY_KEY".to_owned(),
        )
    };
    if cfg!(windows) {
        return absolute(app_data).ok_or_else(missing);
    }
    if cfg!(target_os = "macos") {
        return absolute(home)
            .map(|home| home.join("Library/Application Support"))
            .ok_or_else(missing);
    }
    if let Some(config) = absolute(xdg_config_home) {
        return Ok(config);
    }
    absolute(home)
        .map(|home| home.join(".config"))
        .ok_or_else(missing)
}

/// Where a library's key lives under a configuration directory:
/// `suprnova/library-keys/<hex>.key`, the hex of the public key's
/// fingerprint.
pub fn key_path_in(config: &Path, public: &PublicKey) -> PathBuf {
    let fingerprint = public.fingerprint();
    let hex = fingerprint
        .as_str()
        .strip_prefix("sha256:")
        .unwrap_or(fingerprint.as_str());
    config.join(KEY_DIRECTORY).join(format!("{hex}.key"))
}

/// Writes a new key file, creating its directory, and refuses to replace a
/// file that is already there.
pub fn write_key_file(path: &Path, secret: &SecretKey, public: &PublicKey) -> Result<()> {
    let io = |what: &str, error: std::io::Error| {
        RegistryError::Io(format!("cannot {what} {}: {error}", path.display()))
    };
    if let Some(parent) = path.parent() {
        create_private_dir(parent).map_err(|error| {
            RegistryError::Io(format!("cannot create {}: {error}", parent.display()))
        })?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            RegistryError::Invalid(format!(
                "{} already exists; a key file is never replaced",
                path.display()
            ))
        } else {
            io("create", error)
        }
    })?;
    let contents = format!(
        "{KEY_FILE_FORMAT}\npublic {}\nsecret {}\n",
        public.encode(),
        STANDARD.encode(secret.bytes())
    );
    file.write_all(contents.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| io("write", error))
}

/// Creates a directory and its parents, the ones it creates readable only
/// by their owner on Unix.
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// Reads a key file. Nothing of its contents reaches an error message.
pub fn read_key_file(path: &Path) -> Result<(SecretKey, PublicKey)> {
    let malformed = || {
        RegistryError::Invalid(format!(
            "{} is not a Suprnova library key file (`{KEY_FILE_FORMAT}`)",
            path.display()
        ))
    };
    let metadata = std::fs::metadata(path).map_err(|error| {
        RegistryError::Io(format!(
            "cannot read the key file {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() || metadata.len() > MAX_KEY_FILE_BYTES {
        return Err(malformed());
    }
    let text = std::fs::read_to_string(path).map_err(|error| {
        RegistryError::Io(format!(
            "cannot read the key file {}: {error}",
            path.display()
        ))
    })?;
    let mut lines = text.lines();
    if lines.next() != Some(KEY_FILE_FORMAT) {
        return Err(malformed());
    }
    let public = lines
        .next()
        .and_then(|line| line.strip_prefix("public "))
        .and_then(|encoded| PublicKey::parse(encoded).ok())
        .ok_or_else(malformed)?;
    let secret: [u8; 32] = lines
        .next()
        .and_then(|line| line.strip_prefix("secret "))
        .and_then(|encoded| STANDARD.decode(encoded).ok())
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(malformed)?;
    if lines.any(|line| !line.is_empty()) {
        return Err(malformed());
    }
    Ok((SecretKey::from_bytes(secret), public))
}

/// The key file `sign` reads for a library: the one `named` names
/// (`SUPRNOVA_LIBRARY_KEY`) when given, else the library's default file under
/// `config`. A key file inside the library is refused, by its path as given
/// and by the file it resolves to, so a key never ships with the library.
pub fn key_file_for(
    library_root: &Path,
    public: &PublicKey,
    named: Option<OsString>,
    config: &Path,
) -> Result<PathBuf> {
    let path = match named.filter(|value| !value.is_empty()) {
        Some(value) => PathBuf::from(value),
        None => key_path_in(config, public),
    };
    refuse_inside_library(library_root, &path)?;
    std::fs::metadata(&path).map_err(|error| {
        RegistryError::Io(format!(
            "no key file at {}: {error}; `live:registry new` writes it, or SUPRNOVA_LIBRARY_KEY names it",
            path.display()
        ))
    })?;
    Ok(path)
}

/// Refuses a key path inside the library: by the path as given, and by
/// where its nearest existing ancestor resolves, so neither `..` nor a link
/// puts a key in the project. The path itself need not exist yet.
pub fn refuse_inside_library(library_root: &Path, path: &Path) -> Result<()> {
    let absolute = |path: &Path| -> Result<PathBuf> {
        let joined = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|error| {
                    RegistryError::Io(format!("cannot read the working directory: {error}"))
                })?
                .join(path)
        };
        Ok(normalize(&joined))
    };
    let inside = || {
        RegistryError::Invalid(format!(
            "the key file {} is inside the library; keep it outside the project, in your configuration directory or where SUPRNOVA_LIBRARY_KEY names",
            path.display()
        ))
    };
    let root = absolute(library_root)?;
    let candidate = absolute(path)?;
    if candidate.starts_with(&root) {
        return Err(inside());
    }
    let root_resolved = std::fs::canonicalize(library_root).map_err(|error| {
        RegistryError::Io(format!(
            "cannot resolve {}: {error}",
            library_root.display()
        ))
    })?;
    let existing = candidate
        .ancestors()
        .find_map(|ancestor| std::fs::canonicalize(ancestor).ok());
    if existing.is_some_and(|resolved| resolved.starts_with(&root_resolved)) {
        return Err(inside());
    }
    Ok(())
}

/// Removes `.` and folds `..` lexically, so a path cannot name the library
/// through a detour the prefix check would miss.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;

    use super::{
        KEY_FILE_FORMAT, config_dir_from, key_file_for, key_path_in, read_key_file, write_key_file,
    };
    use crate::registry::signing::{PublicKey, SecretKey};

    fn public() -> PublicKey {
        PublicKey::parse(&format!("ed25519:{}", STANDARD.encode([3u8; 32]))).expect("key")
    }

    fn os(value: &str) -> Option<OsString> {
        Some(OsString::from(value))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn the_configuration_directory_follows_xdg_then_home() {
        assert_eq!(
            config_dir_from(os("/xdg"), os("/home/a"), None).expect("dir"),
            PathBuf::from("/xdg")
        );
        assert_eq!(
            config_dir_from(os("relative"), os("/home/a"), None).expect("dir"),
            PathBuf::from("/home/a/.config"),
            "a relative XDG_CONFIG_HOME is ignored, as the XDG specification says"
        );
        assert_eq!(
            config_dir_from(None, os("/home/a"), None).expect("dir"),
            PathBuf::from("/home/a/.config")
        );
        assert!(config_dir_from(None, None, None).is_err());
        assert!(config_dir_from(None, os("relative"), None).is_err());
    }

    #[test]
    fn reg_018_a_key_is_named_by_its_fingerprint_under_the_configuration_directory() {
        let path = key_path_in(Path::new("/config"), &public());
        let hex = public()
            .fingerprint()
            .as_str()
            .trim_start_matches("sha256:")
            .to_owned();
        assert_eq!(
            path,
            PathBuf::from(format!("/config/suprnova/library-keys/{hex}.key"))
        );
    }

    #[test]
    fn reg_018_a_key_file_round_trips_and_is_never_overwritten() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = key_path_in(dir.path(), &public());
        write_key_file(&path, &SecretKey::from_bytes([5u8; 32]), &public()).expect("writes");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.starts_with(KEY_FILE_FORMAT), "{text}");
        let (secret, read_public) = read_key_file(&path).expect("reads");
        assert_eq!(secret.bytes(), &[5u8; 32]);
        assert_eq!(read_public, public());
        assert!(
            write_key_file(&path, &SecretKey::from_bytes([6u8; 32]), &public()).is_err(),
            "an existing key file is never replaced"
        );
        let (secret, _) = read_key_file(&path).expect("reads");
        assert_eq!(secret.bytes(), &[5u8; 32]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "the key file is private to its owner");
        }
    }

    #[test]
    fn a_malformed_key_file_is_refused_without_echoing_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("bad.key");
        std::fs::write(
            &path,
            "suprnova-library-key/1\npublic nope\nsecret c2VjcmV0\n",
        )
        .expect("write");
        let error = read_key_file(&path).expect_err("refused").to_string();
        assert!(!error.contains("c2VjcmV0"), "{error}");
        std::fs::write(&path, "something else\n").expect("write");
        assert!(read_key_file(&path).is_err());
    }

    #[test]
    fn reg_020_sign_reads_the_named_key_or_the_configuration_directory_and_refuses_one_inside_the_project()
     {
        let dir = tempfile::tempdir().expect("tempdir");
        let library = dir.path().join("acme");
        let config = dir.path().join("config");
        std::fs::create_dir_all(&library).expect("library");
        let default = key_path_in(&config, &public());
        write_key_file(&default, &SecretKey::from_bytes([5u8; 32]), &public()).expect("key");
        assert_eq!(
            key_file_for(&library, &public(), None, &config).expect("default"),
            default
        );

        let elsewhere = dir.path().join("ci.key");
        std::fs::copy(&default, &elsewhere).expect("copy");
        assert_eq!(
            key_file_for(&library, &public(), Some(elsewhere.clone().into()), &config)
                .expect("named"),
            elsewhere
        );

        let inside = library.join("signing.key");
        std::fs::copy(&default, &inside).expect("copy");
        let error = key_file_for(&library, &public(), Some(inside.into()), &config)
            .expect_err("a key inside the project is refused");
        assert!(error.to_string().contains("inside the library"), "{error}");

        #[cfg(unix)]
        {
            let link = dir.path().join("link.key");
            std::os::unix::fs::symlink(library.join("signing.key"), &link).expect("symlink");
            assert!(
                key_file_for(&library, &public(), Some(link.into()), &config).is_err(),
                "a link from outside to a key inside the project is refused"
            );
        }

        let missing = key_file_for(
            &library,
            &public(),
            Some(dir.path().join("none").into()),
            &config,
        );
        assert!(missing.is_err());
    }
}
