# x2k

Send long-form articles to your Kindle from the command line. It starts with
**X Articles**: `x2k <url>` fetches the article, builds an EPUB (with images)
and emails it to your Send-to-Kindle address.

It is also made for scripts and agents: no prompts outside `x2k init`, a
`--json` mode and meaningful exit codes.

## Install

```sh
# from a local checkout
cargo install --path .

# or straight from git
cargo install --git https://github.com/germanfndez/xtokindle
```

## One-time setup

1. **Gmail app password.** Turn on 2-Step Verification, then create an app
   password at <https://myaccount.google.com/apppasswords>. Any other SMTP
   server works too.
2. **Run `x2k init`.** It asks for your Kindle address (Amazon > Manage Your
   Content and Devices > Preferences > Personal Document Settings), your sender
   email and the app password, then writes the config file.
3. **Approve the sender in Amazon.** Add your sender email to Amazon > Manage
   Your Content and Devices > Preferences > Personal Document Settings >
   Approved Personal Document E-mail List. Without this, Amazon silently drops
   the documents.

## Usage

```sh
x2k https://x.com/user/status/123                 # fetch, build the EPUB, email it
x2k https://x.com/user/status/123 --dry-run       # only build the EPUB (current directory)
x2k https://x.com/user/status/123 --output ~/books  # also keep a copy (file path or directory)
x2k https://x.com/user/status/123 --json          # machine-readable result
```

`--dry-run` never reads the config and never sends anything.

With `--json`, stdout is a single JSON object.

```json
{
  "status": "sent",
  "title": "How to think",
  "author": "Jane Doe",
  "url": "https://x.com/user/status/123",
  "words": 1234,
  "file": null,
  "sent_to": "you_123@kindle.com"
}
```

`status` is `"sent"` or `"saved"` (dry run). `file` and `sent_to` are `null`
when they do not apply. Errors look like this:

```json
{ "status": "error", "kind": "unsupported_url", "message": "unsupported URL: https://example.com/a" }
```

`kind` is one of `unsupported_url`, `fetch`, `parse`, `config`, `render`,
`send`, `io`. Without `--json`, results go to stdout and errors to stderr.

## Exit codes

| Code | Meaning |
| ---- | ------- |
| 0 | Success |
| 1 | Other error (rendering the EPUB, writing a file) |
| 2 | Usage error (bad arguments) |
| 3 | Unsupported URL |
| 4 | Could not fetch or parse the article |
| 5 | Config missing or invalid |
| 6 | Could not send the email |

## Configuration

The file is `~/.config/x2k/config.toml`, created by `x2k init` (readable only
by you on Unix because it holds a password).

```toml
kindle_email = "you_123@kindle.com"

[smtp]
host = "smtp.gmail.com"
port = 465
username = "you@gmail.com"
password = "app password"
from = "you@gmail.com"   # optional, defaults to username
security = "tls"         # optional: "tls", "starttls" or "none"; defaults to tls on 465, else starttls
```

| Variable | Effect |
| -------- | ------ |
| `X2K_CONFIG` | Use this config file path instead of the default |
| `XDG_CONFIG_HOME` | Config lives in `$XDG_CONFIG_HOME/x2k/config.toml` |
| `X2K_SMTP_PASSWORD` | Overrides the password in the file (so it can stay out of it) |

## How it works

1. A source recognizes the URL. The X source calls the third-party
   [FxTwitter](https://github.com/FxEmbed/FxEmbed) API (`api.fxtwitter.com`),
   so x2k depends on that service being up and keeping its response format.
2. The article is converted to a small internal model (title, author, blocks).
3. Images are downloaded and embedded, and an EPUB is built.
4. The EPUB is emailed over SMTP to your Kindle address.

## Adding a new source

1. Implement the `Source` trait (`src/source.rs`): `name`, `can_handle` and
   `fetch`, returning an `Article`. See `src/sources/x.rs`.
2. Register it in `default_registry()` in `src/app.rs`. Sources registered
   first win when several match a URL.

## Development

```sh
cargo test                 # unit and offline integration tests
cargo test -- --ignored    # also run the live tests (they use the network)
cargo clippy --all-targets -- -D warnings
```
