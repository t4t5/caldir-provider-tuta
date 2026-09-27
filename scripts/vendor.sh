#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: scripts/vendor.sh <tutanota-checkout>" >&2
  exit 2
fi

checkout=$(cd "$1" && pwd)
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
revision=$(git -C "$checkout" rev-parse HEAD)
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT

mkdir -p "$staging/tuta-sdk"
cp -a "$checkout/tuta-sdk/rust/sdk" "$staging/tuta-sdk/"
cp -a "$checkout/tuta-sdk/rust/crypto-primitives" "$staging/tuta-sdk/"
cp -a "$checkout/tuta-sdk/rust/util" "$staging/tuta-sdk/"
cp -a "$checkout/LICENSE.txt" "$staging/tuta-sdk/LICENSE.txt"

for patch_file in "$root"/patches/*.patch; do
  patch --directory="$staging/tuta-sdk" --strip=1 --no-backup-if-mismatch < "$patch_file"
done

rm -rf "$root/vendor/tuta-sdk"
mv "$staging/tuta-sdk" "$root/vendor/tuta-sdk"
printf '%s\n' "$revision" > "$root/vendor/UPSTREAM_REV"

# The SDK sends CARGO_PKG_VERSION as its client version, which Tuta rejects once
# it falls too far behind, so keep the workspace version in step with upstream.
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$checkout/Cargo.toml" | head -n 1)
sed -i.bak "/^\[workspace.package\]/,/^\[/ s/^version = \".*\"$/version = \"$version\"/" "$root/Cargo.toml"
rm "$root/Cargo.toml.bak"

echo "vendored Tuta SDK $version at $revision"

