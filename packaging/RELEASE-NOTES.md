# Zecky Lock it for Windows 0.1.0 beta 1

A calm, self-custody Zcash wallet for Windows, the sibling of Lock it for Mac.
Personal open-source project. **Beta: use small amounts only (under 0.1 ZEC).**

## Downloads

| File | What |
|---|---|
| `ZeckyLockIt-0.1.0-beta.1-setup.exe` | Installer. Per-user, no admin rights needed. |
| `ZeckyLockIt-0.1.0-beta.1-portable.zip` | The same app as a folder; unzip and run `Zecky Lock it.exe`. |
| `SHA256SUMS.txt`, `SHA256SUMS.txt.asc` | Checksums, and my signature on them. |
| `zecky-release-key.asc` | My public signing key. |

**Check before you run it:** see `VERIFY.txt`. The app is not code-signed, so
Windows SmartScreen warns: "More info", then "Run anyway".

Needs Windows 10 (1903+) or 11, x64. Go Max needs about 35 GB free. Seal it needs a webcam.

## What's in it

- **The lock.** Your 24 words are sealed by the keys you choose: Windows Hello,
  a passphrase, a six-digit Zecky code, security keys (YubiKey, main + backup).
  Optional time lock, address book with "only saved addresses", "Really open the lock?" pause.
- **Your wallets.** Extra wallets behind the same lock (savings apart from spending).
- **Seal it.** Pair a phone; payments are signed on the phone, shown and read as QR codes
  (Keystone protocol, `ur:zcash-pczt`). Nothing that can spend is on the PC.
- **Go Max.** Your own pruned Zcash node (Zakura + Zaino) on the PC; the wallet reads from it once caught up.
- **Private by default.** Shielded only; payments sent over Tor.
- **NU7 ready.** Ironwood pool; move Orchard to Ironwood and lock in transparent Zcash from home.

## Testing

1. Create a wallet and write the 24 words on paper.
2. Receive a small amount and wait until it shows.
3. Send part of it back.
4. Something odd? Settings › About › **Copy a problem report**, and open an issue here
   with it. The report has no words, keys, addresses or amounts.

## Known limits

- Not code-signed (SmartScreen warning).
- Seal it is new on Windows: please try it with tiny amounts first.
- Uninstalling keeps your wallet in `%LOCALAPPDATA%\app.zecky.lockit` on purpose.
