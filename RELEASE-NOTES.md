# KohdaLab PLE 0.3.6 preview

## Changes

On first normal GUI launch, a missing `~/.kohdalab/ple/config.json` (`%USERPROFILE%\.kohdalab\ple\config.json` on Windows) is automatically created from safe built-in defaults. No manual JSON preparation is needed.

## Compatibility and safety

Initialization publishes a complete file atomically without replacing an existing profile. Existing invalid, foreign or unsupported profiles remain intact and errors are reported. Explicit profiles and Demo/smoke launches do not initialize production defaults. Creating/loading settings does not connect hardware, enable outputs, acquire images or move shutters. Existing acquisition, data formats, native/common APIs and cleanup policies are unchanged.

## Downloads and validation

Use the installer, portable package and corresponding source/build kit attached to this release. Platform tests and artifact provenance qualify the exact packaged source; first-launch regression checks include concurrent creation, preservation and unusable directories. No new physical-device qualification is claimed. Windows executables remain unsigned; supported Mac packages use ad hoc signing and are not notarized.

## 日本語

初回の通常GUI起動時に、アプリ専用の標準設定を自動作成します。既存・不正・未対応の設定は保持し、エラーを表示します。設定作成・読込による装置接続・出力ON・撮影・シャッター移動は行いません。従来の測定・保存形式・API・終了処理は変更しません。
