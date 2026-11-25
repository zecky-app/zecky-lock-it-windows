# Zecky Lock it for Windows 0.1.0 beta 7

A calm, self-custody Zcash wallet for Windows, with Go Max: your own pruned Zcash
node, optional, on your PC. Free and open source (Apache-2.0).
**Beta: use small amounts only (under 0.1 ZEC).**

Install it over beta 6: your wallet and Go Max's data stay, and the node carries on
where it is (no new sync).

## Downloads

| File | What |
|---|---|
| `ZeckyLockIt-0.1.0-beta.7-setup.exe` | Installer. Per-user, no admin rights needed. |
| `ZeckyLockIt-0.1.0-beta.7-portable.zip` | The same app as a folder: unzip and run `Zecky Lock it.exe`. |
| `SHA256SUMS.txt`, `SHA256SUMS.txt.asc` | Checksums, and my signature on them. |
| `zecky-release-key.asc` | My public signing key. Fingerprint: `5FAF 9FC6 6F04 5B5F 7148 4960 77EB 552B C4BF DDD8`. |

**Check before you run it:** see `VERIFY.txt`.

Needs Windows 10 (1903+) or 11, x64. Go Max needs about 30 GB free.

## What's new in beta 7

The same as Lock it for Mac:

- **Signing devices: Keystone and Cupcake** (experimental). Pair one from the +
  next to your wallets; its keys stay on the device, and every payment is signed
  there. Pairing asks whether the wallet is new or already holds Zcash, so its
  older payments show up. The webcam now reads moving QR codes reliably.
- **"Up to date" means the newest blocks are checked.** Older blocks are checked
  after that; meanwhile the balance says it may still grow. Each wallet sends once
  its own blocks are checked.
- **A message when Zcash arrives,** once per payment.
- **Watch a wallet** from its viewing key, such as your shop's: balance, payments
  and memos, never able to spend.
- **Export a wallet's payments as CSV** (Activity), for your bookkeeping.
- **Settings › Business:** watch a wallet, share a viewing key (hidden from
  screenshots while it shows), export CSV.
- **Settings › Security › Camera:** whether Windows allows the camera, with a
  button to Windows' camera settings.
- **The Go Max button** shows the switch: the coin sits right while Go Max is on.

Same node as beta 6: Zakura 1.8.9 and the NU7 build of Zaino.

## What's new in beta 6

- **Lock it moves to your own node reliably.** Go Max now counts as up to date
  against real heights (the blocks its peers announce, and zec.rocks), not the
  node's own time-based guess. Since NU7's 25-second blocks that guess ran a few
  blocks ahead, so the move to your own node could take hours.
- **"Your own node (Go Max)" in Settings › Network.** Pick it once Go Max is up to
  date; no address to type. While Go Max is off or catching up, Lock it reads from
  zec.rocks and says so.
- **Sending waits until the wallet is up to date.** Right after starting, a payment
  could be built on an old view of the chain and refused as expired. Now Lock it
  says it is still catching up, with the block it is at. A payment that expired before it got
  through is named and dropped: it was never paid.
- Small: no "node not answering" flashing during a slow moment, the Go Max page no
  longer shows a network height below the node's, dates in English, "since Canopy"
  wording, and the licence field matches the LICENSE file (Apache 2.0).

Also in beta 6, the same as Lock it for Mac:

- **Receive › Transparent.** Your wallet's transparent (t1) address, for senders
  that cannot pay a shielded one, shown in a red PUBLIC frame. When Zcash arrives
  there, Lock it asks to lock it in, so it is private again.
- **Sign with Cupcake.** A Cupcake wallet can be your signing phone, like Seal it.
  "Seal it phone" is now called "signing phone".
- **TEX payments** show in Activity as "TEX payment" and "TEX payment · forwarding fee".
  If only part of a payment gets through, Lock it says which part.
- **Wallet name on Receive and Send.** With several wallets, the one you are using
  is named under the title.
- **The time lock counts blocks as well as time,** so moving the PC's clock does
  not open it sooner.
