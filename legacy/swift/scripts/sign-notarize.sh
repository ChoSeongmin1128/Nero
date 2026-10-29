#!/usr/bin/env bash
set -euo pipefail
project_root="$(cd "$(dirname "$0")/.." && pwd)"
repository_root="$(cd "$project_root/../.." && pwd)"
app_path="${GROVE_APP_PATH:-$repository_root/.work/legacy-swift/dist/Grove.app}"
identity="${GROVE_SIGN_IDENTITY:?Set GROVE_SIGN_IDENTITY to the Developer ID Application certificate SHA1}"
grove_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app_path/Contents/Info.plist")"
zip_path="$repository_root/.work/legacy-swift/dist/Grove-$grove_version-arm64.zip"

if ! /usr/bin/security find-identity -v -p codesigning | /usr/bin/awk -v identity="$identity" '$2 == identity && /Developer ID Application:/ { found=1 } END { exit !found }'; then
  echo "A valid Developer ID Application identity is required." >&2
  exit 2
fi

while IFS= read -r -d '' code; do
  case "$(/usr/bin/file -b "$code")" in
    *Mach-O*) /usr/bin/codesign --force --options runtime --timestamp --sign "$identity" "$code" ;;
  esac
done < <(/usr/bin/find "$app_path/Contents" -type f -print0)
/usr/bin/codesign --force --options runtime --timestamp --sign "$identity" "$app_path/Contents/Helpers/Moss.bundle"
/usr/bin/codesign --force --options runtime --timestamp --sign "$identity" --entitlements "$project_root/App/Grove.entitlements" "$app_path"
/usr/bin/codesign --verify --deep --strict "$app_path"

if [[ -e "$zip_path" ]]; then /usr/bin/trash "$zip_path"; fi
/usr/bin/ditto -c -k --keepParent "$app_path" "$zip_path"
notary_arguments=()
if [[ -n "${GROVE_NOTARY_PROFILE:-}" ]]; then
  notary_arguments=(--keychain-profile "$GROVE_NOTARY_PROFILE")
else
  notary_arguments=(--key "${GROVE_NOTARY_KEY_PATH:?Set GROVE_NOTARY_KEY_PATH}" --key-id "${GROVE_NOTARY_KEY_ID:?Set GROVE_NOTARY_KEY_ID}" --issuer "${GROVE_NOTARY_ISSUER_ID:?Set GROVE_NOTARY_ISSUER_ID}")
fi
/usr/bin/xcrun notarytool submit "$zip_path" "${notary_arguments[@]}" --wait --output-format json > "$repository_root/.work/legacy-swift/notarization.json"
python3 - "$repository_root/.work/legacy-swift/notarization.json" <<'PY'
import json,sys
result=json.load(open(sys.argv[1]))
print('Notarization:',result.get('status'))
if result.get('status') != 'Accepted':
    raise SystemExit('Notarization did not pass. Inspect the submission log before distribution.')
PY
/usr/bin/xcrun stapler staple "$app_path"
/usr/bin/xcrun stapler validate "$app_path"
/usr/sbin/spctl --assess --type execute --verbose=2 "$app_path"
/usr/bin/trash "$zip_path"
/usr/bin/ditto -c -k --keepParent "$app_path" "$zip_path"
/usr/bin/shasum -a 256 "$zip_path" > "$zip_path.sha256"
echo "$zip_path"
