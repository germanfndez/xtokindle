# AGENTS.md

This file is for AI coding agents. It has two parts:

- [Using x2k as a tool](#using-x2k-as-a-tool): for agents that call the CLI to send articles to a Kindle.
- [Working on this repository](#working-on-this-repository): for agents that change the code.

Humans should start with the [README](README.md).

## Using x2k as a tool

`x2k <url>` fetches a long-form article (currently X Articles), builds an EPUB
and emails it to the user's Kindle. It never prompts the user, except in
`x2k init`.

### Calling it

```sh
x2k <url> --json                     # fetch, render and send; prints one JSON object
x2k <url> --dry-run --json           # build the EPUB only (no config, no email)
x2k <url> --dry-run --output <path>  # save the EPUB to a file or directory
```

- Always pass `--json` and parse stdout. Don't parse the human-readable output.
- Use `--dry-run` to check that a URL works without sending anything.
- Never run `x2k init` from an agent. It is interactive and fails without a
  TTY. If the config is missing (exit code 5), ask the user to run it.

### Result contract

Success:

```json
{"status":"sent","title":"...","author":"Name (@handle)","url":"...","words":2885,"file":null,"sent_to":"you@kindle.com"}
```

- `status` is `"sent"`, or `"saved"` for `--dry-run`.
- `file` is the saved path or `null`.
- `sent_to` is the Kindle address or `null`.

Error (still on stdout when `--json` is set):

```json
{"status":"error","kind":"config","message":"..."}
```

`kind` is one of `unsupported_url`, `fetch`, `parse`, `config`, `render`,
`send`, `io`. Don't rely on key order.

### Exit codes and what to do

| Code | Meaning | Agent action |
| ---- | ------- | ------------ |
| 0 | Success | Report title and destination |
| 1 | Render or file I/O error | Report the message; retrying rarely helps |
| 2 | Bad arguments | Fix the invocation |
| 3 | Unsupported URL | Tell the user only X/Twitter post URLs are supported |
| 4 | Fetch/parse failed (post missing, private, not an X Article, or FxTwitter down) | Check the URL, then retry once later |
| 5 | Config missing/invalid, or `password_command` failed | Ask the user to run `x2k init` or unlock their password manager (e.g. `export BW_SESSION="$(bw unlock --raw)"`) |
| 6 | Email could not be sent | Report it; often a wrong/revoked app password or a sender not approved in Amazon |

### Secrets

- The config lives at `~/.config/x2k/config.toml` (or `$X2K_CONFIG`). It may
  contain an SMTP password: never print it, copy it, or commit it.
- The password may come from `[smtp] password_command` (for example
  `bw get password x2k-gmail`). Never run that command yourself to read the
  password.

## Working on this repository

Rust CLI (edition 2024), with a library crate (`src/lib.rs`) and a thin binary
(`src/main.rs`).

### Commands

```sh
cargo test                                  # unit + offline integration tests (must pass)
cargo test -- --ignored                     # live tests against the real FxTwitter API
cargo clippy --all-targets -- -D warnings   # must be clean
cargo fmt --check                           # must be clean (run `cargo fmt` first)
cargo run -- <url> --dry-run --output /tmp  # manual end-to-end check without sending
```

### Layout

| Path | Responsibility |
| ---- | -------------- |
| `src/main.rs` | CLI parsing (`clap`), printing, exit codes |
| `src/app.rs` | Orchestration: `run` (real dependencies), `process` (injectable, used by tests), `default_registry`, `AppError`, `Outcome` |
| `src/article.rs` | Source-agnostic model: `Article`, `Block`, `Span` |
| `src/source.rs` | `Source` trait, `SourceRegistry`, `SourceError` |
| `src/sources/x.rs` | X source: URL parsing, FxTwitter fetch, Draft.js → `Article` |
| `src/render.rs` | `Article` → XHTML → EPUB, image download/embedding |
| `src/cover.rs` | Letterboxes wide covers into a 1600×2560 Kindle cover |
| `src/config.rs` | TOML config: paths, validation, `password_command`, save with `0600` |
| `src/sender.rs` | `Sender` trait, SMTP delivery via `lettre` |
| `src/init.rs` | Interactive `x2k init` (logic kept in pure functions) |
| `tests/cli.rs` | Integration tests of the binary (offline; live ones are `#[ignore]`) |
| `tests/fixtures/` | Recorded API responses used by unit tests |
| `docs/` | User guides (e.g. `bitwarden.md`) |

Data flow: `URL → SourceRegistry → Source::fetch → Article → render_epub → Sender::send`.

### Conventions

- **TDD**: write the failing test first, then the code. Unit tests go in the same
  file (`#[cfg(test)] mod tests`). Binary-level tests go in `tests/cli.rs`.
- **No network in default tests.** Keep I/O thin and put logic in pure
  functions that tests can call with fixtures. Mark live tests `#[ignore]`.
- **Errors**: one `thiserror` enum per module, mapped to `AppError` kinds and
  exit codes in `src/app.rs`. No `unwrap()` outside tests; `expect("...")` only for invariants that truly cannot fail, with the reason as the message.
- **Dependencies**: pure Rust only (rustls, no OpenSSL; `epub-builder` uses
  `zip-library`). Use `default-features = false` and enable only what's needed.
- **Text offsets**: FxTwitter/Draft.js offsets are UTF-16 code units. Convert
  them, never slice strings by those offsets directly.
- **Security**: escape all text and attributes in XHTML; keep only
  `http`/`https`/`mailto` links. Never log passwords or `password_command` output.
- **Docs**: update `README.md` (and `docs/`) in the same change as user-visible behavior.
- **Commits**: [Conventional Commits](https://www.conventionalcommits.org/)
  (`feat:`, `fix:`, `docs:`, `chore:`), each with code, tests and docs together.
  Don't add AI attribution or `Co-Authored-By` lines.
- **Never commit** `odd/`, `.atl/`, real configs, or anything with credentials.

### Adding a new source

1. Create `src/sources/<name>.rs` implementing `Source` (`name`, `can_handle`, `fetch`).
2. Keep fetching thin; test the mapping to `Article` with a recorded fixture in `tests/fixtures/`.
3. Declare it in `src/sources/mod.rs` and register it in `default_registry()` in `src/app.rs`.
4. Update the README (supported sources) and, if relevant, the exit code guidance above.
