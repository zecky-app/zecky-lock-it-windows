# Zecky Lock it for Windows 0.1.0 beta 2

A calm, self-custody Zcash wallet for Windows, with Go Max: your own pruned Zcash
node, optional, on your PC. Free and open source (Apache-2.0).
**Beta: use small amounts only (under 0.1 ZEC).**

Install it over beta 1: your wallet and Go Max's data stay.

## Downloads

| File | What |
|---|---|
| `ZeckyLockIt-0.1.0-beta.2-setup.exe` | Installer. Per-user, no admin rights needed. |
| `ZeckyLockIt-0.1.0-beta.2-portable.zip` | The same app as a folder: unzip and run `Zecky Lock it.exe`. |
| `SHA256SUMS.txt`, `SHA256SUMS.txt.asc` | Checksums, and my signature on them. |
| `zecky-release-key.asc` | My public signing key. Fingerprint: `5FAF 9FC6 6F04 5B5F 7148 4960 77EB 552B C4BF DDD8`. |

**Check before you run it:** see `VERIFY.txt`.

Needs Windows 10 (1903+) or 11, x64. Go Max needs about 30 GB free.

## What's new

- **Before you start Go Max.** The first switch-on checks this PC first: free space
  (about 30 GB needed, the only thing that stops a start), memory, SSD or hard drive,
  and power, plus what to expect. Read from Windows on this PC; nothing is sent.
  Again any time: "Check this computer" on the Go Max page.
- **Keeps the PC awake** while Go Max catches up (the screen may still turn off), and
  lets go once it is up to date.
- **Only sync while plugged in**, on by default on laptops: on battery Go Max pauses
  and says so, and carries on at the charger.
- **Check for updates** in Settings › About: asks GitHub for the newest version, only
  when you press it. GitHub sees your PC's internet address. Lock it never checks by
  itself and never downloads an update.
- **Phone access over Tor is off** until phone pairing exists: nothing on your PC
  accepts connections from outside.
- **Clearer Go Max words:** "Up to date" only at the newest block, why Lock it is
  still on its server, "A little behind", "Not connected to other Zcash nodes" (Wi-Fi
  gone, or a network that blocks Zcash nodes), and where to get a new version in the
  update notice.
- **Safer node notices:** heights the clock does not allow are not trusted, so no
  early "needs an update" notice.
- Terms and Privacy policy in Settings › About.

## Good to know

- **The first check can take days on an older laptop.** A recent PC with an SSD needs
  about 14 hours; an older laptop or a hard drive several days. The slowest stretch is
  the 2022-23 spam years (roughly blocks 1,710,000 to 2,100,000): the percentage
  crawls there, that is normal. Your wallet works meanwhile, from its server.
- **The node is still Zakura 1.8.5,** which follows the chain up to block 3,643,309
  (about 5 December). The next beta brings the next node; Lock it says so two weeks
  ahead.
- Help for common problems: `READ-ME-FIRST.txt`, next to the app.

## Disclaimer

Self-custody: nobody can recover lost words, and payments are final. Zecky Lock it
is provided as is, without warranty of any kind; you use it at your own risk. See
`LICENSE-AND-DISCLAIMER.txt` (shown by the installer) and `LICENSE`.
