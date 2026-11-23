# Changelog / 変更履歴

## 1.0.1

- `control.stop` validates a supplied `job_id` before stopping; jobless Stop, optical cleanup and linked command IDs are unchanged. / `control.stop`に指定した`job_id`を停止前に検証。ジョブ指定なしのStop・光学後処理・リンク済みコマンドIDは従来どおり。
- The CONEX bridge takes TRKR's cross-process serial-port lock before opening the Newport SDK and refuses a port already in use. The lock is kept when close is unconfirmed. This applies only to the same Windows host and user. / CONEXブリッジがNewport SDKを開く前にTRKRと共通のシリアルポートロックを取得し、使用中のポートを拒否。終了を確認できない場合はロックを保持。同じWindows PC・同一ユーザー間のみ有効。
- External control is shown as a nonmodal top strip with Python and actual activity, then Stop and Take control. / 外部制御をモーダルでない上部帯として表示（Python・実際の動作・Stop・Take control）。

## 1.0.0 — 2026-11-23

Initial official release of KohdaLab PLE. / KohdaLab PLEの初回正式リリース。

- LightField acquisition with matched background subtraction and raw SPE/array recording. / LightField取得・条件照合済みBG差分・生SPE/配列保存。
- Remote CONEX CC scanning, live power readback and normalization. / リモートCONEX CC走査・電力ライブ読戻し・正規化。
- Explicit native TLPMX 64-bit SDK or legacy power-meter helper support. / 明示指定のnative TLPMX 64-bit SDK・従来パワーメーターhelperに対応。
- Live spectra, overlays, heatmaps and analysis selection, with Python GUI API control. / ライブスペクトル・代表曲線・ヒートマップ・解析区間選択・Python GUI API操作。
- Windows Setup/portable packages and macOS app with the spectral icon. / スペクトルアイコン付きWindows Setup・portable版とmacOSアプリ。

See [release notes](RELEASE-NOTES.md) and [validation scope](docs/validation/README.md). / [リリースノート](RELEASE-NOTES.md)と[検証範囲](docs/validation/README.md)を参照してください。
