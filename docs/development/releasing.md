# Build and release / ビルド・配布

[Repository layout / 構成](repository-layout.md) · [Signing / 署名](../../CODE_SIGNING_POLICY.md) · [Redistribution / 再配布](../../REDISTRIBUTION.md)

## Shared conventions / 共通ルール

As in TRKR, the root Cargo workspace is authoritative. `scripts/build_release.py` is the release entry point. Source, Windows x64 and macOS arm64 ZIPs use the workspace version, with per-platform SHA-256 files. Manufacturer DLLs, private profiles, real runs, and build outputs are excluded from the source archive. Windows additionally provides a versioned NSIS Setup.exe from the same payload; disk images are not generated.

TRKR同様、ルートCargoを唯一のソースとし、`scripts/build_release.py`を配布入口にします。ソース・Windows x64・macOS arm64のZIP名はworkspace版に従い、プラットフォーム別SHA-256を併置します。ソースZIPからメーカーDLL・私有設定・実測・ビルド出力を除きます。Windowsでは同じpayloadから版付きNSIS Setup.exeも作ります。DMGは生成しません。

## Local package / ローカル配布物

Build scripts require Python 3.11 or later; this does not change the packaged Demo runtime requirements. / ビルドスクリプトにはPython 3.11以降を使います。配布済みDemoの実行条件は変わりません。

Run the platform command on its intended build OS, with Cargo dependencies and the corresponding Rust target installed. The staging helpers collect locked dependency licenses offline after a successful build. Use a fresh output directory: the archive writer refuses to overwrite an existing ZIP.

対応するビルドOSで実行し、Cargo依存物とRust targetを事前に用意します。staging helperはビルド後に固定依存物のライセンスをオフライン収集します。既存ZIPは上書きしないため、新しい出力先を使ってください。

```powershell
python scripts/build_release.py --platform windows-x64 --with-installer --shutter-source build/shutter-source --output dist/release-windows
```

```sh
python3 scripts/build_release.py --platform macos-arm64 --output dist/release-macos
python3 scripts/build_release.py --platform source --output dist/release-source
```

If the platform stage already exists, use `--no-build`, and optionally `--portable-dir` or `--macos-app`. A Windows cross-build stage can be packaged this way on macOS, but it is not a Windows runtime validation.

stageが既にある場合は`--no-build`、必要なら`--portable-dir`・`--macos-app`を指定します。macOS上で作ったWindowsクロスビルドのstageも梱包できますが、Windows実行の確認とは区別します。

## GitHub workflows / GitHubでの運用

- `test.yml`: source checks, Rust tests/lints, device-free Python tests, build and Windows profile CLI check. / ソース検査、Rust、実機なしPython、ビルド、Windows設定CLI確認。
- `docs.yml`: local Markdown/HTML link validation. / ローカル文書リンクの検証。
- `release.yml`: manual artifact builds or `v*` tags. A tag must exactly match the Cargo version. Only tag builds publish a GitHub prerelease after verification and both platform packages succeed. / 手動の成果物作成または`v*`タグ。タグはCargo版と完全一致させ、検証と両OSの梱包成功後、タグ実行だけがGitHub prereleaseを作成します。

`workflow_dispatch` creates downloadable workflow artifacts without publishing a release. Publishing a tag is a deliberate maintainer action; it does not change repository visibility. Hardware validation remains a separate recorded step. / 手動実行は公開リリースを作らずworkflow artifactを出力します。タグ送信は保守者が明示的に行い、リポジトリの公開範囲は変えません。実機確認は別途記録します。

## Reproducibility / 再現性

ZIP member ordering and timestamps are fixed (`SOURCE_DATE_EPOCH`, default 1980-01-01). This makes archives reproducible for identical staged files in the same compression environment; it does not claim byte-identical native builds across compilers or machines. Keep checksums, commit/tag, SDK versions and qualification records together.

ZIPの順序と時刻は固定します（`SOURCE_DATE_EPOCH`、既定1980-01-01）。同じstage・圧縮環境での再現性を対象とし、異なるcompilerやPCでネイティブ実行ファイルが完全一致することは主張しません。チェックサム・コミット／タグ・SDK版・実機確認を一緒に保持してください。

macOS bundle versions are derived from the workspace manifest. Both platform staging helpers assemble fresh directories and preserve the previous package until the new one is complete. / Macのbundle版番号はworkspaceから取得します。両OSのstageは新規フォルダーで作り、完成まで前版を保持します。

## Release evidence / 配布証跡

TRKR/IVと同様に、具体的な版のReleaseへZIPとSHA-256をまとめます。Windows版には`START-HERE.md`、`windows-imports.txt`、`windows-metadata.json`を含め、展開したexeの版番号と設定検査をWindowsで実行します。全検証・両OSビルド成功後に、`scripts/release_metadata.py`で各ZIP／Windows exeのハッシュを再照合し、`BUILD-ORIGIN.json`、`VALIDATION.json`、統合`SHA256SUMS.txt`を作成します。結果はソフト検証に限定し、実機検証やネイティブGUI画面の試験と混同しません。

## Shutter companion / 同梱シャッターGUI

Windows CI checks out `woolen-yarn/kohdalab-shutter` at `8d22245899c36206622341ed5957919508aac0a1` under `build/shutter-source`. `--shutter-source build/shutter-source` builds its GUI/CLI with the same static-CRT option, checks imports/CLI help, retains matching source/MIT/Cargo licenses, and stages it under `companions/kohdalab-shutter`. Release metadata requires the companion and verifies its revision/protocol/exe hashes before publishing. No upstream files or firmware are modified. Local custom builds without `--shutter-source` use a separately installed compatible shutter GUI.

## Windows Setup

`scripts/build_windows_installer.py` uses the suite-standard NSIS template in `packaging/windows`. The pinned NSIS 3.12 compiler is checksum-verified and kept in `build/tools`, without system installation. The complete inspected stage is installed in `Program Files\Kohdalab\KohdaLab PLE`. Existing application folders are moved to a sibling backup before activation; user configuration/results remain outside Program Files. Uninstall removes only the exact packaged files, without recursive deletion.

Windows CI first exercises the installer shell with a small fixture. After the real native build, `scripts/verify_windows_installer.py` checks every installed payload hash, installed version/configuration/Python discovery, repeat/update, refusal of locked PLE/shutter, shortcuts/registration and removal while retaining user data and previous-folder operator files. These checks run only in disposable GitHub Actions users. `windows-installer-metadata.json` and `windows-installer-validation.json` bind the Setup SHA-256 and payload hash to the portable ZIP; publish refuses mismatches.
