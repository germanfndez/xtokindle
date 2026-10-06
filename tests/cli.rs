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
