//! `x2k init`: asks a few questions and writes the config file.
//!
//! The prompts are thin; the rules (validation, Gmail defaults) live in pure
//! functions so they can be tested without a terminal.

use std::path::Path;

use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, Password, Select};

use crate::app::AppError;
use crate::config::{
    Config, ConfigError, Security, SmtpConfig, default_config_path, looks_like_email,
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
    pub password: String,
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
            password: answers.password.trim().to_string(),
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

    let password = Password::with_theme(&theme)
        .with_prompt("SMTP password (hidden)")
        .interact()
        .map_err(prompt_error)?;

    let config = build_config(Answers {
        kindle_email,
        sender_email,
        password,
        server,
    })?;
    config.save(&path)?;
    Ok(next_steps(&path, &config.smtp.from))
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
            password: "abcd efgh ijkl mnop".to_string(),
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
        assert_eq!(config.smtp.password, "abcd efgh ijkl mnop");
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
            password: " secret ".to_string(),
            ..gmail_answers()
        };

        let config = build_config(answers).unwrap();

        assert_eq!(config.kindle_email, "me_123@kindle.com");
        assert_eq!(config.smtp.username, "me@gmail.com");
        assert_eq!(config.smtp.password, "secret");
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
            password: "   ".to_string(),
            ..gmail_answers()
        };

        assert!(matches!(
            build_config(answers),
            Err(ConfigError::Invalid(_))
        ));
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
