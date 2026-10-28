# Contributing / 開発への参加

Use focused changes and the checks in [README](README.md). Keep measurement/analysis in Rust, vendor SDK calls in `bridge/`, remote controller services in `servers/`, and GUI automation in `bindings/`. Read [repository layout](docs/development/repository-layout.md) before changing paths.

変更は対象を絞り、[README](README.md)の検証を実行します。測定・解析はRust、SDKは`bridge/`、制御PCサービスは`servers/`、外部操作は`bindings/`に置きます。パス変更前に構成ガイドを確認してください。

Tests and CI must not connect instruments, home/move axes, open beams, or change drivers. Preserve scientific definitions, thresholds, stored raw data and historical evidence. Do not commit real runs, tokens, private profiles or vendor DLLs. Keep upstream notices intact. Document hardware-tested scope separately from simulated checks.

テスト・CIで実機接続、原点復帰・移動、ビーム開放、ドライバー変更をしません。科学的な定義・閾値・生データ・過去の証跡を維持し、実測データ・トークン・私有設定・メーカーDLLはコミットしません。上流表記を保持し、実機確認と模擬確認を区別してください。
