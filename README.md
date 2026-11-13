# Zecky Lock it for Windows

The same calm, self-custody Zcash wallet as Lock it for Mac. Savings, Safety, Dignity, Quiet.

Two pieces are here so far: **the lock** (who may release funds) and **Go Max** (starting,
watching and stopping your own pruned node). There is no wallet yet.

## What is here

| Path | What it is |
|---|---|
| `crates/lockit-auth` | The lock: a sealed vault that opens only with every key the owner chose (1 to 3), and the three kinds of key: Windows Hello (TPM), a FIDO2 security key, a passphrase. |
| `crates/lockit-node` | Go Max: writes the Zakura and Zaino configs, starts both, reads the node's status, and stops both cleanly (on Windows through a small Ctrl-C helper, see `process.rs`). |
| `crates/lockit-tor` | Go Max's own onion address (Arti, Tor in Rust): made on the first start, kept after that, forwarding only the wallet port. For reaching the node from a paired phone. |
| `src-tauri` | The app shell (Tauri 2, Rust). Commands for the *practice lock* and for Go Max. |
| `.github/workflows/windows.yml` | Builds the node for Windows, tests the clean stop on a real Windows runner, builds the unsigned installer. Runs once the repo is pushed. |
| `ui` | The window: plain HTML, CSS and JavaScript, no build step, no remote content. Newsreader and Inter are bundled. |

## Practice build

The app seals a random *practice secret*, not a wallet. It exists so the keys can be tried
on a real Windows PC without any risk: set up a lock, close the app, open it again, and
check that the same eight-character fingerprint comes back.

## Go Max

The node programs are not in this repository. CI builds them from pinned upstream
sources (see the workflow) and the installer carries them in `node\`. For a local build,
copy `zakurad.exe`, `zainod.exe`, `SHA256SUMS.txt` and `SOURCES.txt` into
`src-tauri/node/` and build with:

```
cargo tauri build --config src-tauri/tauri.gomax.conf.json
```

While Go Max runs, the app also runs a Tor onion service with this install's own
address, so a paired phone can reach the wallet service from anywhere, with no extra app
or account. Only the wallet port is passed on; every other port is refused.

Go Max is private by default: the node listens on this computer only, so other nodes
cannot connect in, Windows never shows a firewall prompt, and the RPC and wallet ports
stay on `127.0.0.1`. The chain state lives in `%LOCALAPPDATA%`, never roaming.

To try the start and stop logic by itself:

```
cargo run --example go_max_smoke -p lockit-node -- zakurad.exe zainod.exe C:\go-max-test 20 3
```

## Status, honestly

- The vault, the passphrase key and the key policy are covered by tests that run anywhere:
  `cargo test -p lockit-auth`. Go Max's config writing: `cargo test -p lockit-node`.
- Go Max's start, status and clean stop have run against the real Zakura and Zaino:
  natively on Linux, and as Windows programs under Wine. Not yet on real Windows; the
  CI job `go-max-windows` does that once the repo is pushed.
- The onion address is made offline and kept across restarts (tested on Linux and under
  Wine). Publishing it on the Tor network has not been tried: no network where it was
  built. The pairing QR is not built yet; it must match Lock it Mac's format.
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
