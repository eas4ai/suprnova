//! The counter renders through its preview page as a browser receives it:
//! the test starts this application's own server on a free port, then reads
//! the page and the counter's stylesheet.

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The preview server, stopped when the test ends, failed or not.
struct Server {
    child: Child,
    port: u16,
    log: PathBuf,
    database: PathBuf,
}

impl Server {
    fn start() -> Self {
        let port = TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .expect("a free port")
            .port();
        let stem = format!("{package}-test-{}-{port}", std::process::id());
        let log = std::env::temp_dir().join(format!("{stem}.log"));
        let database = std::env::temp_dir().join(format!("{stem}.db"));
        let child = Command::new(env!("CARGO_BIN_EXE_{package}"))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .env("SERVER_HOST", "127.0.0.1")
            .env("SERVER_PORT", port.to_string())
            .env(
                "DATABASE_URL",
                format!("sqlite://{}?mode=rwc", database.display()),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).expect("the server log"))
            .spawn()
            .expect("start the preview server");
        let server = Server {
            child,
            port,
            log,
            database,
        };
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut ready = false;
        while !ready && Instant::now() < deadline {
            ready = TcpStream::connect(("127.0.0.1", port)).is_ok();
            if !ready {
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        assert!(ready, "the preview server did not start:\n{}", server.log());
        server
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// One `GET`, the whole response as text.
    fn get(&self, path: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .expect("a read timeout");
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .expect("send the request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read the response");
        response
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.log);
        let _ = std::fs::remove_file(&self.database);
    }
}

#[test]
fn the_counter_renders_through_its_preview_page() {
    let server = Server::start();
    let page = server.get("/preview/counter");
    assert!(page.starts_with("HTTP/1.1 200"), "{page}\n{}", server.log());
    assert!(
        page.contains("data-suprnova-live-document-key=\"{namespace}-counter\""),
        "{page}"
    );
    assert!(
        page.contains("class=\"{namespace}-counter-value\">0</output>"),
        "{page}"
    );
    assert!(page.contains(">Add one</button>"), "{page}");
    assert!(
        page.contains("href=\"/{namespace}-ui/counter/counter.css\""),
        "{page}"
    );
    let stylesheet = server.get("/{namespace}-ui/counter/counter.css");
    assert!(stylesheet.starts_with("HTTP/1.1 200"), "{stylesheet}");
    assert!(stylesheet.contains(".{namespace}-counter-value"), "{stylesheet}");
}
