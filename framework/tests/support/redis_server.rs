//! A throwaway `redis-server` on a free port that requires a password,
//! stopped when dropped. The standing Redis the gate uses lets the default
//! user in without one, so a client that drops the credentials of its URL
//! still works against it; this server refuses such a client.
//! `redis-server` must be on the `PATH`.
//!
//! Shared across test binaries via `#[path]`.

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
