//! Delivers a document to the user's Send-to-Kindle address over SMTP.

use std::time::Duration;

use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};

use crate::config::{Security, SmtpConfig};

/// Everything that can go wrong while building or sending the email.
#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("failed to build the email: {0}")]
    Build(String),
    #[error("failed to send the email: {0}")]
    Smtp(String),
}

/// A file to deliver.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub file_name: String,
    pub bytes: Vec<u8>,
    pub title: String,
}

/// Anything that can deliver a document (SMTP in production, a fake in tests).
pub trait Sender {
    fn send(&self, doc: &Document) -> Result<(), SendError>;
}

/// Builds the email: the title as subject and the EPUB as attachment.
pub fn build_message(from: &str, to: &str, doc: &Document) -> Result<Message, SendError> {
    let build_error = |e: &dyn std::fmt::Display| SendError::Build(e.to_string());

    let from: Mailbox = from.parse().map_err(|e| build_error(&e))?;
    let to: Mailbox = to.parse().map_err(|e| build_error(&e))?;
    let epub = ContentType::parse("application/epub+zip").map_err(|e| build_error(&e))?;

    let body = SinglePart::plain(format!("{} (sent with x2k)", doc.title));
    let attachment = Attachment::new(doc.file_name.clone()).body(doc.bytes.clone(), epub);

    Message::builder()
        .from(from)
        .to(to)
        .subject(doc.title.as_str())
        .multipart(MultiPart::mixed().singlepart(body).singlepart(attachment))
        .map_err(|e| build_error(&e))
}

/// Adds a hint to SMTP errors that usually mean a credentials or sender problem.
fn describe_smtp_error(message: &str) -> String {
    let lower = message.to_lowercase();
    let looks_like_auth_failure = lower.contains("535") || lower.contains("authentication");
    if looks_like_auth_failure {
        format!(
            "{message}\nHint: check the username and password (Gmail needs an app password, \
not your account password) and make sure the sender is in your Kindle approved senders list."
        )
    } else {
        message.to_string()
    }
}

/// Sends documents to the Kindle address through an SMTP server.
pub struct SmtpSender {
    transport: SmtpTransport,
    from: String,
    to: String,
}

impl SmtpSender {
    /// Prepares the SMTP connection settings. Nothing is sent until `send` is called.
    pub fn new(smtp: &SmtpConfig, kindle_email: &str) -> Result<SmtpSender, SendError> {
        let builder = match smtp.security {
            Security::Tls => SmtpTransport::relay(&smtp.host),
            Security::StartTls => SmtpTransport::starttls_relay(&smtp.host),
            Security::None => Ok(SmtpTransport::builder_dangerous(&smtp.host)),
        }
        .map_err(|e| SendError::Build(e.to_string()))?;

        let transport = builder
            .port(smtp.port)
            .credentials(Credentials::new(
                smtp.username.clone(),
                smtp.password.clone(),
            ))
            .timeout(Some(Duration::from_secs(30)))
            .build();

        Ok(SmtpSender {
            transport,
            from: smtp.from.clone(),
            to: kindle_email.to_string(),
        })
    }
}

impl Sender for SmtpSender {
    fn send(&self, doc: &Document) -> Result<(), SendError> {
        let message = build_message(&self.from, &self.to, doc)?;
        self.transport
            .send(&message)
            .map(|_| ())
            .map_err(|e| SendError::Smtp(describe_smtp_error(&e.to_string())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;

    fn document() -> Document {
        Document {
            file_name: "my-title.epub".into(),
            bytes: b"PK fake epub".to_vec(),
            title: "My Title".into(),
        }
    }

    fn formatted(message: &Message) -> String {
        String::from_utf8_lossy(&message.formatted()).into_owned()
    }

    #[test]
    fn message_has_headers_and_the_epub_attachment() {
        let message = build_message("me@gmail.com", "you_abc@kindle.com", &document()).unwrap();

        let text = formatted(&message);

        assert!(text.contains("From: me@gmail.com"));
        assert!(text.contains("To: you_abc@kindle.com"));
        assert!(text.contains("Subject: My Title"));
        assert!(text.contains("Content-Type: application/epub+zip"));
        assert!(text.contains("filename=\"my-title.epub\""));
    }

    #[test]
    fn message_rejects_a_bad_address() {
        let result = build_message("not an address", "you_abc@kindle.com", &document());

        assert!(matches!(result, Err(SendError::Build(_))));
    }

    #[test]
    fn auth_errors_get_a_hint() {
        let hint = describe_smtp_error("permanent error (535): Username and Password not accepted");

        assert!(hint.contains("app password"));
        assert!(hint.contains("approved"));
        assert!(hint.contains("535"));
    }

    #[test]
    fn other_errors_are_left_alone() {
        assert_eq!(
            describe_smtp_error("connection refused"),
            "connection refused"
        );
    }

    /// A tiny SMTP server: accepts one message and returns everything the client sent.
    fn start_fake_smtp_server() -> (u16, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            writer.write_all(b"220 fake ESMTP\r\n").unwrap();

            let mut received = String::new();
            let mut in_data = false;
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap() > 0 {
                received.push_str(&line);
                let reply: &[u8] = if in_data {
                    if line == ".\r\n" {
                        in_data = false;
                        b"250 queued\r\n"
                    } else {
                        b""
                    }
                } else if line.starts_with("DATA") {
                    in_data = true;
                    b"354 go ahead\r\n"
                } else if line.starts_with("EHLO") {
                    b"250-fake\r\n250 AUTH PLAIN\r\n"
                } else if line.starts_with("AUTH") {
                    b"235 welcome\r\n"
                } else if line.starts_with("QUIT") {
                    writer.write_all(b"221 bye\r\n").unwrap();
                    break;
                } else {
                    b"250 ok\r\n"
                };
                writer.write_all(reply).unwrap();
                line.clear();
            }
            received
        });
        (port, handle)
    }

    #[test]
    fn smtp_sender_delivers_the_attachment_to_a_server() {
        let (port, server) = start_fake_smtp_server();
        let smtp = SmtpConfig {
            host: "127.0.0.1".into(),
            port,
            username: "me@example.com".into(),
            password: "secret".into(),
            from: "me@example.com".into(),
            security: Security::None,
        };

        let sender = SmtpSender::new(&smtp, "you_abc@kindle.com").unwrap();
        sender.send(&document()).unwrap();

        let received = server.join().unwrap();
        assert!(received.contains("RCPT TO:<you_abc@kindle.com>"));
        assert!(received.contains("filename=\"my-title.epub\""));
        assert!(received.contains("application/epub+zip"));
    }

    #[test]
    fn smtp_sender_reports_connection_failures() {
        // Bind then drop, so the port is closed.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let smtp = SmtpConfig {
            host: "127.0.0.1".into(),
            port,
            username: "me@example.com".into(),
            password: "secret".into(),
            from: "me@example.com".into(),
            security: Security::None,
        };

        let result = SmtpSender::new(&smtp, "you_abc@kindle.com")
            .unwrap()
            .send(&document());

        assert!(matches!(result, Err(SendError::Smtp(_))));
    }
}
