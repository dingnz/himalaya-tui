<div align="center">
  <img src="./logo.svg" alt="Logo" width="128" height="128" />
  <h1>📫 Himalaya TUI</h1>
  <p>TUI to manage emails</p>
  <p>
    <a href="https://matrix.to/#/#pimalaya:matrix.org"><img alt="Matrix" src="https://img.shields.io/badge/chat-%23pimalaya-blue?style=flat&logo=matrix&logoColor=white"/></a>
    <a href="https://fosstodon.org/@pimalaya"><img alt="Mastodon" src="https://img.shields.io/badge/news-%40pimalaya-blue?style=flat&logo=mastodon&logoColor=white"/></a>
  </p>
</div>

> [!IMPORTANT]
> Himalaya TUI is in active development and currently shipped as `v0.0.1`. Expect breaking changes between releases; the CLI counterpart [pimalaya/himalaya](https://github.com/pimalaya/himalaya) remains the stable interface for production use.

## Table of contents

- [Features](#features)
- [Installation](#installation)
  - [Cargo](#cargo)
  - [Nix](#nix)
  - [Sources](#sources)
- [Configuration](#configuration)
- [Usage](#usage)
  - [Keybindings](#keybindings)
  - [Composing messages](#composing-messages)
- [Interfaces](#interfaces)
- [Social](#social)
- [Sponsoring](#sponsoring)

## Features

- **Three-pane layout** built on [ratatui](https://ratatui.rs): mailboxes, envelopes, message body or composer
- **In-app composer** powered by [edtui](https://crates.io/crates/edtui) with system-editor handoff (`Alt-e`)
- **Provider discovery wizard** shared with [himalaya](https://github.com/pimalaya/himalaya): PACC, Thunderbird Autoconfiguration, RFC 6186 SRV
- **Shared configuration file** with `himalaya`: same `[accounts.<name>]` blocks load on both binaries (see [Configuration](#configuration))
- **IMAP** support <sup>[rfc9051](https://www.iana.org/go/rfc9051)</sup> (requires `imap` feature)
- **JMAP** support <sup>[rfc8620](https://www.iana.org/go/rfc8620), [rfc8621](https://www.iana.org/go/rfc8621)</sup> (requires `jmap` feature)
- **Maildir** support (requires `maildir` feature)
- **SMTP** backend <sup>[rfc5321](https://www.iana.org/go/rfc5321)</sup> (requires `smtp` feature)
- **TLS** support:
  - [native-tls](https://crates.io/crates/native-tls) (requires `native-tls` feature)
  - [rustls](https://crates.io/crates/rustls):
    - AWS-LC crypto provider (requires `rustls-aws` feature)
    - Ring crypto provider (requires `rustls-ring` feature)
- **SASL** support: anonymous, login, plain, oauthbearer, xoauth2, scram-sha-256
- **Discovery** support: Autoconfiguration (Thunderbird), PACC and RFC 6186 (SRV lookups)

*Himalaya TUI is written in [Rust](https://www.rust-lang.org/) and uses [cargo features](https://doc.rust-lang.org/cargo/reference/features.html) to gate backend support. The default feature set is declared in [`Cargo.toml`](./Cargo.toml).*

## Installation

### Pre-built binary

Himalaya TUI is not yet released, therefore the only way to get a pre-built binary is to check out the [releases](https://github.com/pimalaya/himalaya-tui/actions/workflows/releases.yml) GitHub workflow and look for the *Artifacts* section.

> [!IMPORTANT]
> Such binaries are built with the default cargo features. If you need specific features, please use another installation method.

### Cargo

```
cargo install --locked --git https://github.com/pimalaya/himalaya-tui.git
```

With only IMAP+SMTP support:

```
cargo install --locked --git https://github.com/pimalaya/himalaya-tui.git \
  --no-default-features \
  --features imap,smtp,rustls-ring
```

### Nix

If you have the [Flakes](https://nixos.wiki/wiki/Flakes) feature enabled:

```
nix profile install github:pimalaya/himalaya-tui
```

Or run without installing:

```
nix run github:pimalaya/himalaya-tui
```

### Sources

```
git clone https://github.com/pimalaya/himalaya-tui
cd himalaya-tui
nix run
```

## Configuration

Run `himalaya-tui`. With no configuration file on disk the wizard prompts for an email address, a server URL or a bare domain, runs provider discovery, asks for SASL or HTTP credentials, then keeps the resulting account in memory for that session only (the TUI does not write to disk).

A persistent configuration is loaded from the first valid path among:

- `$XDG_CONFIG_HOME/himalaya/config.toml`
- `$HOME/.config/himalaya/config.toml`
- `$HOME/.himalayarc`

These are the same paths the [`himalaya`](https://github.com/pimalaya/himalaya) CLI looks at: one TOML file backs both binaries, **starting from himalaya CLI v2**. TUI-only fields (`from`, `from-name`, `signature`, `signature-delim`) and CLI-only sections (`table`, `envelope`, `mailbox`, `message`, `attachment`) coexist without errors. See [`config.sample.toml`](./config.sample.toml) for a documented template.

> [!NOTE]
> A himalaya CLI v1 configuration file is **not** compatible with himalaya TUI: the v1 schema differs from the v2 one shared with the TUI. Upgrade the CLI to v2 (or rewrite the file using [`config.sample.toml`](./config.sample.toml)) before pointing the TUI at it.

Override the path with `-c <PATH>` or `HIMALAYA_CONFIG=<PATH>`; multiple paths can be passed at once, separated by `:`. The first one is the base and the rest are deep-merged on top.

Pass `--no-config` to ignore both, even when a file is present: useful for testing another account in memory without exposing stored credentials.

CLI flags (see `himalaya-tui --help`):

- `[ACCOUNT]`: account name when a config is loaded; otherwise a wizard seed (email, URL or domain)
- `-c, --config <PATH>`: override the default config file path (env: `HIMALAYA_CONFIG`)
- `--no-config`: skip on-disk config and run the wizard
- `--from <EMAIL>`: override the From address used when sending; also prefills the wizard's SASL/JMAP login
- `--from-name <NAME>`: override the From display name
- `--keybinds <vim|emacs>`: composer keybinding flavor (overrides the top-level `keybinds` TOML field; defaults to Vim)

## Usage

### Keybindings

Top-level navigation:

| Action | Universal | Vim flavor | Emacs flavor |
|---|---|---|---|
| Cycle panel | `Tab` | `Tab` | `Tab` |
| Next item | `↓` | `j` | `Ctrl-n` |
| Previous item | `↑` | `k` | `Ctrl-p` |
| Next page | `PageDown` | `Ctrl-d` | `Ctrl-v` |
| Previous page | `PageUp` | `Ctrl-u` | `Alt-v` |
| Select | `Enter` | `Enter` | `Enter` |
| Close panel / dialog / quit | `Esc` | `q` | `Ctrl-g` |
| Start a new draft | `Ctrl-c` | `Ctrl-c` | `Ctrl-c` |

By default, only the universal keys fire. Opt into a flavor with `--keybinds <vim|emacs>` (or the top-level `keybinds = "emacs"` TOML field) to enable the matching column as additive aliases.

Composer:

| Key | Action |
|---|---|
| `Ctrl-e` (Vim) / `Alt-e` | Hand off to `$VISUAL` or `$EDITOR` for the current draft |
| `Esc` | Open the compose actions dialog (Send, Preview, Save to Drafts, Cancel) |

Inside the composer, the chosen flavor drives [edtui](https://crates.io/crates/edtui)'s built-in keybindings (Vim normal/insert vs. Emacs insert-style). In Vim mode, `Ctrl-e` (edtui's normal-mode binding) opens the external editor; in Emacs mode, `Ctrl-e` is rebound to "move to end of line" and `Alt-e` is the only system-editor key.

Envelope dialog actions: Read, Reply, Reply All, Forward, Copy, Move, Add flag, Remove flag.

### Composing messages

Drafts are written in [MML](https://github.com/pimalaya/mml) and compiled to MIME on send. Headers (`From`, `To`, `Subject`...) live at the top of the buffer; the body and any MML directives (attachments, signing, encryption) follow.

Sending routes through SMTP when an `[accounts.<name>.smtp]` block is configured, otherwise through JMAP. Drafts can be saved to the `Drafts` mailbox at any time.

## Interfaces

Himalaya TUI is one of several front-ends to the Pimalaya libraries. See [pimalaya/himalaya#interfaces](https://github.com/pimalaya/himalaya#interfaces) for the full list (CLI, Vim, Emacs, Raycast).

## Social

- Chat on [Matrix](https://matrix.to/#/#pimalaya:matrix.org)
- News on [Mastodon](https://fosstodon.org/@pimalaya) or [RSS](https://fosstodon.org/@pimalaya.rss)
- Mail at [pimalaya.org@posteo.net](mailto:pimalaya.org@posteo.net)

## Sponsoring

[![nlnet](https://nlnet.nl/logo/banner-160x60.png)](https://nlnet.nl/)

Special thanks to the [NLnet foundation](https://nlnet.nl/) and the [European Commission](https://www.ngi.eu/) that have been financially supporting the project for years:

- 2022 → 2023: [NGI Assure](https://nlnet.nl/project/Himalaya/)
- 2023 → 2024: [NGI Zero Entrust](https://nlnet.nl/project/Pimalaya/)
- 2024 → 2026: [NGI Zero Core](https://nlnet.nl/project/Pimalaya-PIM/)
- *2027 in preparation...*

If you appreciate the project, feel free to donate using one of the following providers:

[![GitHub](https://img.shields.io/badge/-GitHub%20Sponsors-fafbfc?logo=GitHub%20Sponsors)](https://github.com/sponsors/soywod)
[![Ko-fi](https://img.shields.io/badge/-Ko--fi-ff5e5a?logo=Ko-fi&logoColor=ffffff)](https://ko-fi.com/soywod)
[![Buy Me a Coffee](https://img.shields.io/badge/-Buy%20Me%20a%20Coffee-ffdd00?logo=Buy%20Me%20A%20Coffee&logoColor=000000)](https://www.buymeacoffee.com/soywod)
[![Liberapay](https://img.shields.io/badge/-Liberapay-f6c915?logo=Liberapay&logoColor=222222)](https://liberapay.com/soywod)
[![thanks.dev](https://img.shields.io/badge/-thanks.dev-000000?logo=data:image/svg+xml;base64,PHN2ZyB3aWR0aD0iMjQuMDk3IiBoZWlnaHQ9IjE3LjU5NyIgY2xhc3M9InctMzYgbWwtMiBsZzpteC0wIHByaW50Om14LTAgcHJpbnQ6aW52ZXJ0IiB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciPjxwYXRoIGQ9Ik05Ljc4MyAxNy41OTdINy4zOThjLTEuMTY4IDAtMi4wOTItLjI5Ny0yLjc3My0uODktLjY4LS41OTMtMS4wMi0xLjQ2Mi0xLjAyLTIuNjA2di0xLjM0NmMwLTEuMDE4LS4yMjctMS43NS0uNjc4LTIuMTk1LS40NTItLjQ0Ni0xLjIzMi0uNjY5LTIuMzQtLjY2OUgwVjcuNzA1aC41ODdjMS4xMDggMCAxLjg4OC0uMjIyIDIuMzQtLjY2OC40NTEtLjQ0Ni42NzctMS4xNzcuNjc3LTIuMTk1VjMuNDk2YzAtMS4xNDQuMzQtMi4wMTMgMS4wMjEtMi42MDZDNS4zMDUuMjk3IDYuMjMgMCA3LjM5OCAwaDIuMzg1djEuOTg3aC0uOTg1Yy0uMzYxIDAtLjY4OC4wMjctLjk4LjA4MmExLjcxOSAxLjcxOSAwIDAgMC0uNzM2LjMwN2MtLjIwNS4xNTYtLjM1OC4zODQtLjQ2LjY4Mi0uMTAzLjI5OC0uMTU0LjY4Mi0uMTU0IDEuMTUxVjUuMjNjMCAuODY3LS4yNDkgMS41ODYtLjc0NSAyLjE1NS0uNDk3LjU2OS0xLjE1OCAxLjAwNC0xLjk4MyAxLjMwNXYuMjE3Yy44MjUuMyAxLjQ4Ni43MzYgMS45ODMgMS4zMDUuNDk2LjU3Ljc0NSAxLjI4Ny43NDUgMi4xNTR2MS4wMjFjMCAuNDcuMDUxLjg1NC4xNTMgMS4xNTIuMTAzLjI5OC4yNTYuNTI1LjQ2MS42ODIuMTkzLjE1Ny40MzcuMjYuNzMyLjMxMi4yOTUuMDUuNjIzLjA3Ni45ODQuMDc2aC45ODVabTE0LjMxNC03LjcwNmgtLjU4OGMtMS4xMDggMC0xLjg4OC4yMjMtMi4zNC42NjktLjQ1LjQ0NS0uNjc3IDEuMTc3LS42NzcgMi4xOTVWMTQuMWMwIDEuMTQ0LS4zNCAyLjAxMy0xLjAyIDIuNjA2LS42OC41OTMtMS42MDUuODktMi43NzQuODloLTIuMzg0di0xLjk4OGguOTg0Yy4zNjIgMCAuNjg4LS4wMjcuOTgtLjA4LjI5Mi0uMDU1LjUzOC0uMTU3LjczNy0uMzA4LjIwNC0uMTU3LjM1OC0uMzg0LjQ2LS42ODIuMTAzLS4yOTguMTU0LS42ODIuMTU0LTEuMTUydi0xLjAyYzAtLjg2OC4yNDgtMS41ODYuNzQ1LTIuMTU1LjQ5Ny0uNTcgMS4xNTgtMS4wMDQgMS45ODMtMS4zMDV2LS4yMTdjLS44MjUtLjMwMS0xLjQ4Ni0uNzM2LTEuOTgzLTEuMzA1LS40OTctLjU3LS43NDUtMS4yODgtLjc0NS0yLjE1NXYtMS4wMmMwLS40Ny0uMDUxLS44NTQtLjE1NC0xLjE1Mi0uMTAyLS4yOTgtLjI1Ni0uNTI2LS40Ni0uNjgyYTEuNzE5IDEuNzE5IDAgMCAwLS43MzctLjMwNyA1LjM5NSA1LjM5NSAwIDAgMC0uOTgtLjA4MmgtLjk4NFYwaDIuMzg0YzEuMTY5IDAgMi4wOTMuMjk3IDIuNzc0Ljg5LjY4LjU5MyAxLjAyIDEuNDYyIDEuMDIgMi42MDZ2MS4zNDZjMCAxLjAxOC4yMjYgMS43NS42NzggMi4xOTUuNDUxLjQ0NiAxLjIzMS42NjggMi4zNC42NjhoLjU4N3oiIGZpbGw9IiNmZmYiLz48L3N2Zz4=)](https://thanks.dev/soywod)
[![PayPal](https://img.shields.io/badge/-PayPal-0079c1?logo=PayPal&logoColor=ffffff)](https://www.paypal.com/paypalme/soywod)
