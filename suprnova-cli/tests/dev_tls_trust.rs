//! `suprnova dev:tls` installs a certificate authority into every browser
//! certificate store on the machine. Whoever controls that CA can mint a
//! trusted certificate for any site, so a *project* must not be able to
//! choose it.
//!
//! `dev:tls` loads the project's `.env` (for `SERVER_PORT`), and `dotenvy`
//! defines variables the real environment leaves unset — so a checked-in
//! `.env` with `PORTLESS_STATE_DIR=…` used to select the certificate that got
//! installed. These tests drive the real binary with a fake `portless` and a
//! fake `certutil` that logs its arguments, so the assertions are about what
//! was actually handed to the trust store.
//!
//! Teeth: with the environment snapshot taken *after* `dotenvy::dotenv()`,
//! the certutil log names the attacker's ca.pem and
//! `the_project_env_cannot_choose_the_ca` fails.

#![cfg(all(unix, target_os = "linux"))]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use base64::Engine as _;
use tempfile::{TempDir, tempdir};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

fn combined(out: &Output) -> String {
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    s
}

/// A structurally valid single-certificate PEM. `certutil` is faked, so the
/// body only has to satisfy `dev:tls`'s own structural checks.
fn pem(seed: u8) -> String {
    let mut der = vec![0x30u8];
    der.extend(std::iter::repeat_n(seed, 200));
    let body = base64::engine::general_purpose::STANDARD.encode(&der);
    format!("-----BEGIN CERTIFICATE-----\n{body}\n-----END CERTIFICATE-----\n")
}

fn write_shim(dir: &Path, name: &str, script: &str) {
    let path = dir.join(name);
    fs::write(&path, script).expect("write shim");
    let mut perms = fs::metadata(&path).expect("stat shim").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).expect("chmod shim");
}

/// A hostile project: its `.env` points `PORTLESS_STATE_DIR` at a directory
/// the "attacker" controls, which holds a different CA from the real one in
/// `$HOME/.portless`.
struct Fixture {
    dir: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();

        let home = root.join("home");
        fs::create_dir_all(home.join(".portless")).expect("mkdir .portless");
        fs::write(home.join(".portless").join("ca.pem"), pem(0x11)).expect("write real ca");

        let evil = root.join("evil");
        fs::create_dir_all(&evil).expect("mkdir evil");
        fs::write(evil.join("ca.pem"), pem(0x22)).expect("write evil ca");

        let project = root.join("project");
        fs::create_dir_all(&project).expect("mkdir project");
        fs::write(
            project.join(".env"),
            format!(
                "SERVER_PORT=8765\nPORTLESS_STATE_DIR={}\nHOME={}\n",
                evil.display(),
                evil.display()
            ),
        )
        .expect("write hostile .env");
        fs::write(
            project.join("Cargo.toml"),
            "[package]\nname = \"probe\"\nversion = \"0.1.0\"\n",
        )
        .expect("write manifest");

        let bin = root.join("fakebin");
        fs::create_dir_all(&bin).expect("mkdir fakebin");
        write_shim(&bin, "portless", "#!/bin/sh\nexit 0\n");
        write_shim(
            &bin,
            "certutil",
            "#!/bin/sh\necho \"$@\" >> \"$CERTUTIL_LOG\"\nexit 0\n",
        );

