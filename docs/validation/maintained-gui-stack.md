# Maintained GUI stack / GUI依存の保守更新

Unreleased: eframe/egui 0.34.3, egui_plot 0.35 and read-fonts replace the unmaintained font parser. Minimum desktop Rust is 1.92. App lifecycle and tests use the new root UI; measurement behavior, layouts and assertions are unchanged.

未公開の変更としてeframe/egui 0.34.3・egui_plot 0.35・read-fontsへ移行し、保守終了したフォント解析依存を置換しました。Desktopの最低Rust版は1.92です。測定動作・配置・既存の期待値を維持し、GUIとテストのライフサイクルを新APIへ移行しています。

Local evidence: 122 Rust tests, strict Clippy and native Mac Demo common-control verification passed. License collection for Windows and Mac reports zero missing texts, with pinned upstream revisions and hashes. Final Windows/Mac distribution CI is required separately.

ローカルではRust 122件、strict Clippy、Mac実GUIのDemo API検証が成功しました。Windows/Macのライセンス収集は不足0件で、上流のコミットとハッシュを記録しています。最終Windows/Mac配布CIは別途確認します。

New physical fault/stop tests were waived by the operator this round; prior use is not a newly passed hardware test.

利用者の指示により今回の実機異常・停止試験は省略しています。運用実績を今回の実機試験合格として記録しません。

## Common SDK lease race / 共通SDKの制御権競合

The Python SDK now serializes lease transitions and rejects retired heartbeat
workers by generation. A heartbeat queued before voluntary release cannot poison
the next acquisition. Real active-lease revocation remains an error. Deterministic
regression tests cover both cases; the common SDK suite passes 16 tests.

Python SDKでは制御権の遷移を排他化し、終了したheartbeatを世代で判別します。
正常な解放の前に待機していたheartbeatが次の制御権取得を誤って失敗させる競合を修正しました。
実際の制御権喪失は引き続きエラーとし、両ケースの再現試験を含む共通SDK 16件が成功しています。
