//! The application flow: fetch an article, render it, then send it and/or save it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::{
    Config, ConfigError, default_config_path, resolve_password, run_shell_command,
};
use crate::render::{
    Image, RenderError, collect_image_urls, download_images, file_name, render_epub,
};
use crate::sender::{Document, SendError, Sender, SmtpSender};
use crate::source::{SourceError, SourceRegistry};
use crate::sources::x::XSource;

/// Everything that can go wrong in a run. Each variant maps to a `kind` and an exit code.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Send(#[from] SendError),
    #[error("{0}")]
    Io(String),
}

impl AppError {
    /// Stable machine-readable name, used in the `--json` error output.
    pub fn kind(&self) -> &'static str {
        match self {
            AppError::Source(SourceError::UnsupportedUrl(_)) => "unsupported_url",
            AppError::Source(SourceError::Fetch { .. }) => "fetch",
            AppError::Source(SourceError::Parse(_)) => "parse",
            AppError::Config(_) => "config",
            AppError::Send(_) => "send",
            AppError::Render(_) => "render",
            AppError::Io(_) => "io",
        }
    }

    /// Process exit code. Documented in `--help` and the README (2 is clap's usage error).
    pub fn exit_code(&self) -> i32 {
        match self.kind() {
            "unsupported_url" => 3,
            "fetch" | "parse" => 4,
            "config" => 5,
            "send" => 6,
            _ => 1,
        }
    }

    /// The error as a JSON object for `--json`.
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "status": "error",
            "kind": self.kind(),
            "message": self.to_string(),
        })
        .to_string()
    }
}

/// What the user asked for besides the URL.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Build the EPUB locally; load no config and send nothing.
    pub dry_run: bool,
    /// Where to write the EPUB (a file path or a directory).
    pub output: Option<PathBuf>,
}

/// Whether the article was emailed or only saved to disk.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Sent,
    Saved,
}

/// The result of a successful run; serialized as-is for `--json`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Outcome {
    pub status: Status,
    pub title: String,
    pub author: String,
    pub url: String,
    pub words: usize,
    pub file: Option<String>,
    pub sent_to: Option<String>,
}

impl Outcome {
    /// The outcome as a JSON object for `--json`.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("an Outcome always serializes")
    }

    /// A one-line summary for people.
    pub fn human_message(&self) -> String {
        match (&self.status, &self.sent_to, &self.file) {
            (Status::Sent, Some(email), _) => format!(
                "Sent \"{}\" by {} ({} words) to {email}",
                self.title, self.author, self.words
            ),
            (_, _, Some(file)) => format!("Saved \"{}\" to {file}", self.title),
            _ => format!("Done: \"{}\"", self.title),
        }
    }
}

/// Where and how to deliver the EPUB. Without one, the run only saves a file.
pub struct Delivery<'a> {
    pub sender: &'a dyn Sender,
    pub kindle_email: &'a str,
}

/// The one place where sources are registered. A new source is a one-line addition.
pub fn default_registry() -> SourceRegistry {
    let mut registry = SourceRegistry::new();
    registry.register(Box::new(XSource::new()));
    registry
}

/// The real run: real sources, real image downloads and, unless `--dry-run`, real SMTP.
///
/// In the normal flow the URL is checked, the config is loaded and the SMTP password
/// is resolved first (this may run `password_command`), so an unsupported URL, a missing
/// config or a failing password command fails fast, before any network request.
/// `--dry-run` never resolves the password.
pub fn run(url: &str, options: &Options) -> Result<Outcome, AppError> {
    let registry = default_registry();
    registry.find(url)?;
    if options.dry_run {
        return process(url, options, &registry, &download_images, None);
    }

    let path = default_config_path().ok_or_else(|| {
        ConfigError::Read("cannot locate the config file: set X2K_CONFIG or HOME".to_string())
    })?;
    let config = Config::load(&path)?;
    // Fail fast: a password manager that is locked should not cost a fetch.
    let password = resolve_password(&config.smtp.password, run_shell_command)?;
    let sender = SmtpSender::new(&config.smtp, &password, &config.kindle_email)?;
    let delivery = Delivery {
        sender: &sender,
        kindle_email: &config.kindle_email,
    };
    process(url, options, &registry, &download_images, Some(delivery))
}

