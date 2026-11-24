# KohdaLab PLE 1.0.2

- Windows installer packaging works with Python 3.15 and retains Windows path validation. The packaging test suite also runs on Python 3.11–3.14. / Windowsインストーラーの作成がPython 3.15に対応しました。Windowsのファイル名検証を維持し、Python 3.11〜3.14でも試験しています。
- Updated the GUI framework and plotting library together while keeping Rust 1.92 as the minimum build version. Patched a transitive `url` dependency covered by RUSTSEC-2026-0592. / GUI・グラフ用ライブラリを同時に更新し、ビルドに必要な最低Rust版は1.92です。間接依存`url`の脆弱性（RUSTSEC-2026-0592）を修正しました。
- Added Linux build and test coverage and RustSec checks to CI. Internal refactoring retains measurement order, Stop handling, saved-run formats and the GUI API contract. / Linuxのビルド・テストとRustSec監査をCIに追加しました。内部構成を整理し、測定順序・停止処理・保存形式・GUI APIは維持しました。

Windows x64 Setup and portable ZIP, macOS Apple silicon app ZIP, matching source and SHA-256 checksums are provided. Windows binaries are unsigned; macOS uses ad hoc signing. Instrument acquisition requires Windows and the relevant manufacturer SDKs. / Windows x64 Setupとportable ZIP、macOS Apple siliconアプリZIP、対応ソース、SHA-256を配布します。Windows版は未署名、macOS版はアドホック署名です。実機取得にはWindowsとメーカーSDKが必要です。

Validation covers Demo, software tests and package checks. Refer to [the hardware record](https://github.com/woolen-yarn/kohdalab-ple/blob/v1.0.2/docs/validation/hardware-1.0.0.md) for the scope of previous instrument checks. / 今回の検証範囲はDemo、ソフトウェア試験、配布物です。過去の実機確認の範囲は[実機記録](https://github.com/woolen-yarn/kohdalab-ple/blob/v1.0.2/docs/validation/hardware-1.0.0.md)をご覧ください。
