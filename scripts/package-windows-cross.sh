#!/usr/bin/env sh
# Create a Windows release candidate on a macOS/Linux cross-build host.
# This script never substitutes a placeholder executable: cargo must successfully
# produce the PE binary before any distribution directory is made.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
target=x86_64-pc-windows-gnu
target_dir=${CARGO_TARGET_DIR:-"$root/target"}
out=${1:-"$root/dist/kohdalab-ple-windows-cross"}
cc=${CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER:-x86_64-w64-mingw32-gcc}
ar=${CARGO_TARGET_X86_64_PC_WINDOWS_GNU_AR:-x86_64-w64-mingw32-ar}
objdump=${WINDOWS_OBJDUMP:-x86_64-w64-mingw32-objdump}

if [ -e "$out" ]; then
    printf '%s\n' "refusing to overwrite existing output: $out" >&2
    exit 2
fi
command -v cargo >/dev/null
command -v rustup >/dev/null
command -v "$cc" >/dev/null
command -v "$ar" >/dev/null
command -v "$objdump" >/dev/null
command -v jq >/dev/null

cd "$root"
if ! rustup target list --installed | grep -Fqx "$target"; then
    printf '%s\n' "Rust target is not installed: $target. Run: rustup target add $target" >&2
    exit 2
fi
export CARGO_TARGET_DIR="$target_dir"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$cc"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_AR="$ar"
export CC_x86_64_pc_windows_gnu="$cc"
export AR_x86_64_pc_windows_gnu="$ar"
cargo build --offline --release --locked --target "$target" --bin kohdalab-ple

exe="$target_dir/$target/release/kohdalab-ple.exe"
if [ ! -s "$exe" ]; then
    printf '%s\n' "cargo reported success but produced no executable: $exe" >&2
    exit 1
fi

mkdir -p "$out"
"$objdump" -p "$exe" | sed -n 's/^[[:space:]]*DLL Name: //p' | sort -fu > "$out/windows-imports.txt"
if grep -Eqi '^(libgcc|libwinpthread|libstdc\+\+)' "$out/windows-imports.txt"; then
    printf '%s\n' 'dynamic MinGW runtime DLL detected; refuse incomplete distribution' >&2
    cat "$out/windows-imports.txt" >&2
    rm -f "$out/windows-imports.txt"
    rmdir "$out"
    exit 1
fi

cp "$exe" "$out/"
cp README.md LICENSE CHANGELOG.md CODE_SIGNING_POLICY.md CONTRIBUTING.md PRIVACY.md REDISTRIBUTION.md ROADMAP.md SAFETY.md SECURITY.md "$out/"
cp -R bridge bindings examples docs servers third_party "$out/"
mkdir "$out/scripts"
cp scripts/preflight.ps1 scripts/verify_run.py scripts/verify_gui_api.py scripts/collect_licenses.py "$out/scripts/"
python3 scripts/collect_licenses.py --target "$target" --output "$out/third_party/rust"
cargo metadata --offline --locked --format-version 1 |
    jq -r '.packages[] | select(.source != null) | [.name, .version, (.license // "NOASSERTION")] | @tsv' |
    sort -u > "$out/rust-dependency-licenses.tsv"
find "$out" -type d -name __pycache__ -prune -exec rm -rf {} +
find "$out" -type f -name '*.pyc' -delete
cat > "$out/WINDOWS_RUNTIME.md" <<'EOF'
# Windows runtime requirements

`windows-imports.txt` is generated from the executable used in this package.
This package contains no vendor DLLs. Install LightField Automation, the
Thorlabs software/DLL permitted by its vendor license, and 64-bit Python on the
acquisition PC separately. The executable imports the Windows Universal CRT;
use a supported Windows installation with the Microsoft Universal C Runtime
(or the Microsoft Visual C++ Redistributable supplied by Microsoft when needed).

Run `scripts/preflight.ps1` before physical qualification and retain its report.
Use `scripts/verify_run.py` on every saved qualification run. Both are described
in `docs/hardware-qualification.md`; preflight does not connect to instruments
or move the stage.

`third_party/rust/NOTICE.json` and the adjacent package directories preserve
license texts for the locked Rust dependencies. Supplemental upstream sources
and their hashes are recorded in `third_party/rust-upstream/provenance.json`.
EOF
printf '%s\n' "Windows release candidate: $out"
