use clap::Parser;

// Wired into the CLI in T7.
#[allow(dead_code)]
mod article;
// Wired into the CLI in T7.
#[allow(dead_code)]
mod config;
// Wired into the CLI in T7.
#[allow(dead_code)]
mod render;
// Wired into the CLI in T7.
#[allow(dead_code)]
mod sender;
// Wired into the CLI in T7.
#[allow(dead_code)]
mod source;
// Wired into the CLI in T7.
#[allow(dead_code)]
mod sources;

/// Send long-form articles (starting with X Articles) to your Kindle.
#[derive(Parser, Debug)]
#[command(name = "x2k", version, about)]
struct Cli {
    /// URL of the article to send (e.g. https://x.com/user/status/123)
    url: String,

    /// Build the EPUB locally without sending it
    #[arg(long)]
    dry_run: bool,

    /// Print the result as JSON (useful for agents and scripts)
    #[arg(long)]
    json: bool,
}

fn main() {
    let cli = Cli::parse();
    println!("{cli:?}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_url_with_defaults() {
        let cli = Cli::try_parse_from(["x2k", "https://x.com/user/status/1"]).unwrap();

        assert_eq!(cli.url, "https://x.com/user/status/1");
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
}
