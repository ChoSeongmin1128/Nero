#!/usr/bin/env bash
set -euo pipefail

configuration="${1:-release}"
case "$configuration" in
  debug|release) ;;
  *)
    echo "Usage: $0 [debug|release]" >&2
    exit 2
    ;;
esac

project_root="$(cd "$(dirname "$0")/.." && pwd)"
repository_root="$(cd "$project_root/../.." && pwd)"
app_path="${GROVE_APP_OUTPUT:-$repository_root/.work/legacy-swift/dist/Grove.app}"
contents_path="$app_path/Contents"
build_path="$repository_root/.work/legacy-swift/swift-build"
binary_path="$build_path/$configuration/Grove"
workers_path="${GROVE_NATIVE_WORKERS_DIR:-$repository_root/.work/legacy-swift/native-workers}"

# Local beta uses already-qualified native workers, never a Python/Homebrew runtime.
for worker in Moss/MossHarness Nemotron/bin/nemo-speech; do
  if [[ ! -x "$workers_path/$worker" ]]; then
    echo "Missing native worker: $worker. Prepare GROVE_NATIVE_WORKERS_DIR before packaging." >&2
    exit 2
  fi
done
test -f "$workers_path/Moss/mlx-swift_Cmlx.bundle/Contents/Resources/default.metallib"

cd "$project_root"
export CLANG_MODULE_CACHE_PATH="$repository_root/.work/legacy-swift/module-cache"
export SWIFTPM_MODULECACHE_OVERRIDE="$repository_root/.work/legacy-swift/module-cache"
swift build --scratch-path "$build_path" --jobs 2 -c "$configuration" --product Grove

if [[ -e "$app_path" ]]; then
  /usr/bin/trash "$app_path"
fi

mkdir -p "$contents_path/MacOS" "$contents_path/Resources"
mkdir -p "$contents_path/Helpers" "$contents_path/Resources/Licenses"
cp "$binary_path" "$contents_path/MacOS/Grove"
cp "$project_root/App/Info.plist" "$contents_path/Info.plist"
cp "$project_root/App/Icon/Grove.icns" "$contents_path/Resources/Grove.icns"
cp -R "$build_path/$configuration/Grove_GroveApp.bundle" "$contents_path/Resources/"
cp "$repository_root/THIRD_PARTY_NOTICES.md" "$contents_path/Resources/"
moss_contents="$contents_path/Helpers/Moss.bundle/Contents"
mkdir -p "$moss_contents/MacOS" "$moss_contents/Resources"
cp "$workers_path/Moss/MossHarness" "$moss_contents/MacOS/MossHarness"
cp "$project_root/App/MossWorker-Info.plist" "$moss_contents/Info.plist"
for bundle in mlx-swift_Cmlx swift-crypto_Crypto swift-transformers_Hub; do
  ditto "$workers_path/Moss/$bundle.bundle" "$moss_contents/Resources/$bundle.bundle"
done
ditto "$workers_path/Moss/Licenses" "$contents_path/Resources/Licenses/Moss"
cp "$workers_path/Moss/Package.resolved" "$contents_path/Resources/Licenses/Moss/Package.resolved"
mkdir -p "$contents_path/Helpers/Nemotron"
ditto "$workers_path/Nemotron/bin" "$contents_path/Helpers/Nemotron/bin"
mkdir -p "$contents_path/Helpers/Nemotron/lib"
cp -P "$workers_path/Nemotron/lib/"*.dylib "$contents_path/Helpers/Nemotron/lib/"
ditto "$workers_path/Nemotron/share/nemo-speech" "$contents_path/Resources/Nemotron"
ditto "$workers_path/Nemotron/share/licenses/nemo-speech" "$contents_path/Resources/Licenses/Nemotron"
while IFS= read -r library; do
  /usr/bin/codesign --force --sign - "$library"
done < <(find "$contents_path/Helpers/Nemotron/lib" -type f -name '*.dylib')
/usr/bin/codesign --force --sign - "$contents_path/Helpers/Nemotron/bin/nemo-speech"
/usr/bin/codesign --force --sign - "$contents_path/Helpers/Moss.bundle"
printf 'APPL????' > "$contents_path/PkgInfo"

/usr/bin/plutil -lint "$contents_path/Info.plist" >/dev/null
/usr/bin/codesign \
  --force \
  --sign - \
  --entitlements "$project_root/App/Grove.entitlements" \
  "$app_path"
/usr/bin/codesign --verify --deep --strict "$app_path"

echo "$app_path"
