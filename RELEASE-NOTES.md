# KohdaLab PLE 0.3.8 preview

## Changes

The bundled common Python control client now ignores an empty native home-directory environment variable and uses a valid platform fallback. Its JSON Lines response reader scans only newly received bytes, while retaining the established response-size limit and error behavior.

## Compatibility and safety

The common SDK remains 1.0.1 and the control wire protocol remains version 1. Existing profiles, result paths, raw and processed CSV schemas, measurement analysis, control ownership, and shutdown policy are unchanged. The separately pinned `kohdalab-shutter` companion is unchanged.

## Downloads and validation

This private preview provides Windows x64 Setup and portable ZIP, macOS Apple-silicon app ZIP, and source/build evidence. Windows artifacts are unsigned; the macOS app is ad hoc signed and not notarized. CI checks software packaging, installer evidence, and artifact/source hashes. It does not constitute new qualification of LightField, stages, meters, shutters, or an optical setup.

## 日本語

同梱の共通Python制御クライアントは、OS固有のホームディレクトリ環境変数が空の場合に有効なプラットフォーム側フォールバックを使用します。JSON Lines応答は新しく受信したバイト列だけを走査し、既存の応答サイズ上限とエラー動作を維持します。

## 互換性と安全性

共通SDKは1.0.1、制御通信プロトコルはv1のままです。既存プロファイル、結果保存先、raw/processed CSV形式、測定解析、制御権、終了方針は変更しません。別途固定している`kohdalab-shutter` companionも変更しません。

## 配布と検証

このprivate previewでは、Windows x64 Setupとportable ZIP、macOS Apple silicon app ZIP、source/build証跡を配布します。Windows成果物は未署名で、macOS appはad hoc署名・未notarizeです。CIはソフトウェア梱包、installer証跡、成果物とソースのハッシュを検証しますが、LightField、ステージ、メーター、シャッター、光学系の新たな実機認定を意味しません。
