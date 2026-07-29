//! `suprnova dev:tls` — register a portless HTTPS dev URL and trust
//! portless's local CA in every browser certificate store on the machine.
//!
//! On Linux, browsers read NSS databases (`~/.pki/nssdb`, Flatpak
//! `~/.var/app/<id>/.pki/nssdb`, Firefox profile `cert9.db`), not the
//! system trust store. We install the CA there with `certutil`, which
//! needs no sudo. macOS/Windows delegate to `portless trust`.
//!
//! See `manual/dev-tls.md` for the end-to-end workflow and troubleshooting.
//!
//! # Trust boundary
//!
//! Installing a CA into a browser trust store is the most dangerous thing
//! this CLI does: whoever controls that certificate can mint a valid-looking
//! certificate for any site the browser visits. So nothing a *project* can
//! say is allowed to influence which certificate gets installed.
//!
//! That mattered because `run` loads the project's `.env` (for `SERVER_PORT`)
//! before resolving the CA, and `dotenvy` fills in variables the real
//! environment does not define. A checked-in `.env` containing
//! `PORTLESS_STATE_DIR=/tmp/attacker` — or `HOME=/tmp/attacker` — therefore
//! chose the CA, and `git clone && suprnova dev:tls` installed it. The
//! trust-relevant environment is now snapshotted by [`TrustEnv::capture`]
//! *before* `.env` is loaded, the certificate is checked for ownership,
//! permissions and structure, and its SHA-256 fingerprint is shown to the
//! user and pinned under the CLI's own state directory so a CA that changes
//! underneath them cannot be installed without a human looking at it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::ui;

/// The CA's nickname in NSS, matching the cert's subject CN. Only used by
/// the Linux NSS path; gated so non-Linux builds don't warn under
/// `-D warnings`.
#[cfg(target_os = "linux")]
const CA_NICKNAME: &str = "portless Local CA";

/// Default backend port. Mirrors `serve::DEFAULT_BACKEND_PORT` and the
/// framework's `suprnova::config::providers::server::DEFAULT_SERVER_PORT`;
/// kept in sync deliberately (the CLI can't depend on the framework crate).
const DEFAULT_BACKEND_PORT: u16 = 8765;

/// A browser NSS certificate database to install the CA into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NssDb {
    /// Filesystem path to the NSS database directory.
    pub path: PathBuf,
    /// Whether the caller should `mkdir -p` this directory before
    /// running `certutil`. True for Chromium-family stores (safe to
    /// create ahead of the browser); false for Firefox profiles (we
    /// never fabricate one).
    pub create_if_missing: bool,
}

/// Resolve the app name. `--name` wins; else `[package].name` from the
/// project's `Cargo.toml`; else an error telling the user what to do.
pub fn resolve_name(cli: Option<String>, cargo_name: Option<String>) -> Result<String, String> {
    cli.or(cargo_name).ok_or_else(|| {
        "Could not determine the app name. Pass --name <name>, or run from a \
         Suprnova project root that has a Cargo.toml."
            .to_string()
    })
}

/// Resolve the backend port. `--port` wins; else `SERVER_PORT` (passed in
/// as `env_server_port`); else the 8765 default. No free-port scan —
/// `dev:tls` registers a route, it doesn't bind.
pub fn resolve_port(cli: Option<u16>, env_server_port: Option<u16>) -> u16 {
    cli.or(env_server_port).unwrap_or(DEFAULT_BACKEND_PORT)
}

/// Locate portless's CA. `$PORTLESS_STATE_DIR/ca.pem` when the state dir
/// is set, else `<home>/.portless/ca.pem`.
pub fn ca_path_for(state_dir: Option<&Path>, home: &Path) -> PathBuf {
    match state_dir {
        Some(dir) => dir.join("ca.pem"),
        None => home.join(".portless").join("ca.pem"),
    }
}

