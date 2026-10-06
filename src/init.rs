//! `x2k init`: asks a few questions and writes the config file.
//!
//! The prompts are thin; the rules (validation, Gmail defaults) live in pure
//! functions so they can be tested without a terminal.

use std::path::Path;

use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, Password, Select};

use crate::app::AppError;
use crate::config::{
    Config, ConfigError, PasswordSource, Security, SmtpConfig, default_config_path,
    looks_like_email, resolve_password, run_shell_command,
};

const GMAIL_HOST: &str = "smtp.gmail.com";
const GMAIL_PORT: u16 = 465;

/// Which SMTP server sends the emails.
#[derive(Debug, Clone, PartialEq)]
pub enum Server {
    /// Gmail: `smtp.gmail.com`, port 465, implicit TLS.
    Gmail,
    Custom {
        host: String,
        port: u16,
        security: Security,
    },
}

/// Everything the user typed, before validation.
#[derive(Debug, Clone, PartialEq)]
pub struct Answers {
    pub kindle_email: String,
    pub sender_email: String,
    pub password: PasswordSource,
    pub server: Server,
}

/// Turns the answers into a validated `Config`. The sender email is also the SMTP username.
pub fn build_config(answers: Answers) -> Result<Config, ConfigError> {
    let sender = answers.sender_email.trim().to_string();
    let (host, port, security) = match answers.server {
        Server::Gmail => (GMAIL_HOST.to_string(), GMAIL_PORT, Security::Tls),
        Server::Custom {
            host,
            port,
            security,
        } => (host.trim().to_string(), port, security),
    };

    let config = Config {
        kindle_email: answers.kindle_email.trim().to_string(),
        smtp: SmtpConfig {
            host,
            port,
            username: sender.clone(),
            password: match answers.password {
                PasswordSource::Plain(password) => {
                    PasswordSource::Plain(password.trim().to_string())
                }
                PasswordSource::Command(command) => {
                    PasswordSource::Command(command.trim().to_string())
                }
            },
            from: sender,
            security,
        },
    };
    config.validate()?;
    Ok(config)
}

/// Prompt validator: the message is shown to the user when the value is rejected.
fn validate_email(value: &str) -> Result<(), String> {
    if looks_like_email(value.trim()) {
        Ok(())
    } else {
        Err("that does not look like an email address".to_string())
    }
}

/// The security options shown in the prompt, in order.
const SECURITY_CHOICES: [(&str, Security); 3] = [
    ("TLS (usually port 465)", Security::Tls),
    ("STARTTLS (usually port 587)", Security::StartTls),
    ("None (local testing only)", Security::None),
];

/// Examples shown when the user chooses to run a password manager command.
const PASSWORD_COMMAND_EXAMPLES: [&str; 3] = [
    "bw get password x2k-gmail",
    "op read \"op://Private/x2k/password\"",
    "security find-generic-password -s x2k -w",
];

/// How the password is provided, as shown in the prompt (same order as the labels).
const PASSWORD_CHOICES: [&str; 2] = [
    "Store it in the config file",
    "Run a password manager command",
];

/// What to tell the user after test-running a password command. Never includes the value.
fn command_test_message(result: Result<String, ConfigError>) -> String {
    match result {
        Ok(_) => "The command worked.".to_string(),
        Err(error) => format!("The command failed (you can still save it): {error}"),
    }
}

/// Which security option to preselect for a port.
fn default_security_index(port: u16) -> usize {
    if port == GMAIL_PORT { 0 } else { 1 }
}

/// What the user must do after the config is saved.
pub fn next_steps(path: &Path, sender_email: &str) -> String {
    format!(
        "Saved the config to {}\n\n\
One more step, or Amazon will silently drop your documents:\n\
  Amazon > Manage Your Content and Devices > Preferences > Personal Document Settings\n\
  > Approved Personal Document E-mail List > add {sender_email}\n\n\
Then try: x2k <article-url> --dry-run",
        path.display()
    )
}

