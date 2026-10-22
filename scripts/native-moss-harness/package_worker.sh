#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "Usage: $0 XCODE_RELEASE_DIRECTORY CHECKOUTS_DIRECTORY NEW_WORKER_DIRECTORY PATCH_RECEIPT" >&2
  exit 2
fi
products="$(cd "$1" && pwd -P)"
checkouts="$(cd "$2" && pwd -P)"
destination="$3"
patch_receipt="$4"
script_directory="$(cd "$(dirname "$0")" && pwd -P)"
if [[ -e "$destination" ]]; then
  echo "Refusing to overwrite an existing worker directory." >&2
  exit 2
fi
test -x "$products/MossHarness"
for bundle in mlx-swift_Cmlx swift-transformers_Hub swift-crypto_Crypto; do
  test -d "$products/$bundle.bundle"
done
test -f "$products/mlx-swift_Cmlx.bundle/Contents/Resources/default.metallib"
python3 "$script_directory/verify_worker_package.py" preflight "$products" "$checkouts" "$patch_receipt"

mkdir -p "$destination/Licenses"
cp "$products/MossHarness" "$destination/MossHarness"
for bundle in mlx-swift_Cmlx swift-transformers_Hub swift-crypto_Crypto; do
  ditto "$products/$bundle.bundle" "$destination/$bundle.bundle"
done

# Preserve dependency notices beside the statically linked worker. This directory
# intentionally contains no model weights, recordings, benchmark output, or tokens.
for dependency in "$checkouts"/*; do
  [[ -d "$dependency" ]] || continue
  name="$(basename "$dependency")"
  for notice in LICENSE LICENSE.txt LICENSE.md NOTICE NOTICE.txt; do
    if [[ -f "$dependency/$notice" ]]; then
      mkdir -p "$destination/Licenses/$name"
      cp "$dependency/$notice" "$destination/Licenses/$name/$notice"
    fi
  done
done
cp "$script_directory/Package.resolved" "$destination/Package.resolved"
/usr/bin/codesign --force --sign - "$destination/MossHarness"
/usr/bin/codesign --verify --strict "$destination/MossHarness"
python3 "$script_directory/verify_worker_package.py" finalize "$products" "$checkouts" "$destination" "$patch_receipt"
echo "$destination"
