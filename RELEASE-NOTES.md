# KohdaLab PLE 0.3.5 preview

## Changes / 変更内容

Add Control Protocol 1 and the shared Python SDK. Distinguish command ACKs from linked measurement jobs, retain exclusive ownership through unresolved cleanup, and report cleanup failures explicitly. Keep the original GUI measurement/confirmation workflow.

## Compatibility and migration / 互換性と移行

Native clients remain supported; native Stop retains its ownership. Only accepted common release retires its lease after cleanup confirmation. Existing optical cleanup barrier, BG/analysis confirmation, manufacturer SDK requirements and CSV/SPE storage remain. Config stays `~/.kohdalab/ple/config.json`.

## Downloads / ダウンロード

Windows x64 Setup/portable ZIP, macOS Apple-silicon Demo/results ZIP, source and checksums. Physical LightField acquisition still requires Windows, 64-bit Python and matching manufacturer SDKs. Compatible Shutter companion and its source/licenses are included as before. Windows is unsigned; Mac is ad hoc signed, not notarized.

[Release packages](https://github.com/woolen-yarn/kohdalab-ple/releases). Existing older releases remain available.

## Validation and provenance / 検証と出所

Local Rust workspace: 108 passed; strict clippy passed. Actual native Demo GUI completed six rows and 641 spectral bins with ownership, confirmation, idempotency, expiry and cleanup checks. Focused common tests cover failed cleanup, active release and native Stop compatibility.

The common Python SDK has nine device-free tests. Source/build identifiers, package manifests and SHA-256 evidence accompany the release. These checks do not newly qualify physical instruments, optical closure, camera calibration or scientific accuracy.

## 日本語

共通GUI APIを追加し、受付と測定完了、停止確認の未完了・失敗を区別します。従来API、BG・解析確認、CSV/SPEを維持します。実機取得にはWindowsとメーカーSDK等が必要で、MacはDemo・結果確認用です。
