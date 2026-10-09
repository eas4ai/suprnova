//! Throwaway `redis-server`s on free ports, stopped when dropped, for the
//! two configurations the standing Redis the gate uses does not have: one
//! that requires a password (the standing one lets the default user in
//! without one, so a client that drops the credentials of its URL still
//! works against it), and one that speaks only TLS, with a certificate
//! authority and a server certificate `openssl` makes for the run.
//! `redis-server` and `openssl` must be on the `PATH`.
//!
//! Shared across test binaries via `#[path]`.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A password-protected `redis-server`, killed when dropped.
pub struct PasswordRedis {
    /// The port it listens on, on 127.0.0.1.
    pub port: u16,
    child: Child,
    _dir: tempfile::TempDir,
}

impl Drop for PasswordRedis {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl PasswordRedis {
    /// Start a server whose default user needs `password`.
    pub fn start(password: &str) -> Self {
        let dir = tempfile::tempdir().expect("tempdir for redis-server");
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind a free port")
            .local_addr()
            .expect("local addr")
            .port();
        let child = Command::new("redis-server")
            .args([
                "--port",
                &port.to_string(),
                "--bind",
                "127.0.0.1",
                "--requirepass",
                password,
                "--save",
                "",
                "--appendonly",
                "no",
                "--dir",
                ".",
            ])
            .current_dir(dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("redis-server could not start");
        let start = Instant::now();
        while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "the redis-server never listened"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        Self {
            port,
            child,
            _dir: dir,
        }
    }
}

/// A TLS-only `redis-server`, killed when dropped. A client trusts its
/// certificate authority through `ca`.
pub struct TlsRedis {
    pub port: u16,
    pub ca: PathBuf,
    child: Child,
    _dir: tempfile::TempDir,
}

impl Drop for TlsRedis {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Run `program` in `dir` and fail the test unless it succeeds.
fn run(dir: &Path, program: &str, args: &[&str]) {
    let output = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|error| panic!("{program} could not run: {error}"));
    assert!(
        output.status.success(),
        "{program} {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

impl TlsRedis {
    /// Make a certificate authority and a certificate for `127.0.0.1`, and
    /// start a server that accepts only TLS connections presenting it.
    pub fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        run(
            path,
            "openssl",
            &[
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-keyout",
                "ca.key",
                "-out",
                "ca.crt",
                "-days",
                "2",
                "-subj",
                "/CN=suprnova-test-ca",
            ],
        );
        run(
            path,
            "openssl",
            &[
                "req",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-keyout",
                "server.key",
                "-out",
                "server.csr",
                "-subj",
                "/CN=127.0.0.1",
            ],
        );
        std::fs::write(path.join("server.ext"), "subjectAltName=IP:127.0.0.1\n").unwrap();
        run(
            path,
            "openssl",
            &[
                "x509",
                "-req",
                "-in",
                "server.csr",
                "-CA",
                "ca.crt",
                "-CAkey",
                "ca.key",
                "-CAcreateserial",
                "-out",
                "server.crt",
                "-days",
                "2",
                "-extfile",
                "server.ext",
            ],
        );
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let child = Command::new("redis-server")
            .args([
                "--port",
                "0",
                "--tls-port",
                &port.to_string(),
                "--bind",
                "127.0.0.1",
                "--tls-cert-file",
                "server.crt",
                "--tls-key-file",
                "server.key",
                "--tls-ca-cert-file",
                "ca.crt",
                "--tls-auth-clients",
                "no",
                "--save",
                "",
                "--appendonly",
                "no",
                "--dir",
                ".",
            ])
            .current_dir(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("redis-server could not start");
        let start = Instant::now();
        while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "the TLS redis-server never listened"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        Self {
            port,
            ca: path.join("ca.crt"),
            child,
            _dir: dir,
        }
    }
}