/// The testable core: fetch, render, then send and/or save.
///
/// `download` fetches the images (injected so tests need no network).
/// With `delivery` the EPUB is emailed (and also saved when `--output` is set);
/// without it the EPUB is saved, to the current directory by default.
pub fn process(
    url: &str,
    options: &Options,
    registry: &SourceRegistry,
    download: &dyn Fn(&[String]) -> HashMap<String, Image>,
    delivery: Option<Delivery>,
) -> Result<Outcome, AppError> {
    let article = registry.find(url)?.fetch(url)?;
    let images = download(&collect_image_urls(&article));
    let document = Document {
        file_name: file_name(&article),
        bytes: render_epub(&article, &images)?,
        title: article.title.clone(),
    };

    let file = match (&delivery, &options.output) {
        (None, output) => Some(save(&document, output.as_deref())?),
        (Some(_), Some(output)) => Some(save(&document, Some(output))?),
        (Some(_), None) => None,
    };
    let sent_to = match delivery {
        Some(delivery) => {
            delivery.sender.send(&document)?;
            Some(delivery.kindle_email.to_string())
        }
        None => None,
    };

    Ok(Outcome {
        status: if sent_to.is_some() {
            Status::Sent
        } else {
            Status::Saved
        },
        words: article.word_count(),
        title: article.title,
        author: article.author,
        url: article.url,
        file,
        sent_to,
    })
}

/// Writes the document and returns the path used.
fn save(document: &Document, output: Option<&Path>) -> Result<String, AppError> {
    let path = resolve_output(output, &document.file_name);
    std::fs::write(&path, &document.bytes)
        .map_err(|e| AppError::Io(format!("failed to write {}: {e}", path.display())))?;
    Ok(path.display().to_string())
}