/// Runs the interactive setup. Returns the text to print when it finishes.
pub fn run() -> Result<String, AppError> {
    let path = default_config_path().ok_or_else(|| {
        ConfigError::Read("cannot locate the config file: set X2K_CONFIG or HOME".to_string())
    })?;
    let theme = ColorfulTheme::default();

    if path.exists()
        && !confirm(
            &theme,
            &format!("{} already exists. Overwrite it?", path.display()),
            false,
        )?
    {
        return Ok("Nothing changed.".to_string());
    }

    println!("Find your Kindle address at Amazon > Manage Your Content and Devices >");
    println!("Preferences > Personal Document Settings > Send-to-Kindle E-Mail Settings.");
    let kindle_email = ask_email(&theme, "Kindle email (name@kindle.com)")?;

    let sender_email = ask_email(
        &theme,
        "Sender email (the account that will send the emails)",
    )?;

    let server = if confirm(&theme, "Use Gmail (smtp.gmail.com:465)?", true)? {
        println!("Gmail needs an app password (2-Step Verification must be on):");
        println!("https://myaccount.google.com/apppasswords");
        Server::Gmail
    } else {
        ask_custom_server(&theme)?
    };

    let password = ask_password(&theme)?;

    let config = build_config(Answers {
        kindle_email,
        sender_email,
        password,
        server,
    })?;
    config.save(&path)?;
    Ok(next_steps(&path, &config.smtp.from))
}

fn ask_password(theme: &ColorfulTheme) -> Result<PasswordSource, AppError> {
    let choice = Select::with_theme(theme)
        .with_prompt("How should x2k get the SMTP password?")
        .items(PASSWORD_CHOICES)
        .default(0)
        .interact()
        .map_err(prompt_error)?;
    if choice == 0 {
        let password = Password::with_theme(theme)
            .with_prompt("SMTP password (hidden)")
            .interact()
            .map_err(prompt_error)?;
        return Ok(PasswordSource::Plain(password));
    }

    println!("The command must print the password. Examples:");
    for example in PASSWORD_COMMAND_EXAMPLES {
        println!("  {example}");
    }
    let command: String = Input::with_theme(theme)
        .with_prompt("Password command")
        .interact_text()
        .map_err(prompt_error)?;
    let source = PasswordSource::Command(command.trim().to_string());

    if confirm(theme, "Run it now to check that it works?", true)? {
        println!(
            "{}",
            command_test_message(resolve_password(&source, run_shell_command))
        );
    }
    Ok(source)
}

fn ask_email(theme: &ColorfulTheme, prompt: &str) -> Result<String, AppError> {
    Input::<String>::with_theme(theme)
        .with_prompt(prompt)
        .validate_with(|value: &String| validate_email(value))
        .interact_text()
        .map_err(prompt_error)
}

fn ask_custom_server(theme: &ColorfulTheme) -> Result<Server, AppError> {
    let host: String = Input::with_theme(theme)
        .with_prompt("SMTP host")
        .interact_text()
        .map_err(prompt_error)?;
    let port: u16 = Input::with_theme(theme)
        .with_prompt("SMTP port")
        .default(587)
        .interact_text()
        .map_err(prompt_error)?;
    let labels: Vec<&str> = SECURITY_CHOICES.iter().map(|(label, _)| *label).collect();
    let choice = Select::with_theme(theme)
        .with_prompt("Connection security")
        .items(&labels)
        .default(default_security_index(port))
        .interact()
        .map_err(prompt_error)?;

    Ok(Server::Custom {
        host,
        port,
        security: SECURITY_CHOICES[choice].1,
    })
}

fn confirm(theme: &ColorfulTheme, prompt: &str, default: bool) -> Result<bool, AppError> {
    Confirm::with_theme(theme)
        .with_prompt(prompt)
        .default(default)
        .interact()
        .map_err(prompt_error)
}

