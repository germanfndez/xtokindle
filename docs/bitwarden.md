# Using x2k with Bitwarden

This guide keeps your SMTP app password in Bitwarden instead of the x2k config
file. x2k asks Bitwarden for it every time it sends an article.

## What you need

- The Bitwarden CLI (`bw`): `brew install bitwarden-cli`
- An app password for your sender account (Gmail: <https://myaccount.google.com/apppasswords>)
- x2k already set up with `x2k init` (see the [README](../README.md#one-time-setup))

Tip: use a dedicated Gmail account just for sending to Kindle. An app password
gives access to that account's mail, so a separate account limits what a leak
could expose.

## 1. Log in to Bitwarden (first time only)

```sh
bw login
```

## 2. Save the app password in Bitwarden

Create a login item named **`x2k-gmail`** (in the app, the browser extension or
the web vault) with:

- **Username**: your sender email (e.g. `you@gmail.com`)
- **Password**: the 16-character app password

The name must be unique in your vault, because `bw get password x2k-gmail`
looks it up by name. If you created the item in another app, pull it into the
CLI with `bw sync`.

## 3. Point x2k at Bitwarden

Edit `~/.config/x2k/config.toml`: remove the `password = "..."` line and add
`password_command` to the `[smtp]` section.

```toml
[smtp]
host = "smtp.gmail.com"
port = 465
username = "you@gmail.com"
password_command = "bw get password x2k-gmail"
```

You can't have both `password` and `password_command` in the file.

## 4. Unlock and send

`bw` only returns secrets while the vault is unlocked, and it reads the session
from the `BW_SESSION` environment variable:

```sh
export BW_SESSION="$(bw unlock --raw)"   # asks for your master password
x2k https://x.com/user/status/123
```

The session lasts until you close the terminal or run `bw lock`.

If the vault is locked, x2k stops before downloading anything and exits with
code 5:

```
Error: password_command `bw get password x2k-gmail` failed: it printed nothing. If you use a password manager CLI, make sure it is unlocked (e.g. `bw unlock` and export BW_SESSION)
```

## Optional: unlock automatically

Add this function to `~/.zshrc` (or `~/.bashrc`). It unlocks Bitwarden only
when needed, then runs x2k:

```sh
x2k() {
  if ! bw status 2>/dev/null | grep -q '"status":"unlocked"'; then
    BW_SESSION="$(bw unlock --raw)" || return 1
    export BW_SESSION
  fi
  command x2k "$@"
}
```

Open a new terminal (or run `source ~/.zshrc`). The first `x2k` call in each
terminal asks for your master password, and the following calls don't.

## Troubleshooting

| Problem | Fix |
| ------- | --- |
| `password_command ... failed: it printed nothing` | The vault is locked. Run `export BW_SESSION="$(bw unlock --raw)"` |
| `Not found.` in the error | No item named `x2k-gmail`: check the name, or run `bw sync` |
| `More than one result was found` | Two items share the name. Rename one, or use the item id: `bw get password <id>` |
| `You are not logged in.` | Run `bw login` |
| Authentication error from Gmail (exit code 6) | The stored app password is wrong or was revoked. Create a new one and update the Bitwarden item |

## Other password managers

`password_command` runs any shell command, so other managers work the same way.
See [Password managers](../README.md#password-managers) in the README.
