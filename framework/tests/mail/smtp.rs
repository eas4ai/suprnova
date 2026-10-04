//! SMTP transport tests.
//!
//! Most run against [`SmtpSink`], an in-process server that records the SMTP
//! commands and the message bytes. One live test needs a Mailpit (or
//! equivalent) SMTP server reachable at `MAIL_SMTP_HOST:MAIL_SMTP_PORT`
//! (defaults to 127.0.0.1:1025); run it with
//! `cargo test -p suprnova --test mail smtp:: -- --ignored`.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use suprnova::async_trait;
use suprnova::mail::smtp::SmtpMailTransport;
use suprnova::mail::transport::{MailTransport, OutgoingMessage};
use suprnova::mail::{Address, Mail, Mailable};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct LiveHello {
    msg: String,
}

#[async_trait]
impl Mailable for LiveHello {
    fn mailable_name() -> &'static str {
        "LiveHello"
    }
    fn subject(&self) -> String {
        "live-test".into()
    }
    fn text_template_source(&self) -> Option<String> {
        Some("hello".into())
    }
    fn from(&self) -> Option<Address> {
        Some("noreply@suprnova.dev".into())
    }
}

#[ignore = "requires a real SMTP server (Mailpit at 127.0.0.1:1025 by default)"]
#[tokio::test]
async fn smtp_transport_sends_through_live_server() {
    let host = std::env::var("MAIL_SMTP_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port: u16 = std::env::var("MAIL_SMTP_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1025);
    let transport = SmtpMailTransport::unencrypted(&host, port).unwrap();
    let _ = Mail::set_transport(Arc::new(transport));
    Mail::to("recipient@example.org")
        .send(LiveHello {
            msg: "hello".into(),
        })
        .await
        .unwrap();
}

/// What an [`SmtpSink`] saw: every command line, the DATA payload of each
/// message, and how many connections were opened.
#[derive(Default, Clone)]
struct SinkLog {
    commands: Vec<String>,
    data: Vec<String>,
    connections: usize,
}

/// Enough of an SMTP server for lettre's client. It accepts everything, so a
/// test can read exactly what the transport put on the wire.
#[derive(Clone, Default)]
struct SmtpSink {
    log: Arc<Mutex<SinkLog>>,
}

impl SmtpSink {
    async fn start() -> (Self, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let sink = Self::default();
        let server = sink.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let server = server.clone();
                tokio::spawn(async move { server.serve(stream).await });
            }
        });
        (sink, port)
    }

    fn snapshot(&self) -> SinkLog {
        self.log.lock().unwrap().clone()
    }

    async fn serve(&self, stream: TcpStream) {
        self.log.lock().unwrap().connections += 1;
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        if write.write_all(b"220 sink ESMTP\r\n").await.is_err() {
            return;
        }
        while let Ok(Some(line)) = lines.next_line().await {
            let command = line.to_ascii_uppercase();
            self.log.lock().unwrap().commands.push(line);
            let reply: &[u8] = if command.starts_with("EHLO") || command.starts_with("HELO") {
                b"250 sink\r\n"
            } else if command == "DATA" {
                if write.write_all(b"354 go ahead\r\n").await.is_err() {
                    return;
                }
                let mut data = String::new();
                while let Ok(Some(body_line)) = lines.next_line().await {
                    if body_line == "." {
                        break;
                    }
                    data.push_str(&body_line);
                    data.push_str("\r\n");
                }
                self.log.lock().unwrap().data.push(data);
                b"250 queued\r\n"
            } else if command == "QUIT" {
                let _ = write.write_all(b"221 bye\r\n").await;
                return;
            } else {
                b"250 OK\r\n"
            };
            if write.write_all(reply).await.is_err() {
                return;
            }
        }
    }
}

fn sink_message() -> OutgoingMessage {
    let mut msg = OutgoingMessage::new(Address::new("author@example.test"));
    msg.to = vec![Address::new("to@example.test")];
    msg.cc = vec![Address::new("cc@example.test")];
    msg.bcc = vec![Address::new("bcc@example.test")];
    msg.subject = "envelope".into();
    msg.text = Some("body".into());
    msg
}

fn commands_starting(log: &SinkLog, prefix: &str) -> Vec<String> {
    log.commands
        .iter()
        .filter(|c| c.to_ascii_uppercase().starts_with(prefix))
        .cloned()
        .collect()
}

#[tokio::test]
async fn smtp_refuses_a_header_that_would_inject_another_header() {
    // lettre writes a header name verbatim, so a CR/LF in it starts a new
    // header (here a `Reply-To` that diverts replies to the attacker).
    let (sink, port) = SmtpSink::start().await;
    let transport = SmtpMailTransport::unencrypted("127.0.0.1", port).unwrap();

    let mut header_name = sink_message();
    header_name.headers = vec![("X-A\r\nReply-To".into(), "attacker@evil.example".into())];
    let mut metadata_key = sink_message();
    metadata_key
        .metadata
        .insert("ref\r\nReply-To".into(), "attacker@evil.example".into());
    let mut header_value = sink_message();
    header_value.headers = vec![("X-A".into(), "a\r\nReply-To: attacker@evil.example".into())];

    for msg in [header_name, metadata_key, header_value] {
        let err = transport.send(&msg).await.unwrap_err();
        assert!(format!("{err}").contains("header"), "{err}");
    }
    let log = sink.snapshot();
    assert!(
        log.data.is_empty() && commands_starting(&log, "MAIL FROM").is_empty(),
        "a refused message must not be sent"
    );
    assert_eq!(
        log.connections, 0,
        "the message is refused before the transport connects"
    );
}