- **Prices over Tor** when Tor is on for sending.
- Fix: after switching wallets, Receive › Transparent could show the previous
  wallet's address.
- Safety: payment QR codes with impossible lengths are refused.

Same node as beta 5: Zakura 1.8.9 and the NU7 build of Zaino.

## What's new in beta 5

- **Fix: Lock it now moves to your own node once Go Max is up to date.** In beta 4
  the node synced fine, but its wallet service (Zaino) did not know the NU7 network
  upgrade (block 3,543,405) and stopped there, so Lock it stayed on zec.rocks. Beta 5
  ships the same Zaino as Lock it for Mac (the NU7 build). Nothing else changed: same
  node (Zakura 1.8.9), same data, no new sync.

## New in beta 4

- **Fix: Go Max no longer stops at block 2,713,001.** In beta 3 the node stopped for
  good there, at the first old-style (Sprout) transaction it had to check: since
  Zakura 1.8.8, checking those is a build option, and beta 3's node was built
  without it. Beta 4's node has it. Your node's data is fine (Zakura retries such a
  block instead of marking it bad): install beta 4 and Go Max carries on from where
  it stopped.
- **Node updated to Zakura 1.8.9** (was 1.8.8): less CPU at the tip and during sync,
  quicker reconnects after a restart, stricter limits on oversized blocks and on the
  node's local counters. It still follows the chain up to block 3,643,309 (about
  5 December).
- **One way only:** the first start of beta 4 upgrades Go Max's data to Zakura's
  new database format, in place, without a new first check ("Upgrading the chain
  state" while it starts). After that, beta 3 or earlier can no longer open it. Stay
  on beta 4 or later.

## New in beta 3

- **Expected finish.** While the first check runs, the Go Max page shows when it
  should be done. The speed is measured on this PC, and the 2022-23 spam years
  (blocks 1,710,000 to 2,100,000), the slowest stretch, are counted in. A line under
  the status says whether the spam years are still ahead, under way or done.
- **Data overview** on the Go Max page: downloaded, kept on this PC, removed after
  check, and uploaded. Every block is checked on this PC first, then old block data
  is removed. Counted on this PC from the node's own counters; nothing is sent.
- **Data in "Before you start Go Max":** the first sync downloads about 350 GB. If
  Windows marks the connection as metered, Lock it says so. It never stops you
  starting.
- **Choose where the node is kept.** "Before you start Go Max" lists this PC's
  internal drives with their free space and whether each is an SSD or a hard drive,
  and recommends one: an SSD with room to spare. Change it later from "Check this
  computer and drive" while Go Max is off: move the node's data (copied and checked,
  then the old copy is removed) or start fresh on the new drive.
- **Battery, like Lock it Mac.** "Only sync while plugged in" now pauses only while
  the node is catching up, and only after about a minute on battery, so a short
  unplug changes nothing. Up to date, the node keeps running on battery.
- **Go Max is private, and says why.** Your node connects to other Zcash nodes but
  accepts no connections, so this PC is never announced as a Zcash node. "What is
  this?" under the switch explains it, and where to help the network more: Zecky
  Rock it, a full node on an Umbrel at home.
- **Node updated to Zakura 1.8.8** (was 1.8.5): finds other nodes even when the DNS
  seeds fail, and reads stored blocks faster. Your synced node data carries over; no
  new first check.

## Good to know

- **The first check can take days on an older laptop, and downloads about 350 GB.**
  A recent PC with an SSD needs about 14 hours; an older laptop or a hard drive
  several days. Use a connection without a data limit. Your wallet works meanwhile,
  from its server.
- **The node follows the chain up to block 3,643,309** (about 5 December), as before:
  Zakura has not yet released a node that goes further. Lock it says so two weeks
  ahead.
- Help for common problems: `READ-ME-FIRST.txt`, next to the app.

## Disclaimer

Self-custody: nobody can recover lost words, and payments are final. Zecky Lock it
is provided as is, without warranty of any kind; you use it at your own risk. See
`LICENSE-AND-DISCLAIMER.txt` (shown by the installer) and `LICENSE`.
