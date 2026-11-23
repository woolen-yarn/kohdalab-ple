# KohdaLab PLE 1.0.1

## Changes / 変更点

Validate the target of Stop, preserve linked job completion, and use the shared TRKR/PLE CONEX serial lock. Update the external-control bar and common SDK.

## Validation scope / 検証範囲

The previous hardware compatibility and safety guidance remains applicable. The release software checks include simulation. New physical-device operating scenarios are outside this qualification.

See SHA256SUMS.txt and the release page's platform packages. / 対応OSとSHA-256は配布ページをご覧ください。

## Previous release / 前の版

### KohdaLab PLE 1.0.0

## Initial release / 初回リリース

PLE acquisition with matched BG subtraction, live spectra and heatmaps, remote CONEX CC control, power normalization, and raw SPE/array recording. / BG差分・ライブスペクトル・ヒートマップ・リモートCONEX CC制御・電力正規化・生SPE/配列保存に対応したPLE測定アプリです。

- Connect through an explicitly configured native TLPMX 64-bit SDK and read back sensor identity, correction wavelength, averaging and native W/dBm values. Python helpers are preferred when present. / 明示指定したnative TLPMX 64-bit SDKで接続し、センサー情報・補正波長・平均時間・W/dBmの読値を取得します。既存Python helperがある場合は従来経路を優先します。
- Read calibrated W/I columns in LightField CSV exports. Convert canonical Windows paths at the SDK boundary to fix Experiment Conflict before BG acquisition. / 実測LightField CSVの校正済みW/I列に対応し、SDKへ渡すWindowsパス表記を修正してBG取得前のExperiment Conflictを解消しました。
- Embed the spectral icon in Windows executables and macOS app bundles. / Windows実行ファイルとmacOSアプリにスペクトルアイコンを組み込みました。

## Compatibility / 互換性

Profiles, legacy helpers and saved-run readers retain compatibility. Measurement columns, saved formats, SHA-256 coverage and API schemas are unchanged. C3 remains deferred. Manufacturer SDK/DLL files are not redistributed. / 既存設定・従来helper・旧runを維持します。測定列・保存形式・SHA-256対象・APIスキーマは変更しません。C3は保留です。メーカーSDK/DLLは同梱しません。

## Packages and validation / 配布と検証

Windows x64 Setup and portable ZIP, macOS Apple-silicon app ZIP, corresponding source and SHA-256 checksums are provided. Windows packages are unsigned; the macOS app is ad hoc signed without Apple notarization. Hardware acquisition requires Windows, 64-bit Python and the relevant manufacturer SDKs. / Windows x64 Setup・portable ZIP、macOS Apple siliconアプリZIP、対応ソース、SHA-256を配布します。Windows版は未署名、macOS版はアドホック署名でApple公証はありません。実機取得にはWindows・64-bit Python・対象メーカーSDKが必要です。

The operator reported hardware confirmation on 2026-11-23. Retained automated evidence covers individual shutter/stage operations, TLPMX connection/readback and LightField BG acquisition/export. Those logs do not establish an illuminated end-to-end PLE run or quantitative calibration. See [hardware evidence and limits](docs/validation/hardware-1.0.0.md); release build checks are recorded in VALIDATION.json. / 2026-11-23に操作者から実機確認済みの報告を受けました。保存済み自動証跡はシャッター・ステージ個別動作、TLPMX接続・読戻し、LightField BG取得・エクスポートの範囲です。そのログから光照射下のPLE通し測定や定量校正の合格は主張しません。[実機証跡と検証範囲](docs/validation/hardware-1.0.0.md)を参照してください。配布ビルドの検証はVALIDATION.jsonへ記録します。