/// Discover candidate browser NSS databases under `home`.
///
/// Pure: computes paths and flags only — it creates nothing, so it stays
/// unit-testable against a temporary `$HOME`. The caller performs any
/// `mkdir -p` (guided by `create_if_missing`).
///
/// - `~/.pki/nssdb` (Chrome/Chromium deb/rpm) is **always** included with
///   `create_if_missing = true`, even when absent — a fresh Chrome may not
///   have created it yet, and trusting there pre-creation works.
/// - `~/.var/app/<id>/.pki/nssdb` (Flatpak Chromium-family) is included
///   only when that nssdb directory already exists (we don't fabricate NSS
///   stores for every Flatpak app), with `create_if_missing = true`.
/// - Firefox profiles under `~/.mozilla/firefox/<p>/` and the Flatpak
///   Firefox profile dir are included only when they already contain a
///   `cert9.db`, with `create_if_missing = false`.
pub fn nss_databases(home: &Path) -> Vec<NssDb> {
    let mut dbs = Vec::new();

    // Chrome / Chromium (deb/rpm) — always, create if needed.
    dbs.push(NssDb {
        path: home.join(".pki").join("nssdb"),
        create_if_missing: true,
    });

    // Flatpak Chromium-family: ~/.var/app/<id>/.pki/nssdb (existing only).
    let var_app = home.join(".var").join("app");
    if let Ok(entries) = std::fs::read_dir(&var_app) {
        for entry in entries.flatten() {
            let nssdb = entry.path().join(".pki").join("nssdb");
            if nssdb.is_dir() {
                dbs.push(NssDb {
                    path: nssdb,
                    create_if_missing: true,
                });
            }
        }
    }

    // Firefox profiles (native + Flatpak), existing cert9.db only.
    let firefox_roots = [
        home.join(".mozilla").join("firefox"),
        home.join(".var")
            .join("app")
            .join("org.mozilla.firefox")
            .join(".mozilla")
            .join("firefox"),
    ];
    for root in firefox_roots {
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let profile = entry.path();
                if profile.join("cert9.db").is_file() {
                    dbs.push(NssDb {
                        path: profile,
                        create_if_missing: false,
                    });
                }
            }
        }
    }

    dbs
}

/// Read a `u16` env var, treating empty/unparseable as unset.
fn env_port(key: &str) -> Option<u16> {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok())
}

/// The trust-relevant environment, snapshotted before the project's `.env`
/// is loaded.
///
/// This type exists purely so the two variables that decide *which*
/// certificate gets installed are read once, at a point where only the
/// invoking shell can have set them. After `dotenvy::dotenv()` runs, reading
/// them from the process environment would also read the project's `.env`.
#[derive(Debug, Clone, Default)]
pub struct TrustEnv {
    /// `$PORTLESS_STATE_DIR` as the real environment defined it, if at all.
    pub state_dir: Option<PathBuf>,
    /// `$HOME` as the real environment defined it, if at all.
    pub home: Option<PathBuf>,
}

impl TrustEnv {
    /// Snapshot the process environment. Call this before loading `.env`.
    pub fn capture() -> Self {
        Self {
            state_dir: std::env::var_os("PORTLESS_STATE_DIR").map(PathBuf::from),
            home: std::env::var_os("HOME").map(PathBuf::from),
        }
    }

    /// The home directory to search for NSS stores and tool state.
    pub fn home(&self) -> Result<&Path, String> {
        self.home
            .as_deref()
            .ok_or_else(|| "Could not determine your home directory ($HOME unset)".to_string())
    }

    /// Where portless's CA should be, according to the snapshot only.
    pub fn ca_path(&self) -> Result<PathBuf, String> {
        Ok(ca_path_for(self.state_dir.as_deref(), self.home()?))
    }
}

/// Largest CA file we will read. A CA certificate is a couple of kilobytes;
/// the cap keeps a redirected path from making us slurp something huge.
const MAX_CA_BYTES: u64 = 64 * 1024;

/// Where the CLI records the fingerprint of the CA it last installed.
///
/// Under the user's home, in the tool's own directory — not next to the CA,
/// and nowhere a project can reach. A pin the same actor could rewrite would
/// detect nothing.
fn pin_path(home: &Path) -> PathBuf {
    home.join(".suprnova").join("dev-tls-ca.sha256")
}

/// A CA certificate that has passed inspection, with the fingerprint the
/// user is asked to approve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaCertificate {
    /// Where it was read from.
    pub path: PathBuf,
    /// Uppercase colon-separated SHA-256 of the DER body.
    pub fingerprint: String,
}

