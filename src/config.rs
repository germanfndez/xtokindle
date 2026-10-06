//! User configuration: where the Kindle address and SMTP settings live.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Environment variable that overrides the SMTP password stored in the file.
pub const PASSWORD_ENV: &str = "X2K_SMTP_PASSWORD";

/// Everything that can go wrong while loading or saving the config.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config file not found at {}; run `x2k init` to create it", .0.display())]
    NotFound(PathBuf),
    #[error("failed to access the config file: {0}")]
    Read(String),
    #[error("invalid config file: {0}")]
    Parse(String),
    #[error("invalid config: {0}")]
    Invalid(String),
    #[error("{0}")]
    PasswordCommand(String),
}

/// How to secure the SMTP connection.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// Implicit TLS (usually port 465).
    Tls,
    /// Plain connection upgraded with STARTTLS (usually port 587).
    StartTls,
    /// No encryption. Only for local testing.
    None,
}

/// Where the SMTP password comes from. Serialized as `password` or `password_command`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum PasswordSource {
    /// The password itself (from `X2K_SMTP_PASSWORD` or the `password` key).
    #[serde(rename = "password")]
    Plain(String),
    /// A shell command that prints the password (the `password_command` key).
    #[serde(rename = "password_command")]
    Command(String),
}

/// SMTP server settings, with every default already filled in.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(flatten)]
    pub password: PasswordSource,
    pub from: String,
    pub security: Security,
}

/// The whole configuration.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Config {
    pub kindle_email: String,
    pub smtp: SmtpConfig,
}

/// The file as written by the user: optional fields are not resolved yet.
#[derive(Deserialize)]
struct RawConfig {
    kindle_email: String,
    smtp: RawSmtp,
}

#[derive(Deserialize)]
struct RawSmtp {
    host: String,
    port: u16,
    username: String,
    password: Option<String>,
    password_command: Option<String>,
    from: Option<String>,
    security: Option<Security>,
}

/// Picks the config file location from the environment values (passed in so tests stay pure).
///
/// Order: `X2K_CONFIG`, then `$XDG_CONFIG_HOME/x2k/config.toml`, then `$HOME/.config/x2k/config.toml`.
pub fn config_path(
    x2k_config: Option<&str>,
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> Option<PathBuf> {
    if let Some(path) = non_empty(x2k_config) {
        return Some(PathBuf::from(path));
    }
    if let Some(dir) = non_empty(xdg_config_home) {
        return Some(Path::new(dir).join("x2k/config.toml"));
    }
    non_empty(home).map(|dir| Path::new(dir).join(".config/x2k/config.toml"))
}

/// An empty environment value counts as "not set".
fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.is_empty())
}

/// `config_path` using the real environment.
pub fn default_config_path() -> Option<PathBuf> {
    let var = |name| std::env::var(name).ok();
    config_path(
        var("X2K_CONFIG").as_deref(),
        var("XDG_CONFIG_HOME").as_deref(),
        var("HOME").as_deref(),
    )
}

impl Config {
    /// Reads and validates the config file. `X2K_SMTP_PASSWORD` overrides the file's password.
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => ConfigError::NotFound(path.to_path_buf()),
            _ => ConfigError::Read(format!("{}: {e}", path.display())),
        })?;
        Config::from_toml_str(&text, std::env::var(PASSWORD_ENV).ok())
    }

    /// Parses and validates TOML text. `env_password`, when set, wins over the file.
    pub fn from_toml_str(text: &str, env_password: Option<String>) -> Result<Config, ConfigError> {
        let raw: RawConfig = toml::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))?;

        if raw.smtp.password.is_some() && raw.smtp.password_command.is_some() {
            return Err(ConfigError::Invalid(
                "set only one of `password` and `password_command` in [smtp], not both".to_string(),
            ));
        }
        // Precedence: environment variable, then `password`, then `password_command`.
        let password = match (
            env_password.filter(|password| !password.is_empty()),
            raw.smtp.password,
            raw.smtp.password_command,
        ) {
            (Some(env), _, _) => PasswordSource::Plain(env),
            (None, Some(password), _) => PasswordSource::Plain(password),
            (None, None, Some(command)) => PasswordSource::Command(command),
            (None, None, None) => PasswordSource::Plain(String::new()),
        };
        let security = raw.smtp.security.unwrap_or(if raw.smtp.port == 465 {
            Security::Tls
        } else {
            Security::StartTls
        });
        let from = raw.smtp.from.unwrap_or_else(|| raw.smtp.username.clone());

        let config = Config {
            kindle_email: raw.kindle_email,
            smtp: SmtpConfig {
                host: raw.smtp.host,
                port: raw.smtp.port,
                username: raw.smtp.username,
                password,
                from,
                security,
            },
        };
        config.validate()?;
        Ok(config)
    }

    /// Checks the values that would make sending fail later (bad emails, no password, port 0).
    pub fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |message: &str| Err(ConfigError::Invalid(message.to_string()));

        if !looks_like_email(&self.kindle_email) {
            return invalid("kindle_email must look like name@kindle.com");
        }
        if !looks_like_email(&self.smtp.username) {
            return invalid("smtp.username must be an email address");
        }
        if !looks_like_email(&self.smtp.from) {
            return invalid("smtp.from must be an email address");
        }
        match &self.smtp.password {
            PasswordSource::Plain(password) if password.is_empty() => {
                return invalid(&format!(
                    "smtp password is missing: set `password` or `password_command` in the file, or {PASSWORD_ENV}"
                ));
            }
            PasswordSource::Command(command) if command.trim().is_empty() => {
                return invalid("smtp.password_command must not be empty");
            }
            _ => {}
        }
        if self.smtp.port == 0 {
            return invalid("smtp.port must be greater than 0");
        }
        Ok(())
    }

    /// Serializes the config as TOML (with all defaults written out).
    pub fn to_toml_string(&self) -> Result<String, ConfigError> {
        toml::to_string(self).map_err(|e| ConfigError::Parse(e.to_string()))
    }

    /// Writes the config, creating parent folders. On Unix the file is readable only by you.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let read_error = |e: std::io::Error| ConfigError::Read(format!("{}: {e}", path.display()));
        let text = self.to_toml_string()?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(read_error)?;
        }
        write_private(path, &text).map_err(read_error)
    }
}

