# Zecky Lock it for Windows 0.1.0 beta 1

A calm, self-custody Zcash wallet for Windows, the sibling of Lock it for Mac,
with Go Max: your own pruned Zcash node, optional, on your PC.
Free and open source (Apache-2.0). **Beta: use small amounts only (under 0.1 ZEC).**

## Downloads

| File | What |
|---|---|
| `ZeckyLockIt-0.1.0-beta.1-setup.exe` | Installer. Per-user, no admin rights needed. |
| `ZeckyLockIt-0.1.0-beta.1-portable.zip` | The same app as a folder: unzip and run `Zecky Lock it.exe`. |
| `SHA256SUMS.txt`, `SHA256SUMS.txt.asc` | Checksums, and my signature on them. |
| `zecky-release-key.asc` | My public signing key. Fingerprint: see below. |

**Check before you run it:** see `VERIFY.txt`. Lock it is not code-signed, so
Windows SmartScreen warns: "More info", then "Run anyway".

Needs Windows 10 (1903+) or 11, x64. Go Max needs about 35 GB free. Seal it needs a webcam.

## What's in it

- **The lock.** Your 24 words are sealed by the keys you choose: Windows Hello, a
  passphrase, a six-digit Zecky code, security keys (YubiKey, main + backup).
  Optional time lock, address book with "only saved addresses", and a "Really open
  the lock?" pause. The keys and the time lock apply to every wallet on the PC.
- **Your wallets.** Extra wallets behind the same lock, as chips above it: rename,
  reorder, keep out of sight, show their words.
- **Go Max.** Your own pruned Zcash node (Zakura + Zaino). Lock it reads from it once
  it has caught up; older blocks come from your server, and the page says so.
- **Private by default.** Shielded only; payments sent over Tor (built in).
- **NU7 ready.** Ironwood; move Orchard to Ironwood and lock in transparent Zcash.
- **Seal it (experimental on Windows).** Pair a phone; payments are signed on the
  phone and passed by QR codes. Try it with a tiny amount first.

## Testing

1. Create a wallet and write the 24 words on paper.
2. Receive a small amount and wait until it shows.
3. Send part of it back.
4. Something odd? Settings › About › **Copy a problem report**, and open an issue
   here with it. The report has no words, keys, addresses or amounts.

## Good to know

- **Go Max needs an update before about 5 December.** Each node version follows the
  chain up to a fixed block (this one: 3,643,309). Lock it says so two weeks ahead,
  and a new beta brings the next node. Your wallet keeps working without Go Max.
- Uninstalling keeps your wallet in `%LOCALAPPDATA%\app.zecky.lockit` on purpose.
- Help for common problems (SmartScreen, antivirus, Tor blocked, slow restore,
  Windows Hello): `READ-ME-FIRST.txt`, next to the app.

## Disclaimer

Self-custody: nobody can recover lost words, and payments are final. Zecky Lock it
is provided as is, without warranty of any kind; you use it at your own risk. See
`LICENSE-AND-DISCLAIMER.txt` (shown by the installer) and `LICENSE`.
