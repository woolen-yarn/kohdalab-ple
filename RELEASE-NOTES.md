# KohdaLab PLE 0.3.7 preview

## Changes

The canonical configuration is `~/.kohdalab/ple/config/config.json` (Windows: `%USERPROFILE%\.kohdalab\ple\config\config.json`). Runtime API descriptors use `ple/run`; default measurements and application logs use sibling `results` and `logs` where applicable. Camera exports continue to honor the user-selected folder. The shared path definition is vendored identically in all five applications.

## Compatibility and safety

A missing default is created atomically on the first normal GUI launch. Existing settings, including invalid/foreign/unsupported files, are preserved with visible errors. Old direct-child, shared, preview and AppData profiles are not imported or removed automatically; use Browse/Load after reviewing an existing profile. Explicit profile and output paths remain respected. No device connection, source output, capture or shutter movement is added. Scientific file formats and control ownership/cleanup policies are unchanged.

Update the bundled Python clients/common SDK (1.0.1) with the GUI. They discover the new app-owned run directory; legacy discovery is read-only. The wire protocol stays at version 1. Existing cross-process recovery interlocks and OS driver caches retain their safety/OS-managed namespaces rather than being mistaken for GUI configuration.

## Downloads and validation

Windows Setup/portable, supported Mac distribution and corresponding source/build kit are attached. Windows artifacts remain unsigned; supported Mac applications are ad hoc signed and not notarized. Normal GUI startup is tested in isolated user homes, including restart/invalid-file preservation and canonical config/run paths. Platform tests, installer evidence and artifact/source hashes qualify the attached builds; no new physical-device qualification is claimed.

## 日本語

設定は `ple/config/config.json`、実行時APIファイルは同じアプリ配下の `run` に統一します。結果・アプリログもアプリ配下の `results`・`logs` を使います（明示した保存先は保持）。初回は標準設定を自動作成します。旧設定は削除・自動移行せず、不正な設定は上書きせずエラーを表示します。装置への自動接続・出力ONや、測定・データ形式・安全動作の変更はありません。Pythonクライアントも同梱版へ更新してください。
