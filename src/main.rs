use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use x2k::app::{self, AppError, Options};

/// Exit codes shown at the end of `--help`.
const EXIT_CODES: &str = "\
Exit codes:
  0  success
  1  other error (rendering the EPUB, writing a file)
  2  usage error (bad arguments)
  3  unsupported URL
  4  could not fetch or parse the article
  5  config missing or invalid (run `x2k init`)
  6  could not send the email";

/// Send long-form articles (starting with X Articles) to your Kindle.
#[derive(Parser, Debug)]
#[command(name = "x2k", version, about, after_help = EXIT_CODES)]
#[command(args_conflicts_with_subcommands = true, arg_required_else_help = true)]
struct Cli {
    /// URL of the article to send (e.g. https://x.com/user/status/123)
    url: Option<String>,

    /// Build the EPUB locally without sending it
    #[arg(long)]
    dry_run: bool,

    /// Print the result as JSON (useful for agents and scripts)
    #[arg(long)]
    json: bool,

    /// Save the EPUB to this file or directory (default with --dry-run: current directory)
    #[arg(long, value_name = "PATH")]
    output: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Set up your Kindle address and SMTP account interactively
    Init,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match (cli.command, cli.url) {
        (Some(Command::Init), _) => {
            eprintln!("x2k init is not available yet");
            ExitCode::from(1)
        }
        (None, Some(url)) => {
            let options = Options {
                dry_run: cli.dry_run,
                output: cli.output,
            };
            report(app::run(&url, &options), cli.json)
        }
        // `arg_required_else_help` already handled running with no arguments.
        (None, None) => ExitCode::from(2),
    }
}

/// Prints the result (JSON on stdout with `--json`, text otherwise) and picks the exit code.
fn report(result: Result<app::Outcome, AppError>, json: bool) -> ExitCode {
    match result {
        Ok(outcome) => {
            if json {
                println!("{}", outcome.to_json());
            } else {
                println!("{}", outcome.human_message());
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            if json {
                println!("{}", error.to_json());
            } else {
                eprintln!("error: {error}");
            }
            ExitCode::from(error.exit_code() as u8)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_url_with_defaults() {
        let cli = Cli::try_parse_from(["x2k", "https://x.com/user/status/1"]).unwrap();

        assert_eq!(cli.url.as_deref(), Some("https://x.com/user/status/1"));
        assert_eq!(cli.output, None);
        assert!(cli.command.is_none());
        assert!(!cli.dry_run);
        assert!(!cli.json);
    }

    #[test]
    fn parses_dry_run_and_json_flags() {
        let cli =
            Cli::try_parse_from(["x2k", "https://x.com/user/status/1", "--dry-run", "--json"])
                .unwrap();

        assert!(cli.dry_run);
        assert!(cli.json);
    }

    #[test]
    fn fails_without_url() {
        assert!(Cli::try_parse_from(["x2k"]).is_err());
    }

    #[test]
    fn parses_output_path() {
        let cli =
            Cli::try_parse_from(["x2k", "https://x.com/user/status/1", "--output", "out"]).unwrap();

        assert_eq!(cli.output, Some(PathBuf::from("out")));
    }

    #[test]
    fn parses_init_subcommand() {
        let cli = Cli::try_parse_from(["x2k", "init"]).unwrap();

        assert!(matches!(cli.command, Some(Command::Init)));
        assert_eq!(cli.url, None);
    }

    #[test]
    fn init_does_not_mix_with_a_url() {
        assert!(Cli::try_parse_from(["x2k", "https://x.com/user/status/1", "init"]).is_err());
    }
}