/// Extract the single DER certificate from a PEM file.
///
/// Exactly one `CERTIFICATE` block is required. `certutil -A` imports only
/// the first certificate in a bundle, so accepting several would mean the
/// fingerprint we showed the user was not necessarily the one installed.
///
/// This deliberately does not parse X.509. A full parse would need another
/// dependency and still could not distinguish a legitimate portless CA from
/// a hostile one — that judgement belongs to the human at the prompt, and
/// what they need for it is a stable fingerprint. The structural checks here
/// only ensure we are fingerprinting a certificate rather than a text file.
pub fn parse_single_certificate(pem: &str) -> Result<Vec<u8>, String> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";

    let mut bodies = Vec::new();
    let mut rest = pem;
    while let Some(start) = rest.find(BEGIN) {
        let after = &rest[start + BEGIN.len()..];
        let end = after.find(END).ok_or_else(|| {
            "the CA file has a CERTIFICATE block that is never closed".to_string()
        })?;
        bodies.push(&after[..end]);
        rest = &after[end + END.len()..];
    }

    match bodies.len() {
        0 => return Err("the CA file contains no CERTIFICATE block".to_string()),
        1 => {}
        n => {
            return Err(format!(
                "the CA file contains {n} CERTIFICATE blocks; expected exactly one, \
                 because only the first would be installed and the fingerprint \
                 shown would not describe what you trusted"
            ));
        }
    }

    let base64_body: String = bodies[0].chars().filter(|c| !c.is_whitespace()).collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(base64_body.as_bytes())
        .map_err(|e| format!("the CA file's certificate body is not valid base64: {e}"))?;

    // A DER certificate is a SEQUENCE; anything else is not a certificate.
    if der.first() != Some(&0x30) || der.len() < 64 {
        return Err("the CA file's certificate body is not DER-encoded".to_string());
    }

    Ok(der)
}

