#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
ple_cargo="${PLE_CARGO:-cargo}"
if [ "${1:-}" != "--built" ]; then
  "$ple_cargo" build --release --locked --bin kohdalab-ple
fi
ple_app="dist/KohdaLab PLE.app"
mkdir -p "$ple_app/Contents/MacOS" "$ple_app/Contents/Resources/bridge" "$ple_app/Contents/Resources/examples" "$ple_app/Contents/Resources/scripts"
cp target/release/kohdalab-ple "$ple_app/Contents/MacOS/kohdalab-ple"
cp bridge/conex_server.py bridge/ple_bridge.py bridge/sdk_check.py bridge/requirements.txt bridge/README.md "$ple_app/Contents/Resources/bridge/"
cp scripts/verify_run.py scripts/verify_gui_api.py scripts/collect_licenses.py "$ple_app/Contents/Resources/scripts/"
cp examples/*.json "$ple_app/Contents/Resources/examples/"
mkdir -p "$ple_app/Contents/Resources/bindings/python/kohdalab_ple"
cp bindings/python/pyproject.toml "$ple_app/Contents/Resources/bindings/python/"
cp bindings/python/kohdalab_ple/*.py "$ple_app/Contents/Resources/bindings/python/kohdalab_ple/"
cp -R docs "$ple_app/Contents/Resources/"
cp README.md LICENSE "$ple_app/Contents/Resources/"
cp -R third_party "$ple_app/Contents/Resources/"
ple_licenses=$(mktemp -d)
trap 'rm -rf "$ple_licenses"' EXIT
python3 scripts/collect_licenses.py --cargo "$ple_cargo" --target aarch64-apple-darwin --output "$ple_licenses/rust"
rm -rf "$ple_app/Contents/Resources/third_party/rust"
cp -R "$ple_licenses/rust" "$ple_app/Contents/Resources/third_party/"
cat > "$ple_app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>kohdalab-ple</string>
<key>CFBundleIdentifier</key><string>org.kohdalab.ple</string>
<key>CFBundleName</key><string>KohdaLab PLE</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.2.1</string>
<key>CFBundleVersion</key><string>3</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
codesign --force --sign - "$ple_app"
printf '%s\n' "$ple_app"
