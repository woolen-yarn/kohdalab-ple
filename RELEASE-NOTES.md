# KohdaLab PLE 0.3.13 Preview

## Changes / 変更内容

- The CONEX reference adapter requires a shared token before listening outside IPv4 loopback. Local use on the default `127.0.0.1` address can still omit the token. / CONEX参照アダプターはIPv4ループバック外で待ち受ける場合に共有トークンが必要です。既定の `127.0.0.1` では引き続きトークンを省略できます。
- LightField settings are checked immediately after background acquisition. If a required setting cannot be read, PLE names the setting, retains the acquired data, and stops before signal acquisition. A changed setting is reported separately. / BG取得直後にLightField設定を検査します。必要な項目を読み戻せない場合は項目名を表示し、取得済みデータを保持して信号測定前に停止します。設定の変化は別のエラーとして表示します。
- Run JSON files are replaced in one operation. At finalization, PLE compares `points.csv` with the sample metadata and saved acquisition count. A mismatch leaves the raw data available and records `incomplete` and the reason in `analysis.run.json`. Open Results ignores legacy `*.previous.json` files. / runのJSONを一度の操作で置換します。終了時に `points.csv`、サンプルメタデータ、保存済み取得数を照合し、不整合時は生データを保持したうえで `analysis.run.json` に `incomplete` と理由を記録します。Open Resultsは旧runの `*.previous.json` を読み飛ばします。
- Windows Open Results opens the selected folder. The PLE GUI API now advertises the existing behavior that a valid token can request Stop without a lease. / WindowsのOpen Resultsで選択したフォルダーを開けるようにしました。PLE GUI APIの能力宣言に、有効なトークンでleaseなしにStopを要求できる既存動作を反映しました。

[PR #16](https://github.com/woolen-yarn/kohdalab-ple/pull/16) · [PR #17](https://github.com/woolen-yarn/kohdalab-ple/pull/17)

## Compatibility / 互換性

If you use the reference adapter across PCs, set `KOHDA_CONEX_TOKEN` or `--token` on the controller PC and set the environment variable named by `stage.token_env` to the same secret on the acquisition PC. Existing profiles and saved data remain readable. The CSV columns, units and SHA-256 coverage are unchanged.

参照アダプターを別PCから使う場合は、制御PCで `KOHDA_CONEX_TOKEN` または `--token` を設定し、取得PCの `stage.token_env` が指す環境変数に同じトークンを設定してください。既存の設定と保存データは引き続き読めます。CSVの列・単位・SHA-256の対象は従来どおりです。

## Packages and validation / 配布と検証

The tag-driven workflow builds Windows x64 Setup and portable ZIP, the macOS Apple-silicon app ZIP, and corresponding source. The release includes SHA-256 checksums and `VALIDATION.json`. Windows packages are unsigned. The macOS app uses ad hoc signing and has no Apple notarization.

タグ起動のワークフローでWindows x64 Setup・portable ZIP、macOS Apple siliconアプリZIP、対応ソースをビルドします。配布版にはSHA-256と `VALIDATION.json` が付きます。Windows版は未署名です。macOS版はアドホック署名で、Apple公証は受けていません。

This Preview is for hardware qualification. Packaged applications need no Rust. Demo needs no instruments, and physical acquisition requires Windows, 64-bit Python and the manufacturer software. Software tests, installer checks and Demo runs provide software evidence. Physical instruments remain unqualified for this Preview.

実機確認用Previewです。配布版の起動にRustは不要で、Demoは機器不要です。実機取得にはWindows・64-bit Python・メーカーソフトが必要です。ソフトウェア試験、インストーラー検査、Demo測定はソフトウェアの検証です。このPreviewの実機検証は未実施です。
