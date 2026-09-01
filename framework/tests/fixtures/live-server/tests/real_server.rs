use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

struct ServerProcess {
    child: Child,
}

impl ServerProcess {
    fn start(port: u16) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_suprnova-live-server-fixture"))
            .env("APP_ENV", "development")
            .env("RUST_LOG", "error")
            .env("SUPRNOVA_LIVE_FIXTURE_PORT", port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start downstream Live fixture");
        Self { child }
    }

    fn terminate(&mut self) {
        let status = Command::new("kill")
            .arg("-TERM")
            .arg(self.child.id().to_string())
            .status()
            .expect("signal fixture server");
        assert!(status.success(), "SIGTERM delivery failed");

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().expect("poll fixture server") {
                assert!(status.success(), "fixture server did not shut down cleanly");
                return;
            }
            assert!(Instant::now() < deadline, "fixture server ignored SIGTERM");
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn available_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve fixture port");
    listener.local_addr().expect("fixture address").port()
}

fn request(port: u16, message: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect fixture server");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set fixture read timeout");
    stream
        .write_all(message.as_bytes())
        .expect("write fixture request");
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .expect("read fixture response");
    String::from_utf8(bytes).expect("fixture response UTF-8")
}

fn get_document_when_ready(port: u16) -> String {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(mut stream) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .expect("set readiness timeout");
                stream
                    .write_all(
                        b"GET /counter HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
                    )
                    .expect("write document request");
                let mut bytes = Vec::new();
                stream
                    .read_to_end(&mut bytes)
                    .expect("read document response");
                return String::from_utf8(bytes).expect("document response UTF-8");
            }
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Err(error) => panic!("fixture server did not become ready: {error}"),
        }
    }
}

fn response_body(response: &str) -> &str {
    response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .expect("HTTP response boundary")
}

fn html_attribute<'html>(html: &'html str, name: &str) -> &'html str {
    let prefix = format!("{name}=\"");
    let start = html
        .find(&prefix)
        .map(|index| index + prefix.len())
        .unwrap_or_else(|| panic!("missing HTML attribute {name}"));
    let tail = &html[start..];
    let end = tail.find('"').expect("terminated HTML attribute");
    &tail[..end]
}

fn decode_base64url_no_pad(input: &str) -> Vec<u8> {
    let mut accumulator = 0_u32;
    let mut bits = 0_u8;
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => panic!("invalid base64url fixture byte"),
        };
        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((accumulator >> bits) as u8);
            accumulator &= (1_u32 << bits) - 1;
        }
    }
    assert_eq!(accumulator, 0, "non-canonical base64url tail");
    output
}

#[test]
fn downstream_suprnova_only_binary_serves_document_action_and_clean_shutdown() {
    let port = available_port();
    let mut server = ServerProcess::start(port);

    let document_response = get_document_when_ready(port);
    assert!(
        document_response.starts_with("HTTP/1.1 200"),
        "document response: {document_response}"
    );
    let document = response_body(&document_response);
    assert!(document.contains("<button id=\"counter\">0</button>"));
    let encoded_seed = html_attribute(document, "data-suprnova-live-snapshot");
    let seed = String::from_utf8(decode_base64url_no_pad(encoded_seed))
        .expect("decoded seed snapshot UTF-8");

    let update = format!(
        r#"{{"base_revision":"0","component":"fixtures.counter","correlation_id":"AAECAwQFBgcICQoLDA0ODw","extensions":{{"x_suprnova_live_document_key_v1":"fixture-counter"}},"idempotency_key":"EBESExQVFhcYGRobHB0eHw","model_proposals":{{}},"operations":[{{"arguments":{{}},"kind":"invoke_action","name":"increment"}}],"protocol_version":1,"runtime_contract_version":1,"snapshot":{{"browser_nonce":"ICEiIyQlJicoKSorLC0uLw","envelope":{seed},"kind":"seed_promotion"}},"snapshot_schema_version":1}}"#,
    );
    let post = format!(
        "POST /__live/v1/action HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/vnd.suprnova.live+json; charset=utf-8; version=1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        update.len(),
        update,
    );
    let action_response = request(port, &post);
    assert!(
        action_response.starts_with("HTTP/1.1 200"),
        "action response: {action_response}"
    );
    assert!(response_body(&action_response).contains("<button id=\\\"counter\\\">1</button>"));

    server.terminate();
}
