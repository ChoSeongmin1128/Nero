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
  echo 'Native audio preparation currently requires macOS.' >&2
  exit 1
fi
if [[ $# -eq 0 ]]; then
  python3 "$root/scripts/native-moss-harness/verify_worker_package.py" verify-staged "$output/moss" --optional
fi
python3 "$root/scripts/native-diarization/prepare.py" --verify-staged "$output/diarization" --target macos-arm64 --optional
xcrun swiftc -O -parse-as-library -target "$(uname -m)-apple-macos26.0" \
  "$root/scripts/audio-prepare/AudioPreparationError.swift" \
  "$root/scripts/audio-prepare/AnalysisAudioPreparer.swift" \
  "$root/scripts/audio-prepare/AudioPrepare.swift" -o "$output/AudioPrepare"
codesign --force --sign - "$output/AudioPrepare"
if [[ $# -eq 0 ]]; then
  echo 'AudioPrepare ready. Prepare the shared speech runtime with scripts/native-diarization/prepare.py --target macos-arm64. MOSS staging remains optional.'
  exit 0
fi
"$root/scripts/native-moss-harness/package_worker.sh" "$1" "$2" "$output/moss" "$3"
chmod -R u+rwX "$output/moss"
python3 "$root/scripts/native-moss-harness/verify_worker_package.py" verify-staged "$output/moss"
