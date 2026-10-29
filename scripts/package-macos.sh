#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
ple_cargo="${PLE_CARGO:-cargo}"
if [ "${1:-}" != "--built" ]; then
  "$ple_cargo" build --release --locked --bin kohdalab-ple
fi
mkdir -p dist
ple_stage=$(mktemp -d dist/.kohdalab-ple-macos-XXXXXX)
trap 'rm -rf "$ple_stage"' EXIT
ple_dest="dist/KohdaLab PLE.app"
ple_app="$ple_stage/KohdaLab PLE.app"
mkdir -p "$ple_app/Contents/MacOS" "$ple_app/Contents/Resources/bridge" "$ple_app/Contents/Resources/examples" "$ple_app/Contents/Resources/scripts"
cp target/release/kohdalab-ple "$ple_app/Contents/MacOS/kohdalab-ple"
cp bridge/conex_server.py bridge/ple_bridge.py bridge/sdk_check.py bridge/requirements.txt bridge/README.md "$ple_app/Contents/Resources/bridge/"
cp scripts/verify_run.py scripts/verify_gui_api.py scripts/collect_licenses.py "$ple_app/Contents/Resources/scripts/"
cp examples/*.json "$ple_app/Contents/Resources/examples/"
mkdir -p "$ple_app/Contents/Resources/bindings/python/kohdalab_ple"
cp bindings/python/pyproject.toml bindings/python/README.md "$ple_app/Contents/Resources/bindings/python/"
cp bindings/python/kohdalab_ple/*.py "$ple_app/Contents/Resources/bindings/python/kohdalab_ple/"
cp -R docs "$ple_app/Contents/Resources/"
cp -R servers "$ple_app/Contents/Resources/"
cp README.md LICENSE CHANGELOG.md CODE_SIGNING_POLICY.md CONTRIBUTING.md PRIVACY.md REDISTRIBUTION.md ROADMAP.md SAFETY.md SECURITY.md "$ple_app/Contents/Resources/"
cp -R third_party "$ple_app/Contents/Resources/"
python3 scripts/collect_licenses.py --cargo "$ple_cargo" --target aarch64-apple-darwin --output "$ple_app/Contents/Resources/third_party/rust"
python3 - "$ple_app/Contents/Info.plist" <<'PY'
import plistlib
import sys
import tomllib
from pathlib import Path
version = tomllib.loads(Path('Cargo.toml').read_text(encoding='utf-8'))['workspace']['package']['version']
numeric_version = version.split('-')[0].split('+')[0]
info = {
    'CFBundleExecutable': 'kohdalab-ple',
    'CFBundleIdentifier': 'org.kohdalab.ple',
    'CFBundleName': 'KohdaLab PLE',
    'CFBundlePackageType': 'APPL',
    'CFBundleShortVersionString': numeric_version,
    'CFBundleVersion': numeric_version,
    'KohdaLabVersion': version,
    'LSMinimumSystemVersion': '12.0',
    'NSHighResolutionCapable': True,
}
with Path(sys.argv[1]).open('wb') as output:
    plistlib.dump(info, output)
PY
codesign --force --sign - "$ple_app"
codesign --verify --deep --strict "$ple_app"
# Do not merge resources from an older app. Keep it until the complete new
# bundle is signed and can be moved into place.
ple_backup=$(mktemp -d dist/.kohdalab-ple-previous-XXXXXX)
if [ -e "$ple_dest" ]; then
  mv "$ple_dest" "$ple_backup/KohdaLab PLE.app"
fi
if ! mv "$ple_app" "$ple_dest"; then
  if [ -e "$ple_backup/KohdaLab PLE.app" ]; then
    mv "$ple_backup/KohdaLab PLE.app" "$ple_dest"
  fi
  exit 1
fi
rm -rf "$ple_backup"
printf '%s\n' "$ple_dest"