/// A failed prompt (e.g. no terminal attached) is reported as an I/O problem.
fn prompt_error(error: dialoguer::Error) -> AppError {
    AppError::Io(format!("could not read your answer: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigError, Security};

    fn gmail_answers() -> Answers {
        Answers {
            kindle_email: "me_123@kindle.com".to_string(),
            sender_email: "me@gmail.com".to_string(),
            password: PasswordSource::Plain("abcd efgh ijkl mnop".to_string()),
            server: Server::Gmail,
        }
    }

    #[test]
    fn gmail_uses_the_gmail_server_over_tls() {
        let config = build_config(gmail_answers()).unwrap();

        assert_eq!(config.kindle_email, "me_123@kindle.com");
        assert_eq!(config.smtp.host, "smtp.gmail.com");
        assert_eq!(config.smtp.port, 465);
        assert_eq!(config.smtp.security, Security::Tls);
        assert_eq!(config.smtp.username, "me@gmail.com");
        assert_eq!(config.smtp.from, "me@gmail.com");
        assert_eq!(
            config.smtp.password,
            PasswordSource::Plain("abcd efgh ijkl mnop".to_string())
        );
    }

    #[test]
    fn custom_server_keeps_the_given_settings() {
        let answers = Answers {
            server: Server::Custom {
                host: "mail.example.com".to_string(),
                port: 587,
                security: Security::StartTls,
            },
            ..gmail_answers()
        };

        let config = build_config(answers).unwrap();

        assert_eq!(config.smtp.host, "mail.example.com");
        assert_eq!(config.smtp.port, 587);
        assert_eq!(config.smtp.security, Security::StartTls);
    }

    #[test]
    fn answers_are_trimmed() {
        let answers = Answers {
            kindle_email: "  me_123@kindle.com ".to_string(),
            sender_email: " me@gmail.com\n".to_string(),
            password: PasswordSource::Plain(" secret ".to_string()),
            ..gmail_answers()
        };

        let config = build_config(answers).unwrap();

        assert_eq!(config.kindle_email, "me_123@kindle.com");
        assert_eq!(config.smtp.username, "me@gmail.com");
        assert_eq!(config.smtp.password, PasswordSource::Plain("secret".into()));
    }

    #[test]
    fn rejects_a_kindle_address_that_is_not_an_email() {
        let answers = Answers {
            kindle_email: "not-an-email".to_string(),
            ..gmail_answers()
        };

        assert!(matches!(
            build_config(answers),
            Err(ConfigError::Invalid(_))
        ));
    }

    #[test]
    fn rejects_an_empty_password() {
        let answers = Answers {
            password: PasswordSource::Plain("   ".to_string()),
            ..gmail_answers()
        };

        assert!(matches!(
            build_config(answers),
            Err(ConfigError::Invalid(_))
        ));
    }

    #[test]
    fn a_password_command_is_kept_instead_of_a_password() {
        let answers = Answers {
            password: PasswordSource::Command(" bw get password x2k-gmail ".to_string()),
            ..gmail_answers()
        };

        let config = build_config(answers).unwrap();

        assert_eq!(
            config.smtp.password,
            PasswordSource::Command("bw get password x2k-gmail".to_string())
        );
        let text = config.to_toml_string().unwrap();
        assert!(text.contains("password_command = \"bw get password x2k-gmail\""));
    }

    #[test]
    fn rejects_an_empty_password_command() {
        let answers = Answers {
            password: PasswordSource::Command("  ".to_string()),
            ..gmail_answers()
        };

        assert!(matches!(
            build_config(answers),
            Err(ConfigError::Invalid(_))
        ));
    }

    #[test]
    fn command_test_message_never_shows_the_password() {
        assert_eq!(
            command_test_message(Ok("hunter2".to_string())),
            "The command worked."
        );

        let failed = command_test_message(Err(ConfigError::PasswordCommand("boom".into())));
        assert!(failed.contains("boom"));
    }

    #[test]
    fn validate_email_accepts_and_rejects() {
        assert!(validate_email("a@b.com").is_ok());
        assert!(validate_email("  a@b.com ").is_ok());
        assert!(validate_email("nope").is_err());
        assert!(validate_email("").is_err());
    }

    #[test]
    fn security_choices_default_to_the_usual_one_for_the_port() {
        assert_eq!(default_security_index(465), 0);
        assert_eq!(default_security_index(587), 1);
    }

    #[test]
    fn built_config_survives_a_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x2k/config.toml");
        let config = build_config(gmail_answers()).unwrap();

        config.save(&path).unwrap();

        assert_eq!(crate::config::Config::load(&path).unwrap(), config);
    }

    #[test]
    fn next_steps_mention_the_path_and_the_approved_sender() {
        let message = next_steps(std::path::Path::new("/tmp/x2k.toml"), "me@gmail.com");

        assert!(message.contains("/tmp/x2k.toml"));
        assert!(message.contains("me@gmail.com"));
        assert!(message.contains("Approved Personal Document E-mail List"));
    }
}
