//! `rediss://` connections, against a TLS `redis-server` each test starts on
//! a free port, with a certificate authority and a server certificate
//! `openssl` makes for the run. Both tools must be on the `PATH`.

use crate::support::unique;
use serial_test::serial;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use suprnova::Redis;
use suprnova::redis::TlsCertificates;

/// A TLS-only `redis-server`, stopped when dropped.
struct TlsServer {
    port: u16,
    ca: PathBuf,
    child: Child,
    _dir: tempfile::TempDir,
}

impl Drop for TlsServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

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

fn tls_server() -> TlsServer {
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
    TlsServer {
        port,
        ca: path.join("ca.crt"),
        child,
        _dir: dir,
    }
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL; runs redis-server and openssl"]
#[serial]
async fn a_client_built_with_tls_reaches_a_tls_server() {
    let server = tls_server();
    let client = suprnova::redis::Client::build_with_tls(
        format!("rediss://127.0.0.1:{}/0", server.port),
        TlsCertificates {
            client_tls: None,
            root_cert: Some(std::fs::read(&server.ca).unwrap()),
        },
    )
    .unwrap();
    let name = unique("tls");
    Redis::define_client(&name, client);
    let redis = Redis::connection(&name).unwrap();
    redis.set("over-tls", "encrypted").await.unwrap();
    assert_eq!(
        redis.get("over-tls").await.unwrap().as_deref(),
        Some("encrypted")
    );
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL; runs redis-server and openssl"]
#[serial]
async fn a_rediss_url_is_accepted_and_an_untrusted_certificate_is_an_error() {
    // In a process of its own, so no other test has installed a crypto
    // provider: a rediss:// connection must not panic for want of one.
    let server = tls_server();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tls::child_connects_to_an_untrusted_tls_server",
            "--nocapture",
            "--ignored",
            "--test-threads=1",
        ])
        .env("SUPRNOVA_REDIS_TLS_PORT", server.port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "the child failed: {stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL; runs redis-server and openssl"]
#[serial]
async fn child_connects_to_an_untrusted_tls_server() {
    let Ok(port) = std::env::var("SUPRNOVA_REDIS_TLS_PORT") else {
        return;
    };
    let name = unique("untrusted");
    Redis::define(&name, &format!("rediss://127.0.0.1:{port}/0"))
        .expect("rediss:// is a Redis URL");
    let redis = Redis::connection(&name).unwrap();
    let error = redis
        .get("anything")
        .await
        .expect_err("the test CA is in no trust store");
    assert!(
        error.to_string().to_lowercase().contains("certificate"),
        "{error}"
    );

    // The cache driver opens its client the same way.
    let config = suprnova::CacheConfig::builder()
        .url(format!("rediss://127.0.0.1:{port}/0"))
        .build();
    let error = suprnova::RedisCache::connect(&config)
        .await
        .err()
        .expect("the test CA is in no trust store");
    assert!(
        error.to_string().to_lowercase().contains("certificate"),
        "{error}"
    );
}