/// Hint appended to every password command failure.
const PASSWORD_MANAGER_HINT: &str = "If you use a password manager CLI, make sure it is unlocked (e.g. `bw unlock` and export BW_SESSION)";

/// How many characters of the command's stderr go into an error message.
const STDERR_LIMIT: usize = 300;

/// Gets the actual password. `run` executes a shell command and returns its stdout,
/// or the reason it failed (injected so tests need no real commands).
///
/// Only trailing `\r`/`\n` are trimmed: app passwords may contain spaces.
pub fn resolve_password(
    source: &PasswordSource,
    run: impl Fn(&str) -> Result<String, String>,
) -> Result<String, ConfigError> {
    let command = match source {
        PasswordSource::Plain(password) => return Ok(password.clone()),
        PasswordSource::Command(command) => command,
    };
    let fail = |reason: &str| {
        ConfigError::PasswordCommand(format!(
            "password_command `{command}` failed: {reason}. {PASSWORD_MANAGER_HINT}"
        ))
    };

    let output = run(command).map_err(|reason| fail(&reason))?;
    let password = output.trim_end_matches(['\r', '\n']);
    if password.is_empty() {
        return Err(fail("it printed nothing"));
    }
    Ok(password.to_string())
}

/// Runs `command` with `sh -c`, without stdin. Returns its stdout.
///
/// The error says why it failed (exit status and a short piece of stderr).
/// It never contains stdout, because stdout may hold the password.
pub fn run_shell_command(command: &str) -> Result<String, String> {
    use std::process::{Command, Stdio};

    let output = Command::new("sh")
        .args(["-c", command])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not start it: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr: String = stderr.trim().chars().take(STDERR_LIMIT).collect();
        return Err(format!("it exited with {}: {stderr}", output.status));
    }
    String::from_utf8(output.stdout).map_err(|_| "its output is not valid UTF-8".to_string())
}

/// One `@` with something on both sides. Good enough to catch typos.
pub fn looks_like_email(value: &str) -> bool {
    let mut parts = value.split('@');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(name), Some(domain), None) if !name.is_empty() && !domain.is_empty()
    )
}

/// Writes the file so only the owner can read it (the file holds a password).
#[cfg(unix)]
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    // `mode` applies to new files; `set_permissions` also tightens an existing one.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(text.as_bytes())
}

#[cfg(not(unix))]
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
kindle_email = "you_abc@kindle.com"

[smtp]
host = "smtp.gmail.com"
port = 465
username = "you@gmail.com"
password = "app password"
from = "sender@gmail.com"
security = "starttls"
"#;

    const MINIMAL: &str = r#"
kindle_email = "you_abc@kindle.com"

