//! End-to-end tests that run the real `x2k` binary. Only the `#[ignore]` one uses the network.

use assert_cmd::Command;
use predicates::prelude::*;

const X_URL: &str = "https://x.com/thedankoe/status/2101361833940791607";

fn x2k() -> Command {
    Command::cargo_bin("x2k").unwrap()
}

#[test]
fn help_succeeds_and_lists_exit_codes() {
    x2k()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Exit codes"))
        .stdout(predicate::str::contains("unsupported URL"));
}

#[test]
fn no_arguments_is_a_usage_error() {
    x2k().assert().code(2);
}

#[test]
fn unsupported_url_exits_with_3() {
    x2k()
        .arg("https://example.com/a")
        .assert()
        .code(3)
        .stderr(predicate::str::contains("unsupported URL"));
}

#[test]
fn unsupported_url_with_json_prints_an_error_object() {
    let output = x2k()
        .args(["https://example.com/a", "--json"])
        .assert()
        .code(3)
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["status"], "error");
    assert_eq!(json["kind"], "unsupported_url");
}

#[test]
fn missing_config_exits_with_5_before_touching_the_network() {
    let dir = tempfile::tempdir().unwrap();

    x2k()
        .env("X2K_CONFIG", dir.path().join("missing.toml"))
        .arg(X_URL)
        .assert()
        .code(5)
        .stderr(predicate::str::contains("x2k init"));
}

/// Writes a config whose password comes from `command`; returns the temp dir holding it.
#[cfg(unix)]
fn config_with_password_command(command: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let text = format!(
        "kindle_email = \"you_abc@kindle.com\"\n\n[smtp]\nhost = \"127.0.0.1\"\nport = 2525\n\
         username = \"you@example.com\"\npassword_command = \"{command}\"\n"
    );
    std::fs::write(&path, text).unwrap();
    (dir, path)
}

#[cfg(unix)]
#[test]
fn failing_password_command_exits_with_5_before_touching_the_network() {
    let (_dir, path) = config_with_password_command("echo locked >&2; exit 7");

    x2k()
        .env("X2K_CONFIG", &path)
        .env_remove("X2K_SMTP_PASSWORD")
        .arg(X_URL)
        .assert()
        .code(5)
        .stderr(predicate::str::contains("password_command"))
        .stderr(predicate::str::contains("locked"));
}

#[cfg(unix)]
#[test]
fn failing_password_command_with_json_has_kind_config() {
    let (_dir, path) = config_with_password_command("exit 7");

    let output = x2k()
        .env("X2K_CONFIG", &path)
        .env_remove("X2K_SMTP_PASSWORD")
        .args([X_URL, "--json"])
        .assert()
        .code(5)
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["kind"], "config");
    assert!(json["message"].as_str().unwrap().contains("exit 7"));
}

#[test]
#[ignore = "uses the network (FxTwitter and X image servers)"]
fn live_dry_run_saves_an_epub() {
    let dir = tempfile::tempdir().unwrap();

    let output = x2k()
        .args([X_URL, "--dry-run", "--json", "--output"])
        .arg(dir.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["status"], "saved");
    let file = json["file"].as_str().unwrap();
    assert!(std::path::Path::new(file).exists());
}