/// SHA-256 of the DER body, formatted the way `certutil` and browsers show
/// it, so a user can compare the two by eye.
pub fn fingerprint_of(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Read and inspect the CA at `path`.
///
/// The file must be a regular file (not a symlink), owned by the same user
/// as `home`, and not writable by anyone else — a CA that another account
/// can rewrite offers no security once trusted, and following a symlink
/// would let the pinned fingerprint describe a file other than the one
/// `certutil` later reads.
pub fn load_ca(path: &Path, home: &Path) -> Result<CaCertificate, String> {
    let meta = path.symlink_metadata().map_err(|e| {
        format!(
            "portless CA not readable at {}: {e}. Start the proxy once so portless \
             generates its CA (e.g. `systemctl start portless` or `portless proxy \
             start`), then re-run `suprnova dev:tls`.",
            path.display()
        )
    })?;

    if meta.file_type().is_symlink() {
        return Err(format!(
            "Refusing to trust {}: it is a symlink. The CA must be a regular file, \
             so that what we fingerprint is what gets installed.",
            path.display()
        ));
    }
    if !meta.file_type().is_file() {
        return Err(format!(
            "Refusing to trust {}: it is not a regular file.",
            path.display()
        ));
    }
    if meta.len() > MAX_CA_BYTES {
        return Err(format!(
            "Refusing to trust {}: it is {} bytes, larger than any CA certificate.",
            path.display(),
            meta.len()
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let home_meta = home
            .metadata()
            .map_err(|e| format!("Could not stat your home directory {}: {e}", home.display()))?;
        if meta.uid() != home_meta.uid() {
            return Err(format!(
                "Refusing to trust {}: it is owned by uid {}, but your home directory \
                 is owned by uid {}. A certificate you do not own is not yours to \
                 trust.",
                path.display(),
                meta.uid(),
                home_meta.uid()
            ));
        }
        if meta.mode() & 0o002 != 0 {
            return Err(format!(
                "Refusing to trust {}: mode {:o} lets any user on this machine write \
                 to it, so trusting it would hand them your browser's certificate \
                 store.",
                path.display(),
                meta.mode() & 0o7777
            ));
        }
        // Group-writable is warned about rather than refused: a umask of 002
        // with per-user groups (the default on several distributions) makes
        // every file group-writable without exposing it to anyone, and a hard
        // refusal there would block a legitimate setup for no gain.
        if meta.mode() & 0o020 != 0 {
            ui::warning(&format!(
                "The CA at {} is group-writable (mode {:o}). If that group has other \
                 members, they can replace the certificate you are about to trust.",
                path.display(),
                meta.mode() & 0o7777
            ));
        }
    }
    #[cfg(not(unix))]
    let _ = home;

    let pem = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read the CA at {}: {e}", path.display()))?;
    let der = parse_single_certificate(&pem)
        .map_err(|e| format!("Refusing to trust {}: {e}", path.display()))?;

    Ok(CaCertificate {
        path: path.to_path_buf(),
        fingerprint: fingerprint_of(&der),
    })
}

/// What to do before mutating a trust store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustDecision {
    /// Install without asking — the user already said so on the command line.
    Proceed,
    /// Ask the human first. `changed` marks the case where a *different* CA
    /// now sits where a previously-approved one did.
    Confirm {
        /// Whether the fingerprint differs from the pinned one.
        changed: bool,
    },
    /// Do not install; the string explains why.
    Refuse(String),
}

/// Decide how to handle a trust-store mutation.
///
/// `--yes` covers the ordinary case, but deliberately does not cover a
/// changed fingerprint: a CA that is not the one approved last time is
/// precisely the event a human has to see, and a flag buried in a script
/// cannot see anything.
pub fn decide_trust(
    pinned: Option<&str>,
    fingerprint: &str,
    assume_yes: bool,
    stdin_is_tty: bool,
) -> TrustDecision {
    let changed = matches!(pinned, Some(p) if p != fingerprint);

    if changed {
        if !stdin_is_tty {
            return TrustDecision::Refuse(
                "The portless CA has changed since you last trusted it. That is either \
                 a CA rotation or someone substituting a certificate, and telling them \
                 apart needs a human — re-run `suprnova dev:tls` from an interactive \
                 terminal. --yes does not cover this case."
                    .to_string(),
            );
        }
        return TrustDecision::Confirm { changed: true };
    }

    if assume_yes {
        return TrustDecision::Proceed;
    }

    if !stdin_is_tty {
        return TrustDecision::Refuse(
            "Refusing to modify your browsers' certificate stores without confirmation, \
             and stdin is not a terminal. Re-run from an interactive shell, or pass \
             --yes if you have already checked the fingerprint above."
                .to_string(),
        );
    }

    TrustDecision::Confirm { changed: false }
}

/// Is `bin` on PATH? Probe by spawning it with a harmless arg; only a
/// `NotFound` spawn error counts as "absent" (a non-zero exit still means
/// the binary exists — e.g. `certutil -H` prints help and exits non-zero).
fn on_path(bin: &str, probe_arg: &str) -> bool {
    match Command::new(bin)
        .arg(probe_arg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(_) => true,
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// Register the portless alias: `portless alias <name> <port> --force`.
/// Writes portless's `routes.json` whether or not the proxy is running,
/// so it's safe to run before the daemon starts.
fn register_alias(name: &str, port: u16) -> Result<(), String> {
    let status = Command::new("portless")
        .args(["alias", name, &port.to_string(), "--force"])
        .status()
        .map_err(|e| format!("Failed to run `portless alias`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "`portless alias {name} {port}` failed (exit {})",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".into())
        ))
    }
}

/// Read the pinned fingerprint, if this machine has trusted a CA before.
///
/// A missing or unreadable pin is treated as "no pin": the user is asked
/// rather than refused, because failing closed on a first run would break
/// every fresh machine.
fn read_pin(home: &Path) -> Option<String> {
    std::fs::read_to_string(pin_path(home))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn write_pin(home: &Path, fingerprint: &str) -> Result<(), String> {
    let path = pin_path(home);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, format!("{fingerprint}\n"))
        .map_err(|e| format!("Failed to record the trusted CA fingerprint: {e}"))
}

/// Ask the user to approve the mutation, showing what they are approving.
fn confirm_trust(ca: &CaCertificate, changed: bool) -> Result<(), String> {
    ui::br();
    if changed {
        ui::warning("The portless CA is NOT the one you trusted last time.");
        ui::warning("If you did not rotate it deliberately, answer no and investigate.");
    }
    ui::info("About to install a certificate authority into your browsers'");
    ui::info("certificate stores. Anything signed by it will be trusted.");
    ui::hint(&format!("    file:        {}", ca.path.display()));
    ui::hint(&format!("    SHA-256:     {}", ca.fingerprint));
    ui::info("Type `yes` to continue, or anything else to abort:");

    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| format!("Failed to read the confirmation from stdin: {e}"))?;

    if line.trim().eq_ignore_ascii_case("yes") {
        Ok(())
    } else {
        Err("Aborted — no certificate store was modified.".to_string())
    }
}

/// Resolve, inspect and approve the CA, then install it.
///
/// Every platform goes through the same inspection and the same
/// confirmation; only the final install differs (NSS directly on Linux,
/// `portless trust` elsewhere).
fn trust_ca(env: &TrustEnv, assume_yes: bool, stdin_is_tty: bool) -> Result<(), String> {
    let home = env.home()?;
    let ca = load_ca(&env.ca_path()?, home)?;

    match decide_trust(
        read_pin(home).as_deref(),
        &ca.fingerprint,
        assume_yes,
        stdin_is_tty,
    ) {
        TrustDecision::Refuse(reason) => {
            ui::hint(&format!("    file:        {}", ca.path.display()));
            ui::hint(&format!("    SHA-256:     {}", ca.fingerprint));
            return Err(reason);
        }
        TrustDecision::Confirm { changed } => confirm_trust(&ca, changed)?,
        TrustDecision::Proceed => {
            ui::info(&format!(
                "Trusting CA {} ({})",
                ca.path.display(),
                ca.fingerprint
            ));
        }
    }

    install_ca(&ca, home)?;
    write_pin(home, &ca.fingerprint)
}

#[cfg(target_os = "linux")]
fn install_ca(ca: &CaCertificate, home: &Path) -> Result<(), String> {
    let ca = &ca.path;

    if !on_path("certutil", "-H") {
        ui::error("certutil (from libnss3-tools) is required to trust the CA in browsers.");
        ui::hint("  Debian/Ubuntu:  sudo apt install libnss3-tools");
        ui::hint("  Fedora/RHEL:    sudo dnf install nss-tools");
        ui::hint("  Arch:           sudo pacman -S nss");
        return Err("certutil not found".to_string());
    }

    let dbs = nss_databases(home);

    let mut trusted = Vec::new();
    for db in &dbs {
        if db.create_if_missing {
            let _ = std::fs::create_dir_all(&db.path);
        }
        if !db.path.is_dir() {
            continue;
        }
        match trust_in_db(&db.path, ca) {
            Ok(()) => trusted.push(db.path.clone()),
            Err(e) => ui::warning(&format!("Could not trust CA in {}: {e}", db.path.display())),
        }
    }

    if trusted.is_empty() {
        return Err("No browser certificate stores could be updated.".to_string());
    }

    ui::success(&format!("CA trusted in {} store(s):", trusted.len()));
    for p in &trusted {
        ui::hint(&format!("    {}", p.display()));
    }
    Ok(())
}

/// Install the CA into one NSS database, delete-then-add for idempotent
/// re-runs (`-t "C,,"` = trusted CA for issuing SSL server certs).
#[cfg(target_os = "linux")]
fn trust_in_db(db: &Path, ca: &Path) -> Result<(), String> {
    let db_arg = format!("sql:{}", db.display());

    // Delete any prior entry under the same nickname (ignore failure: it
    // may simply not exist yet), then add fresh.
    let _ = Command::new("certutil")
        .args(["-d", &db_arg, "-D", "-n", CA_NICKNAME])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let status = Command::new("certutil")
        .args(["-d", &db_arg, "-A", "-t", "C,,", "-n", CA_NICKNAME, "-i"])
        .arg(ca)
        .status()
        .map_err(|e| format!("certutil failed to spawn: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "certutil -A exited {}",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".into())
        ))
    }
}