[smtp]
host = "smtp.example.com"
port = 587
username = "you@example.com"
password = "secret"
"#;

    #[test]
    fn parses_a_full_config() {
        let config = Config::from_toml_str(FULL, None).unwrap();

        assert_eq!(config.kindle_email, "you_abc@kindle.com");
        assert_eq!(config.smtp.host, "smtp.gmail.com");
        assert_eq!(config.smtp.port, 465);
        assert_eq!(config.smtp.username, "you@gmail.com");
        assert_eq!(
            config.smtp.password,
            PasswordSource::Plain("app password".into())
        );
        assert_eq!(config.smtp.from, "sender@gmail.com");
        assert_eq!(config.smtp.security, Security::StartTls);
    }

    #[test]
    fn defaults_from_to_username_and_security_by_port() {
        let config = Config::from_toml_str(MINIMAL, None).unwrap();
        assert_eq!(config.smtp.from, "you@example.com");
        assert_eq!(config.smtp.security, Security::StartTls);

        let tls = MINIMAL.replace("587", "465");
        assert_eq!(
            Config::from_toml_str(&tls, None).unwrap().smtp.security,
            Security::Tls
        );
    }

    #[test]
    fn parses_security_none() {
        let text = format!("{MINIMAL}security = \"none\"\n");

        let config = Config::from_toml_str(&text, None).unwrap();

        assert_eq!(config.smtp.security, Security::None);
    }

    #[test]
    fn env_password_overrides_the_file() {
        let config = Config::from_toml_str(FULL, Some("from env".into())).unwrap();

        assert_eq!(
            config.smtp.password,
            PasswordSource::Plain("from env".into())
        );
    }

    #[test]
    fn env_password_can_replace_a_missing_one() {
        let text = MINIMAL.replace("password = \"secret\"\n", "");

        let config = Config::from_toml_str(&text, Some("from env".into())).unwrap();

        assert_eq!(
            config.smtp.password,
            PasswordSource::Plain("from env".into())
        );
    }

    const WITH_COMMAND: &str = r#"
kindle_email = "you_abc@kindle.com"

