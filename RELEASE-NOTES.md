# KohdaLab PLE 0.3.8 preview

## Changes

The canonical configuration is `~/.kohdalab/ple/config/config.json` (Windows: `%USERPROFILE%\.kohdalab\ple\config\config.json`). Runtime API descriptors use `ple/run`; default measurements and application logs use sibling `results` and `logs` where applicable. Camera exports continue to honor the user-selected folder. The shared path definition is vendored identically in all five applications.

## Compatibility and safety

A missing default is created atomically on the first normal GUI launch. Existing settings, including invalid/foreign/unsupported files, are preserved with visible errors. Old direct-child, shared, preview and AppData profiles are not imported or removed automatically; use Browse/Load after reviewing an existing profile. Explicit profile and output paths remain respected. No device connection, source output, capture or shutter movement is added. Scientific file formats and control ownership/cleanup policies are unchanged.

The common Python control client now ignores empty native home variables in favor of a valid platform fallback. Its response reader checks only newly received bytes for the JSON-lines delimiter, preserving the existing response limit and error contract.

The bundled common SDK remains 1.0.1 and wire protocol stays at version 1. Existing profiles, result paths, raw/processed CSV schemas, measurement analysis, control ownership and shutdown policy are unchanged. The separately pinned Shutter companion remains unchanged.

## Downloads and validation

Windows Setup/portable, supported Mac distribution and corresponding source/build kit are attached. Windows artifacts remain unsigned; supported Mac applications are ad hoc signed and not notarized. Normal GUI startup is tested in isolated user homes, including restart/invalid-file preservation and canonical config/run paths. Platform tests, installer evidence and artifact/source hashes qualify the attached builds; no new physical-device qualification is claimed.

## 日本語

共通Pythonクライアントでは、空のネイティブホーム環境変数を有効なOS側フォールバックで補います。API応答では新しく受信した断片だけからJSON Linesの区切りを検出し、既存のサイズ制限とエラー契約を維持します。

共通SDKは1.0.1、通信プロトコルはv1のままです。既存プロファイル・結果保存先・raw/processed CSV形式・測定解析・制御権・終了方針は変更しません。別途固定しているShutter companionも変更しません。
