# Suite quality audit: KohdaLab PLE / 品質点検

2026-11-05 · Unreleased branch `codex/suite-quality-audit` / 未公開の作業ブランチ

Base / 基点: `191dd6a31de3930e515194cc22cfd38127ff2483`. This records source and software checks, not hardware qualification or a published release. / ソースとソフトウェア検証の記録であり、実機認定や公開済みリリースの記録ではありません。

## Corrections / 修正

Only the shared path/client corrections are applied. Inspection found no additional confirmed defect in finite-value/range validation, calibration ordering, CSV precision, or retained ownership after failed device disconnection.

共通パス・クライアントの修正を適用します。有限値・範囲・校正順序の検証、CSV精度、切断失敗時の機器管理では、今回追加で確定できる不具合は見つかりませんでした。

All five applications share byte-identical Rust path resolution and Python control client sources. Empty native home variables no longer hide a valid alternative. Python evaluates the account-directory fallback only when needed and rejects a relative fallback. Response framing searches each received fragment once instead of repeatedly scanning all preceding bytes; timeout/size/error contracts are unchanged. No speedup factor is claimed without a benchmark.

5アプリで共通Rustパス処理とPython制御クライアントの内容を一致させています。空のホーム環境変数が有効な代替先を隠す問題を修正しました。Pythonのアカウント由来フォールバックは必要時だけ評価し、相対パスへのフォールバックは拒否します。応答は受信断片ごとに区切りを探し、過去の全データを毎回走査しません。タイムアウト・サイズ制限・エラーの契約は維持し、ベンチマーク未実施の速度倍率は記載しません。

## Audit boundaries / 確認範囲

| Area / 項目 | Evidence and constraint / 確認内容と制約 |
| --- | --- |
| Settings and paths / 設定・保存先 | Native precedence, empty values, default layout, cancelled selection, preserving existing profiles / OS別優先順・空値・既定構成・選択取消・既存設定の保持 |
| Responsiveness / 画面応答 | Worker/pending transitions, bounded operations and logs, actual caller paths / ワーカーと処理中状態・操作数とログの上限・実際の呼出経路 |
| External control / 外部制御 | Ownership and pending-operation guards; fragmented, incomplete and oversized responses / 制御権と操作中の排他・分割応答・未完結応答・サイズ超過 |
| Measurement evidence / 記録 | Existing CSV/image precision and no-overwrite tests; no schema, units, calibration, scan path or protection-policy changes / 精度・上書き防止の既存テスト。スキーマ・単位・校正・走査・保護方針は変更なし |
| Maintenance / 保守 | Shared source parity, regression checks, bilingual notes and local documentation links / 共通ソース一致・回帰検証・英日記録・文書リンク |

## Reproducing checks / 検証方法

Use the repository's pinned toolchain and CI prerequisites. / リポジトリの指定ツールチェーンとCIの前提環境を使用してください。

```sh
cargo fmt --all --check
cargo test --workspace --locked
python -m unittest discover -s bindings/control -p 'test_*.py' -v
python scripts/check_docs.py
```

The common Rust crate is also tested explicitly with `cargo test -p kohdalab-control-protocol --locked`. Repository-specific Python/packaging checks remain in the existing CI workflows. / 共通Rustクレートは `cargo test -p kohdalab-control-protocol --locked` でも検証します。固有のPython・配布テストは既存CIで継続します。

## Local results / ローカル結果

Rust 117 passed; Python 95 completed including 7 platform-specific skips; common SDK 14 passed; strict Clippy and documentation links passed. / Rust117件成功。Python95件完了（OS限定7件スキップを含む）、共通SDK14件成功。警告禁止Clippy・文書リンク検証成功。

## Limits / 未確認範囲

Local verification uses macOS, temporary user profiles, synthetic data and mock transports. It does not initialize or move stages, enable real outputs, acquire from cameras/spectrometers, or operate shutters. Windows-specific native dialogs, manufacturer drivers, installer lifecycle and real-device latency require their corresponding CI/manual qualification. Existing platform-specific skips remain visible; no thresholds or historical scientific fixtures are relaxed.

ローカル検証はmacOS・一時プロファイル・合成データ・模擬通信を使用します。実ステージの初期化や移動、出力ON、実カメラ・分光器の取得、シャッター操作は行いません。Windows固有ダイアログ・メーカー製ドライバー・インストール手順・実機の遅延は、対応CIまたは実機確認が必要です。既存のOS限定スキップを隠さず、判定基準や過去の科学的参照データを緩めていません。
