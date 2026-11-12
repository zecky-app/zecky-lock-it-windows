# Zecky Lock it for Windows

The same calm, self-custody Zcash wallet as Lock it for Mac. Savings, Safety, Dignity, Quiet.

This repository is at its first step: **access and authorization**. There is no wallet, no
node and no network code here yet.

## What is here

| Path | What it is |
|---|---|
| `crates/lockit-auth` | The lock: a sealed vault that opens only with every key the owner chose (1 to 3), and the three kinds of key: Windows Hello (TPM), a FIDO2 security key, a passphrase. |
| `src-tauri` | The app shell (Tauri 2, Rust). Commands to set up, open and remove a *practice lock*. |
| `ui` | The window: plain HTML, CSS and JavaScript, no build step, no remote content. Newsreader and Inter are bundled. |

## Practice build

The app seals a random *practice secret*, not a wallet. It exists so the keys can be tried
on a real Windows PC without any risk: set up a lock, close the app, open it again, and
check that the same eight-character fingerprint comes back.

## Status, honestly

- The vault, the passphrase key and the key policy are covered by tests that run anywhere:
  `cargo test -p lockit-auth`.
- The Windows Hello and security-key code type-checks for Windows, but has **never been run
  on a real Windows PC**. Do not protect funds with it until it has been tested on hardware
  and reviewed by someone independent.
- The lock artwork and app icon are placeholders drawn for this build, not the Lock it Mac
  assets.

## Running it

On Windows 11, with Rust (MSVC toolchain) and the Tauri CLI installed:

```
cargo install tauri-cli --version "^2" --locked
cargo tauri dev
```

On a Mac the same commands show the window; only the passphrase key works there.

## Rules of the house

- The window draws and asks. Secrets never cross into it.
- Every key the owner picked is needed to open the lock. There is no "any one of them".
- Nothing here talks to the network.
- Fonts: Newsreader and Inter, SIL Open Font License (see `ui/fonts`).
