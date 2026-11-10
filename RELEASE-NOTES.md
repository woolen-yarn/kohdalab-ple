# KohdaLab PLE 0.3.10 preview

## Maintained GUI runtime / GUI実行基盤の保守

The desktop GUI now uses the maintained eframe/egui 0.34 stack and `read-fonts` instead of the unmaintained font parser. Numeric-input feedback is retained. Controller and API polling now run independently of visible drawing, so a covered window does not stall background work.

Desktop GUIを保守中のeframe/egui 0.34系へ更新し、保守終了したフォント解析を`read-fonts`へ置き換えました。数値入力の反映表示は維持します。制御・APIの処理を描画から分離し、ウィンドウが隠れた場合も背景処理が停止しないようにしました。

The shared Python control client now serializes lease release/reacquisition and ignores retired heartbeat workers. This prevents a queued old heartbeat from incorrectly failing a later voluntary reacquisition; genuine lease loss remains an error.

共通Python制御クライアントでは、制御権の解放・再取得を直列化し、終了済みheartbeatを無視します。以前のheartbeat待機が次の正常な再取得を誤って失敗させる競合を防ぎ、実際の制御権喪失は引き続きエラーとして扱います。

## Compatibility and distribution / 互換性と配布

Existing profiles, saved runs, raw vendor files and acquired arrays remain readable and are not migrated. Windows Setup/portable, macOS Apple-silicon app and source are published as a GitHub pre-release. The Windows companion remains the pinned Shutter 0.6.6 source required by the current package; this release does not change its protocol or ownership flow.

既存設定・保存データ・ベンダー生ファイル・取得配列は移行不要で、従来どおり読めます。Windows Setup／portable、macOS Apple silicon app、sourceをGitHub pre-releaseとして配布します。Windows同梱Shutterは現行パッケージで必要な0.6.6固定ソースのままで、protocolや制御権の流れは変更しません。

Windows hardware acquisition still requires the manufacturer SDKs. Windows binaries are unsigned; macOS applications use ad hoc signatures and are not notarized. Release checks cover Rust/Python tests, Windows Setup/portable, macOS package, checksums and Simulation/Demo API paths. No new physical-device qualification was performed for this release.

Windows実機取得には引き続きメーカーSDKが必要です。Windows版は未署名、macOS版はアドホック署名でApple公証は受けていません。Rust／Python試験、Windows Setup／portable、macOS配布物、チェックサム、Simulation／Demo API経路を検証しています。今回、新たな実機適合試験は実施していません。
