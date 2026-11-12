# KohdaLab PLE 0.3.14 Preview

## Changes / 変更内容

Open Results reads saved-run JSON and validates the background wavelength grid on a background thread, so the window can respond during the read. A failed read keeps the current results visible. The app prevents conflicting operations while loading and gives Stop cleanup priority over applying the new results. / Open Resultsは保存済みrunのJSON読み込みとBG波長グリッドの照合をバックグラウンドで実行し、読込中も画面が応答します。読込に失敗しても表示中の結果を保持します。読込中の競合操作を防ぎ、Stop後処理を結果の反映より優先します。

[PR #23](https://github.com/woolen-yarn/kohdalab-ple/pull/23)

## Compatibility / 互換性

Existing profiles and saved runs remain readable. Saved data formats, SHA-256 coverage and API schemas remain unchanged. / 既存の設定と保存済みrunは引き続き読めます。記録形式、SHA-256の対象、APIのスキーマは従来どおりです。

## Packages and validation / 配布と検証

The Preview includes Windows x64 Setup and portable ZIP, a macOS Apple-silicon app ZIP, corresponding source, and SHA-256 checksums. Windows packages are unsigned. The macOS app is ad hoc signed and has no Apple notarization. / Windows x64 Setup・portable ZIP、macOS Apple siliconアプリZIP、対応ソース、SHA-256を配布します。Windows版は未署名です。macOS版はアドホック署名で、Apple公証は受けていません。

The qualification evidence covers software checks. Physical hardware qualification for this Preview is unverified. See `VALIDATION.json` for the release build checks. / 検証証跡はソフトウェア試験の範囲です。このPreviewの実機検証は未確認です。配布ビルドの検証は `VALIDATION.json` を参照してください。