/// Non-Linux install path. The CA has already been inspected and approved by
/// the shared code above; `portless trust` performs the OS-store mutation.
#[cfg(not(target_os = "linux"))]
fn install_ca(_ca: &CaCertificate, _home: &Path) -> Result<(), String> {
    ui::info("Delegating CA trust to `portless trust` (native OS cert store)...");
    let status = Command::new("portless")
        .arg("trust")
        .status()
        .map_err(|e| format!("Failed to run `portless trust`: {e}"))?;
    if status.success() {
        ui::success("CA trusted via `portless trust`");
        Ok(())
    } else {
        Err(format!(
            "`portless trust` failed (exit {})",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".into())
        ))
    }
}

/// Entry point for `suprnova dev:tls`.
pub fn run(name: Option<String>, port: Option<u16>, no_alias: bool, assume_yes: bool) {
    // Snapshot the trust-relevant environment BEFORE `.env` is loaded.
    // `dotenvy` fills in variables the real environment leaves unset, so
    // reading PORTLESS_STATE_DIR or HOME after this point would let a
    // checked-in `.env` choose which CA lands in the browser trust store.
    let trust_env = TrustEnv::capture();

    // Load .env so SERVER_PORT can resolve the route's target port.
    let _ = dotenvy::dotenv();

    ui::banner();
    ui::header("dev:tls — named HTTPS dev URL via portless");

    // 1. Locate portless.
    if !on_path("portless", "--version") {
        ui::error("portless was not found on your PATH.");
        ui::hint("Install it with:  npm install -g portless");
        ui::hint("Docs: https://portless.sh");
        std::process::exit(1);
    }
    ui::success("portless found");

    // 2. Resolve name + port.
    let cargo_name = crate::commands::cargo_meta::package_name_from_path(Path::new("Cargo.toml"));
    let app_name = match resolve_name(name, cargo_name) {
        Ok(n) => n,
        Err(e) => {
            ui::error(&e);
            std::process::exit(1);
        }
    };
    let backend_port = resolve_port(port, env_port("SERVER_PORT"));
    let url = format!("https://{app_name}.localhost");

    // 3. Register the alias (unless --no-alias).
    if no_alias {
        ui::hint("Skipping route registration (--no-alias).");
    } else {
        match register_alias(&app_name, backend_port) {
            Ok(()) => ui::success(&format!(
                "Route registered   {app_name}.localhost → 127.0.0.1:{backend_port}"
            )),
            Err(e) => {
                ui::error(&e);
                std::process::exit(1);
            }
        }
    }

    // 4. Trust the CA (the load-bearing step).
    let stdin_is_tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    if let Err(e) = trust_ca(&trust_env, assume_yes, stdin_is_tty) {
        ui::error(&e);
        std::process::exit(1);
    }

    // 5. Next steps — always, in order.
    ui::br();
    ui::info("Next:");
    ui::hint("  1. Fully restart your browser — type chrome://restart (a tab");
    ui::hint("     reload is not enough; the cert store is read once at launch)");
    ui::hint("  2. suprnova serve");
    ui::hint(&format!("  3. open {url}"));
    ui::br();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_name_prefers_cli_over_cargo() {
        assert_eq!(
            resolve_name(Some("flag".into()), Some("cargo".into())).unwrap(),
            "flag"
        );
    }

    #[test]
    fn resolve_name_falls_back_to_cargo() {
        assert_eq!(resolve_name(None, Some("cargo".into())).unwrap(), "cargo");
    }

    #[test]
    fn resolve_name_errors_when_both_absent() {
        let err = resolve_name(None, None).expect_err("no name source must error");
        assert!(err.contains("--name"), "error should mention --name: {err}");
    }

    #[test]
    fn resolve_port_precedence_cli_then_env_then_default() {
        assert_eq!(resolve_port(Some(9000), Some(7000)), 9000);
        assert_eq!(resolve_port(None, Some(7000)), 7000);
        assert_eq!(resolve_port(None, None), DEFAULT_BACKEND_PORT);
    }

    #[test]
    fn ca_path_respects_state_dir_then_home() {
        let state = PathBuf::from("/custom/state");
        let home = PathBuf::from("/home/alice");
        assert_eq!(
            ca_path_for(Some(state.as_path()), &home),
            PathBuf::from("/custom/state/ca.pem")
        );
        assert_eq!(
            ca_path_for(None, &home),
            PathBuf::from("/home/alice/.portless/ca.pem")
        );
    }

    #[test]
    fn nss_databases_always_includes_chrome_store_even_when_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path();
        let dbs = nss_databases(home);
        let chrome = home.join(".pki").join("nssdb");
        let found = dbs.iter().find(|d| d.path == chrome).expect("chrome store");
        assert!(
            found.create_if_missing,
            "chrome store must be create_if_missing"
        );
    }

    #[test]
    fn nss_databases_includes_existing_flatpak_chromium_excludes_non_browser() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path();
        // A flatpak browser with an nssdb...
        let browser_nssdb = home.join(".var/app/io.github.someone.Chromium/.pki/nssdb");
        std::fs::create_dir_all(&browser_nssdb).unwrap();
        // ...and a flatpak app with NO nssdb (must be excluded).
        std::fs::create_dir_all(home.join(".var/app/org.example.NotABrowser")).unwrap();

        let dbs = nss_databases(home);
        assert!(
            dbs.iter()
                .any(|d| d.path == browser_nssdb && d.create_if_missing),
            "flatpak nssdb should be discovered: {dbs:?}"
        );
        assert!(
            !dbs.iter()
                .any(|d| d.path.to_string_lossy().contains("NotABrowser")),
            "flatpak app without nssdb must be excluded: {dbs:?}"
        );
    }

    #[test]
    fn nss_databases_includes_firefox_profile_with_cert9_excludes_without() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path();
        let with_db = home.join(".mozilla/firefox/abc.default-release");
        std::fs::create_dir_all(&with_db).unwrap();
        std::fs::write(with_db.join("cert9.db"), b"fake").unwrap();
        let without_db = home.join(".mozilla/firefox/empty.profile");
        std::fs::create_dir_all(&without_db).unwrap();

        let dbs = nss_databases(home);
        let ff = dbs
            .iter()
            .find(|d| d.path == with_db)
            .expect("firefox profile");
        assert!(
            !ff.create_if_missing,
            "firefox profile must NOT be create_if_missing"
        );
        assert!(
            !dbs.iter().any(|d| d.path == without_db),
            "firefox profile lacking cert9.db must be excluded: {dbs:?}"
        );
    }

    /// A syntactically valid single-certificate PEM. The body is not a real
    /// X.509 certificate — `load_ca` only checks structure and fingerprints
    /// the DER, which is all the trust decision needs.
    fn sample_pem(seed: u8) -> String {
        let mut der = vec![0x30u8];
        der.extend(std::iter::repeat_n(seed, 200));
        let body = base64::engine::general_purpose::STANDARD.encode(&der);
        format!("-----BEGIN CERTIFICATE-----\n{body}\n-----END CERTIFICATE-----\n")
    }

    #[test]
    fn trust_env_resolves_the_ca_from_the_snapshot_only() {
        let env = TrustEnv {
            state_dir: Some(PathBuf::from("/custom/state")),
            home: Some(PathBuf::from("/home/alice")),
        };
        assert_eq!(
            env.ca_path().unwrap(),
            PathBuf::from("/custom/state/ca.pem")
        );

        let env = TrustEnv {
            state_dir: None,
            home: Some(PathBuf::from("/home/alice")),
        };
        assert_eq!(
            env.ca_path().unwrap(),
            PathBuf::from("/home/alice/.portless/ca.pem")
        );

        let env = TrustEnv::default();
        assert!(
            env.ca_path().is_err(),
            "without a home directory there is no CA to resolve"
        );
    }

    #[test]
    fn parse_single_certificate_round_trips_one_block() {
        let der = parse_single_certificate(&sample_pem(0xAB)).expect("one block parses");
        assert_eq!(der.first(), Some(&0x30));
        assert_eq!(der.len(), 201);
    }

    #[test]
    fn parse_single_certificate_rejects_zero_or_many_blocks() {
        let none = parse_single_certificate("nothing to see here")
            .expect_err("a file with no certificate must fail");
        assert!(none.contains("no CERTIFICATE"), "got: {none}");

        let bundle = format!("{}{}", sample_pem(0x01), sample_pem(0x02));
        let many = parse_single_certificate(&bundle)
            .expect_err("a bundle must fail: only the first would be installed");
        assert!(many.contains("2 CERTIFICATE blocks"), "got: {many}");
    }

    #[test]
    fn parse_single_certificate_rejects_non_certificate_bodies() {
        let unterminated = "-----BEGIN CERTIFICATE-----\nAAAA\n";
        assert!(parse_single_certificate(unterminated).is_err());

        let bad_base64 = "-----BEGIN CERTIFICATE-----\n!!!!\n-----END CERTIFICATE-----\n";
        assert!(parse_single_certificate(bad_base64).is_err());

        // Valid base64, but not DER (does not start with a SEQUENCE tag).
        let body = base64::engine::general_purpose::STANDARD.encode([0x41u8; 200]);
        let not_der = format!("-----BEGIN CERTIFICATE-----\n{body}\n-----END CERTIFICATE-----\n");
        let err = parse_single_certificate(&not_der).expect_err("non-DER must fail");
        assert!(err.contains("DER"), "got: {err}");
    }

    #[test]
    fn fingerprint_is_the_sha256_of_the_der() {
        // Known vector: SHA-256("abc").
        assert_eq!(
            fingerprint_of(b"abc"),
            "BA:78:16:BF:8F:01:CF:EA:41:41:40:DE:5D:AE:22:23:B0:03:61:A3:96:17:7A:9C:B4:10:FF:61:F2:00:15:AD"
        );
        // Different certificates must fingerprint differently, or the pin
        // detects nothing.
        let a = parse_single_certificate(&sample_pem(0x01)).unwrap();
        let b = parse_single_certificate(&sample_pem(0x02)).unwrap();
        assert_ne!(fingerprint_of(&a), fingerprint_of(&b));
    }

    #[test]
    fn load_ca_accepts_a_well_owned_regular_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path();
        let ca = home.join("ca.pem");
        std::fs::write(&ca, sample_pem(0x7F)).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Pin the mode so the test does not depend on the runner's umask.
            std::fs::set_permissions(&ca, std::fs::Permissions::from_mode(0o644)).unwrap();
        }

        let loaded = load_ca(&ca, home).expect("a normal CA file must load");
        assert_eq!(loaded.path, ca);
        assert_eq!(
            loaded.fingerprint.matches(':').count(),
            31,
            "SHA-256 is 32 bytes"
        );
    }

    #[cfg(unix)]
    #[test]
    fn load_ca_warns_but_accepts_a_group_writable_certificate() {
        // A umask of 002 with per-user groups produces 0664 for everything;
        // refusing there would block a legitimate setup, so this is a warning
        // and the load still succeeds.
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path();
        let ca = home.join("ca.pem");
        std::fs::write(&ca, sample_pem(0x03)).unwrap();
        std::fs::set_permissions(&ca, std::fs::Permissions::from_mode(0o664)).unwrap();

        assert!(
            load_ca(&ca, home).is_ok(),
            "group-writable must warn, not refuse"
        );
    }

    #[cfg(unix)]
    #[test]
    fn load_ca_refuses_a_symlink() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path();
        let real = home.join("real.pem");
        std::fs::write(&real, sample_pem(0x01)).unwrap();
        let link = home.join("ca.pem");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let err = load_ca(&link, home).expect_err("a symlinked CA must be refused");
        assert!(err.contains("symlink"), "got: {err}");
    }

    #[cfg(unix)]
    #[test]
    fn load_ca_refuses_a_world_writable_certificate() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path();
        let ca = home.join("ca.pem");
        std::fs::write(&ca, sample_pem(0x02)).unwrap();
        std::fs::set_permissions(&ca, std::fs::Permissions::from_mode(0o666)).unwrap();

        let err = load_ca(&ca, home).expect_err("a world-writable CA must be refused");
        assert!(err.contains("write"), "got: {err}");
    }

    #[test]
    fn load_ca_refuses_a_missing_or_malformed_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path();
        assert!(load_ca(&home.join("absent.pem"), home).is_err());

        let junk = home.join("junk.pem");
        std::fs::write(&junk, "this is not a certificate").unwrap();
        let err = load_ca(&junk, home).expect_err("a non-certificate must be refused");
        assert!(err.contains("Refusing to trust"), "got: {err}");
    }

    #[test]
    fn decide_trust_asks_before_the_first_mutation() {
        assert_eq!(
            decide_trust(None, "AA:BB", false, true),
            TrustDecision::Confirm { changed: false }
        );
    }

    #[test]
    fn decide_trust_refuses_a_non_interactive_run_without_yes() {
        let decision = decide_trust(None, "AA:BB", false, false);
        match decision {
            TrustDecision::Refuse(reason) => assert!(
                reason.contains("--yes") && reason.contains("terminal"),
                "the refusal should explain both ways out; got: {reason}"
            ),
            other => panic!("a non-interactive run must refuse, got {other:?}"),
        }
    }

    #[test]
    fn decide_trust_honours_explicit_yes() {
        assert_eq!(
            decide_trust(None, "AA:BB", true, false),
            TrustDecision::Proceed
        );
        assert_eq!(
            decide_trust(Some("AA:BB"), "AA:BB", true, false),
            TrustDecision::Proceed
        );
    }

    #[test]
    fn decide_trust_never_lets_yes_cover_a_changed_ca() {
        // Interactive: re-confirm, flagged as a change.
        assert_eq!(
            decide_trust(Some("AA:BB"), "CC:DD", true, true),
            TrustDecision::Confirm { changed: true }
        );
        // Non-interactive: refuse outright, even with --yes.
        match decide_trust(Some("AA:BB"), "CC:DD", true, false) {
            TrustDecision::Refuse(reason) => assert!(
                reason.contains("changed"),
                "the refusal should name the cause; got: {reason}"
            ),
            other => panic!("a changed CA must never install unattended, got {other:?}"),
        }
    }

    #[test]
    fn pin_lives_under_the_tools_own_state_directory() {
        let pin = pin_path(Path::new("/home/alice"));
        assert_eq!(
            pin,
            PathBuf::from("/home/alice/.suprnova/dev-tls-ca.sha256"),
            "the pin must not sit next to the CA it describes"
        );
    }

    #[test]
    fn pin_round_trips_and_treats_an_empty_file_as_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path();
        assert_eq!(read_pin(home), None, "no pin on a fresh machine");

        write_pin(home, "AA:BB:CC").expect("write pin");
        assert_eq!(read_pin(home).as_deref(), Some("AA:BB:CC"));

        std::fs::write(pin_path(home), "   \n").unwrap();
        assert_eq!(read_pin(home), None, "a blank pin is not a fingerprint");
    }

    #[test]
    fn nss_databases_discovers_flatpak_firefox_profile() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path();
        let profile = home.join(".var/app/org.mozilla.firefox/.mozilla/firefox/xyz.default");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(profile.join("cert9.db"), b"fake").unwrap();

        let dbs = nss_databases(home);
        let ff = dbs
            .iter()
            .find(|d| d.path == profile)
            .expect("flatpak firefox profile must be discovered");
        assert!(
            !ff.create_if_missing,
            "firefox profile must NOT be create_if_missing"
        );
    }
}