        Self { dir }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }
    fn home(&self) -> PathBuf {
        self.root().join("home")
    }
    fn evil_ca(&self) -> PathBuf {
        self.root().join("evil").join("ca.pem")
    }
    fn real_ca(&self) -> PathBuf {
        self.home().join(".portless").join("ca.pem")
    }
    fn certutil_log(&self) -> PathBuf {
        self.root().join("certutil.log")
    }

    fn log(&self) -> String {
        fs::read_to_string(self.certutil_log()).unwrap_or_default()
    }

    fn run(&self, args: &[&str]) -> Output {
        let path = format!(
            "{}:{}",
            self.root().join("fakebin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(BIN)
            .arg("dev:tls")
            .arg("--no-alias")
            .args(args)
            .env("PATH", path)
            .env("HOME", self.home())
            .env_remove("PORTLESS_STATE_DIR")
            .env("CERTUTIL_LOG", self.certutil_log())
            .current_dir(self.root().join("project"))
            .output()
            .expect("spawn suprnova binary")
    }
}

#[test]
fn the_project_env_cannot_choose_the_ca() {
    let fx = Fixture::new();
    let out = fx.run(&["--yes"]);
    let text = combined(&out);

    assert_eq!(
        out.status.code(),
        Some(0),
        "dev:tls should have trusted the real CA; output: {text}"
    );

    let log = fx.log();
    assert!(
        !log.contains(&fx.evil_ca().display().to_string()),
        "the project's .env chose the CA that was installed!\ncertutil log:\n{log}\noutput: {text}"
    );
    assert!(
        log.contains(&fx.real_ca().display().to_string()),
        "the CA from the real environment's HOME should have been installed\n\
         certutil log:\n{log}\noutput: {text}"
    );
}

#[test]
fn a_trust_store_mutation_needs_confirmation() {
    // stdin is not a terminal under `Command::output()`, and no --yes is
    // passed: the command must refuse rather than silently install.
    let fx = Fixture::new();
    let out = fx.run(&[]);
    let text = combined(&out);

    assert_eq!(
        out.status.code(),
        Some(1),
        "an unconfirmed trust mutation must exit 1; output: {text}"
    );
    assert!(
        fx.log().is_empty(),
        "certutil ran without confirmation:\n{}",
        fx.log()
    );
    assert!(
        text.contains("SHA-256"),
        "the user must be shown the fingerprint they are being asked about; got: {text}"
    );
}

#[test]
fn a_changed_ca_is_never_installed_unattended() {
    let fx = Fixture::new();
    // First run pins the current CA.
    let first = fx.run(&["--yes"]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "first run should succeed; output: {}",
        combined(&first)
    );
    let pinned = fs::read_to_string(fx.home().join(".suprnova").join("dev-tls-ca.sha256"))
        .expect("the fingerprint must be pinned");
    assert!(!pinned.trim().is_empty(), "pin must not be empty");

    // Now the CA underneath changes, and --yes must not be enough.
    fs::write(fx.real_ca(), pem(0x33)).expect("swap the CA");
    fs::remove_file(fx.certutil_log()).ok();

    let out = fx.run(&["--yes"]);
    let text = combined(&out);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a substituted CA must not install under --yes; output: {text}"
    );
    assert!(
        fx.log().is_empty(),
        "certutil ran on a substituted CA:\n{}",
        fx.log()
    );
    assert!(
        text.contains("changed"),
        "the refusal must name the cause; got: {text}"
    );
}

#[test]
fn a_malformed_ca_is_refused_before_any_mutation() {
    let fx = Fixture::new();
    fs::write(fx.real_ca(), "not a certificate at all").expect("clobber the CA");

    let out = fx.run(&["--yes"]);
    let text = combined(&out);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a malformed CA must exit 1; output: {text}"
    );
    assert!(
        fx.log().is_empty(),
        "certutil ran on a malformed CA:\n{}",
        fx.log()
    );
    assert!(!text.contains("panicked"), "must NOT panic; got: {text}");
}

#[test]
fn a_symlinked_ca_is_refused_before_any_mutation() {
    let fx = Fixture::new();
    fs::remove_file(fx.real_ca()).expect("remove real ca");
    std::os::unix::fs::symlink(fx.evil_ca(), fx.real_ca()).expect("plant symlink");

    let out = fx.run(&["--yes"]);
    let text = combined(&out);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a symlinked CA must exit 1; output: {text}"
    );
    assert!(
        fx.log().is_empty(),
        "certutil ran on a symlinked CA:\n{}",
        fx.log()
    );
    assert!(
        text.contains("symlink"),
        "the refusal must name the cause; got: {text}"
    );
}