[smtp]
host = "smtp.example.com"
port = 587
username = "you@example.com"
password_command = "bw get password x2k-gmail"
"#;

    #[test]
    fn parses_a_password_command() {
        let config = Config::from_toml_str(WITH_COMMAND, None).unwrap();

        assert_eq!(
            config.smtp.password,
            PasswordSource::Command("bw get password x2k-gmail".to_string())
        );
    }

    #[test]
    fn the_file_password_wins_over_nothing_and_is_plain() {
        let config = Config::from_toml_str(MINIMAL, None).unwrap();

        assert_eq!(config.smtp.password, PasswordSource::Plain("secret".into()));
    }

    #[test]
    fn env_password_wins_over_the_command() {
        let config = Config::from_toml_str(WITH_COMMAND, Some("from env".into())).unwrap();

        assert_eq!(
            config.smtp.password,
            PasswordSource::Plain("from env".to_string())
        );
    }

    #[test]
    fn password_and_password_command_together_are_invalid() {
        let text = format!("{MINIMAL}password_command = \"echo hi\"\n");

        let error = Config::from_toml_str(&text, None).unwrap_err();

        assert!(matches!(
            &error,
            ConfigError::Invalid(message)
                if message.contains("password") && message.contains("password_command")
        ));
    }

    #[test]
    fn an_empty_password_command_is_invalid() {
        let text = WITH_COMMAND.replace("bw get password x2k-gmail", "  ");

        assert!(matches!(
            Config::from_toml_str(&text, None),
            Err(ConfigError::Invalid(message)) if message.contains("password_command")
        ));
    }

    #[test]
    fn password_command_round_trips_without_a_plain_password() {
        let config = Config::from_toml_str(WITH_COMMAND, None).unwrap();

        let text = config.to_toml_string().unwrap();

        assert!(text.contains("password_command"));
        assert!(!text.contains("password = "));
        assert_eq!(Config::from_toml_str(&text, None).unwrap(), config);
    }

    #[test]
    fn resolve_returns_a_plain_password_without_running_anything() {
        let never = |_: &str| -> Result<String, String> { panic!("must not run") };

        let password = resolve_password(&PasswordSource::Plain("pw".into()), never).unwrap();

        assert_eq!(password, "pw");
    }

    #[test]
    fn resolve_runs_the_command_and_trims_only_trailing_newlines() {
        let run = |command: &str| {
            assert_eq!(command, "get-it");
            Ok("  my app password \r\n".to_string())
        };

        let password = resolve_password(&PasswordSource::Command("get-it".into()), run).unwrap();

        assert_eq!(password, "  my app password ");
    }

    #[test]
    fn resolve_rejects_empty_output() {
        let run = |_: &str| Ok("\r\n".to_string());

        let error = resolve_password(&PasswordSource::Command("get-it".into()), run).unwrap_err();

        assert!(matches!(&error, ConfigError::PasswordCommand(m) if m.contains("get-it")));
    }

    #[test]
    fn resolve_error_has_the_command_the_reason_and_a_hint() {
        let run = |_: &str| Err("exited with status 1: vault is locked".to_string());

        let error = resolve_password(&PasswordSource::Command("bw get x".into()), run).unwrap_err();

        let message = error.to_string();
        assert!(message.contains("bw get x"));
        assert!(message.contains("vault is locked"));
        assert!(message.contains("bw unlock"));
    }

    #[cfg(unix)]
    #[test]
    fn run_shell_command_returns_stdout() {
        assert_eq!(run_shell_command("printf 'secret\\n'").unwrap(), "secret\n");
    }

    #[cfg(unix)]
    #[test]
    fn run_shell_command_reports_the_exit_status() {
        let reason = run_shell_command("exit 3").unwrap_err();

        assert!(reason.contains('3'), "{reason}");
    }

    #[cfg(unix)]
    #[test]
    fn run_shell_command_reports_stderr_but_never_stdout() {
        let reason = run_shell_command("echo top-secret; echo oops >&2; exit 1").unwrap_err();

        assert!(reason.contains("oops"), "{reason}");
        assert!(!reason.contains("top-secret"), "{reason}");
    }

    #[cfg(unix)]
    #[test]
    fn run_shell_command_truncates_long_stderr() {
        let reason =
            run_shell_command("head -c 2000 /dev/zero | tr '\\0' x >&2; exit 1").unwrap_err();

        assert!(reason.len() < 500, "{}", reason.len());
    }

    #[cfg(unix)]
    #[test]
    fn run_shell_command_does_not_read_stdin() {
        assert_eq!(run_shell_command("cat").unwrap(), "");
    }

    #[test]
    fn missing_password_is_invalid() {
        let text = MINIMAL.replace("password = \"secret\"\n", "");

        let error = Config::from_toml_str(&text, None).unwrap_err();

        assert!(
            matches!(error, ConfigError::Invalid(message) if message.contains("password_command"))
        );
    }

    #[test]
    fn empty_password_is_invalid() {
        let text = MINIMAL.replace("secret", "");

        assert!(matches!(
            Config::from_toml_str(&text, None),
            Err(ConfigError::Invalid(_))
        ));
    }

    #[test]
    fn invalid_emails_are_rejected() {
        for bad in ["nope", "@kindle.com", "you@", "a@b@c"] {
            let text = MINIMAL.replace("you_abc@kindle.com", bad);
            assert!(
                matches!(
                    Config::from_toml_str(&text, None),
                    Err(ConfigError::Invalid(_))
                ),
                "{bad} should be invalid"
            );
        }
        let text = MINIMAL.replace("you@example.com", "not-an-email");
        assert!(matches!(
            Config::from_toml_str(&text, None),
            Err(ConfigError::Invalid(_))
        ));
    }

    #[test]
    fn port_zero_is_invalid() {
        let text = MINIMAL.replace("587", "0");

        assert!(matches!(
            Config::from_toml_str(&text, None),
            Err(ConfigError::Invalid(message)) if message.contains("port")
        ));
    }

    #[test]
    fn broken_toml_is_a_parse_error() {
        assert!(matches!(
            Config::from_toml_str("kindle_email = ", None),
            Err(ConfigError::Parse(_))
        ));
        assert!(matches!(
            Config::from_toml_str("kindle_email = \"a@b.c\"", None),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn path_precedence() {
        let all = config_path(Some("/c.toml"), Some("/xdg"), Some("/home/me"));
        assert_eq!(all, Some(PathBuf::from("/c.toml")));

        let xdg = config_path(None, Some("/xdg"), Some("/home/me"));
        assert_eq!(xdg, Some(PathBuf::from("/xdg/x2k/config.toml")));

        let home = config_path(None, None, Some("/home/me"));
        assert_eq!(
            home,
            Some(PathBuf::from("/home/me/.config/x2k/config.toml"))
        );

        assert_eq!(config_path(None, None, None), None);
    }

    #[test]
    fn empty_env_values_count_as_unset() {
        let path = config_path(Some(""), Some(""), Some("/home/me"));

        assert_eq!(
            path,
            Some(PathBuf::from("/home/me/.config/x2k/config.toml"))
        );
    }

    #[test]
    fn round_trips_through_toml() {
        let config = Config::from_toml_str(FULL, None).unwrap();

        let text = config.to_toml_string().unwrap();

        assert_eq!(Config::from_toml_str(&text, None).unwrap(), config);
    }

    #[test]
    fn load_reports_a_missing_file_with_a_hint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.toml");

        let error = Config::load(&path).unwrap_err();

        assert!(matches!(error, ConfigError::NotFound(_)));
        assert!(error.to_string().contains("x2k init"));
    }

    #[test]
    fn save_creates_folders_and_load_reads_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/x2k/config.toml");
        let config = Config::from_toml_str(FULL, None).unwrap();

        config.save(&path).unwrap();

        assert_eq!(Config::load(&path).unwrap(), config);
    }

    #[cfg(unix)]
    #[test]
    fn save_makes_the_file_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // A pre-existing, world-readable file must be tightened too.
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        Config::from_toml_str(FULL, None)
            .unwrap()
            .save(&path)
            .unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
