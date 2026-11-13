#!/bin/sh
# Signs SHA256SUMS.txt with your own GPG key, on your Mac.
# Run it in the folder with the release files:  sh sign-release.sh
set -e
command -v gpg >/dev/null || { echo "GnuPG is missing. Install it first: brew install gnupg"; exit 1; }
if [ -z "$(gpg --list-secret-keys 2>/dev/null)" ]; then
  echo "You have no signing key yet. Make one (pick your name or handle):"
  echo '  gpg --quick-generate-key "Your Name <you@example.com>" ed25519 sign 3y'
  exit 1
fi
shasum -a 256 -c SHA256SUMS.txt
gpg --armor --detach-sign --output SHA256SUMS.txt.asc SHA256SUMS.txt
gpg --armor --export "$(gpg --list-secret-keys --with-colons | awk -F: '/^fpr/ {print $10; exit}')" > zecky-release-key.asc
gpg --verify SHA256SUMS.txt.asc SHA256SUMS.txt
echo
echo "Signed. Your key's fingerprint (put it on the release page and on X):"
gpg --list-secret-keys --with-colons | awk -F: '/^fpr/ {print $10; exit}'
