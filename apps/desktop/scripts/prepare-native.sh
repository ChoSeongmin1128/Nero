#!/bin/bash
set -euo pipefail
if [[ $# -ne 0 && $# -ne 3 ]]; then
  echo 'Usage: prepare-native.sh [XCODE_RELEASE_DIRECTORY CHECKOUTS_DIRECTORY PATCH_RECEIPT]' >&2
  exit 2
fi
root="$(cd "$(dirname "$0")/../../.." && pwd)"
output="$root/apps/desktop/.generated/native"
mkdir -p "$output"
if [[ "$(uname -s)" != Darwin ]]; then
  echo 'MOSS native preparation currently requires macOS.' >&2
  exit 1
fi
xcrun swiftc -O -parse-as-library -target "$(uname -m)-apple-macos26.0" \
  "$root/Sources/GroveInference/InferenceConfiguration.swift" \
  "$root/Sources/GroveInference/AnalysisAudioPreparer.swift" \
  "$root/scripts/audio-prepare/AudioPrepare.swift" -o "$output/AudioPrepare"
codesign --force --sign - "$output/AudioPrepare"
if [[ $# -eq 0 ]]; then
  echo 'AudioPrepare ready. Supply matching Release products, package checkouts and the runtime patch receipt to stage MOSS.'
  exit 0
fi
"$root/scripts/native-moss-harness/package_worker.sh" "$1" "$2" "$output/moss" "$3"
chmod -R u+rwX "$output/moss"