/// A directory gets the file name appended; a file path is used as is;
/// nothing means the current directory.
pub fn resolve_output(output: Option<&Path>, file_name: &str) -> PathBuf {
    match output {
        Some(path) if path.is_dir() => path.join(file_name),
        Some(path) => path.to_path_buf(),
        None => PathBuf::from(file_name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::article::{Article, Block, Span};
    use crate::config::ConfigError;
    use crate::render::Image;
    use crate::sender::{Document, SendError, Sender};
    use crate::source::{Source, SourceError, SourceRegistry};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::Path;

    const URL: &str = "https://fake.test/article/1";

    struct FakeSource;

    impl Source for FakeSource {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn can_handle(&self, url: &str) -> bool {
            url.starts_with("https://fake.test/")
        }
        fn fetch(&self, url: &str) -> Result<Article, SourceError> {
            Ok(Article {
                title: "Hello World".to_string(),
                author: "Ada".to_string(),
                url: url.to_string(),
                cover_image: None,
                blocks: vec![Block::Paragraph(vec![Span::plain("one two three")])],
            })
        }
    }

    /// Records every document it is asked to send.
    #[derive(Default)]
    struct FakeSender {
        sent: RefCell<Vec<Document>>,
    }

    impl Sender for FakeSender {
        fn send(&self, doc: &Document) -> Result<(), SendError> {
            self.sent.borrow_mut().push(doc.clone());
            Ok(())
        }
    }

    struct FailingSender;

    impl Sender for FailingSender {
        fn send(&self, _doc: &Document) -> Result<(), SendError> {
            Err(SendError::Smtp("boom".to_string()))
        }
    }

    fn registry() -> SourceRegistry {
        let mut registry = SourceRegistry::new();
        registry.register(Box::new(FakeSource));
        registry
    }

    fn no_images(_urls: &[String]) -> HashMap<String, Image> {
        HashMap::new()
    }

    fn options(dry_run: bool, output: Option<&Path>) -> Options {
        Options {
            dry_run,
            output: output.map(Path::to_path_buf),
        }
    }

    #[test]
    fn sends_the_epub_to_the_kindle_address() {
        let sender = FakeSender::default();
        let delivery = Delivery {
            sender: &sender,
            kindle_email: "me@kindle.com",
        };

        let outcome = process(
            URL,
            &options(false, None),
            &registry(),
            &no_images,
            Some(delivery),
        )
        .unwrap();

        assert_eq!(outcome.status, Status::Sent);
        assert_eq!(outcome.title, "Hello World");
        assert_eq!(outcome.author, "Ada");
        assert_eq!(outcome.url, URL);
        assert_eq!(outcome.words, 3);
        assert_eq!(outcome.file, None);
        assert_eq!(outcome.sent_to.as_deref(), Some("me@kindle.com"));

        let sent = sender.sent.borrow();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].file_name, "hello-world.epub");
        assert_eq!(sent[0].title, "Hello World");
        assert!(sent[0].bytes.starts_with(b"PK"), "an EPUB is a zip file");
    }

    #[test]
    fn dry_run_saves_the_file_and_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();

        let outcome = process(
            URL,
            &options(true, Some(dir.path())),
            &registry(),
            &no_images,
            None,
        )
        .unwrap();

        let expected = dir.path().join("hello-world.epub");
        assert_eq!(outcome.status, Status::Saved);
        assert_eq!(outcome.sent_to, None);
        assert_eq!(outcome.file.as_deref(), Some(expected.to_str().unwrap()));
        assert!(expected.exists());
    }

    #[test]
    fn sending_with_output_also_writes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("mine.epub");
        let sender = FakeSender::default();
        let delivery = Delivery {
            sender: &sender,
            kindle_email: "me@kindle.com",
        };

        let outcome = process(
            URL,
            &options(false, Some(&target)),
            &registry(),
            &no_images,
            Some(delivery),
        )
        .unwrap();

        assert_eq!(outcome.status, Status::Sent);
        assert_eq!(outcome.file.as_deref(), Some(target.to_str().unwrap()));
        assert!(target.exists());
        assert_eq!(sender.sent.borrow().len(), 1);
    }

    #[test]
    fn unsupported_url_is_reported_before_anything_else() {
        let error = process(
            "https://example.com/a",
            &options(true, None),
            &registry(),
            &no_images,
            None,
        )
        .unwrap_err();

        assert_eq!(error.kind(), "unsupported_url");
        assert_eq!(error.exit_code(), 3);
    }

    #[test]
    fn send_failures_are_reported() {
        let delivery = Delivery {
            sender: &FailingSender,
            kindle_email: "me@kindle.com",
        };

        let error = process(
            URL,
            &options(false, None),
            &registry(),
            &no_images,
            Some(delivery),
        )
        .unwrap_err();

        assert_eq!(error.kind(), "send");
        assert_eq!(error.exit_code(), 6);
    }

    #[test]
    fn resolve_output_defaults_to_the_file_name() {
        assert_eq!(
            resolve_output(None, "a.epub"),
            Path::new("a.epub").to_path_buf()
        );
    }

    #[test]
    fn resolve_output_joins_directories_and_keeps_file_paths() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_output(Some(dir.path()), "a.epub"),
            dir.path().join("a.epub")
        );

        let file = dir.path().join("custom.epub");
        assert_eq!(resolve_output(Some(&file), "a.epub"), file);
    }

    #[test]
    fn error_kinds_and_exit_codes() {
        let cases: Vec<(AppError, &str, i32)> = vec![
            (
                SourceError::UnsupportedUrl("u".into()).into(),
                "unsupported_url",
                3,
            ),
            (
                SourceError::Fetch {
                    url: "u".into(),
                    reason: "r".into(),
                }
                .into(),
                "fetch",
                4,
            ),
            (SourceError::Parse("p".into()).into(), "parse", 4),
            (ConfigError::Invalid("c".into()).into(), "config", 5),
            (ConfigError::PasswordCommand("p".into()).into(), "config", 5),
            (SendError::Build("s".into()).into(), "send", 6),
            (RenderError::Epub("r".into()).into(), "render", 1),
            (AppError::Io("i".into()), "io", 1),
        ];
        for (error, kind, code) in cases {
            assert_eq!(error.kind(), kind);
            assert_eq!(error.exit_code(), code);
        }
    }

    #[test]
    fn outcome_serializes_to_the_documented_json() {
        let outcome = Outcome {
            status: Status::Sent,
            title: "T".into(),
            author: "A".into(),
            url: "U".into(),
            words: 7,
            file: None,
            sent_to: Some("me@kindle.com".into()),
        };

        let json: serde_json::Value = serde_json::from_str(&outcome.to_json()).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "status": "sent", "title": "T", "author": "A", "url": "U",
                "words": 7, "file": null, "sent_to": "me@kindle.com"
            })
        );
    }

    #[test]
    fn error_serializes_to_the_documented_json() {
        let error = AppError::Source(SourceError::UnsupportedUrl("u".into()));

        let json: serde_json::Value = serde_json::from_str(&error.to_json()).unwrap();

        assert_eq!(json["status"], "error");
        assert_eq!(json["kind"], "unsupported_url");
        assert!(
            json["message"]
                .as_str()
                .unwrap()
                .contains("unsupported URL")
        );
    }

    #[test]
    fn human_messages_describe_the_result() {
        let sent = Outcome {
            status: Status::Sent,
            title: "T".into(),
            author: "A".into(),
            url: "U".into(),
            words: 1234,
            file: None,
            sent_to: Some("me@kindle.com".into()),
        };
        assert_eq!(
            sent.human_message(),
            "Sent \"T\" by A (1234 words) to me@kindle.com"
        );

        let saved = Outcome {
            status: Status::Saved,
            sent_to: None,
            file: Some("./t.epub".into()),
            ..sent
        };
        assert_eq!(saved.human_message(), "Saved \"T\" to ./t.epub");
    }

    #[test]
    fn default_registry_knows_x_urls() {
        let registry = default_registry();
        assert!(registry.find("https://x.com/user/status/1").is_ok());
        assert!(registry.find("https://example.com/a").is_err());
    }
}
