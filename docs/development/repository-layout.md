# Repository layout / リポジトリ構成

This repository follows [KohdaLab TRKR 066f8968](https://github.com/woolen-yarn/kohdalab-trkr/tree/066f89686efc1155156b0d079fcb1766d897d0f4): a root Cargo workspace, product crates, optional scripting, focused scripts, and classified documentation.

[TRKR 066f8968](https://github.com/woolen-yarn/kohdalab-trkr/tree/066f89686efc1155156b0d079fcb1766d897d0f4)と、ルートCargo・製品crates・任意のスクリプト連携・目的別スクリプト・文書分類を統一しています。

| Path / 配置 | Responsibility / 責務 |
| --- | --- |
| `Cargo.toml`, `Cargo.lock` | Only authoritative workspace and lockfile / 唯一のworkspaceと依存固定 |
| `crates/ple-core/` | Plans, analysis, BG, storage / 計画・解析・BG・保存 |
| `crates/ple-desktop/` | GUI, run coordination and device actors / GUI・測定調整・機器アクター |
| `bindings/python/` | Thin GUI client / 薄いGUIクライアント |
| `bridge/` | Vendor-SDK process boundary / メーカーSDKとのプロセス境界 |
| `servers/conex-cc/` | Controller-PC service integration slot / 制御PCサーバー受け入れ先 |
| `scripts/` | Build, package, validation entry points / ビルド・配布・検証の入口 |
| `tests/` | Device-free integration and packaging checks / 実機を使わない連携・配布検査 |
| `docs/guides/` | Installation and operation / 導入・操作 |
| `docs/development/` | Architecture and release maintenance / 設計・配布保守 |
| `docs/reference/` | Protocols, hardware and file contracts / 通信・機器・ファイル仕様 |
| `docs/validation/`, `docs/images/` | Historical evidence and figures / 検証履歴と図 |
| `third_party/` | Upstream notices and provenance / 上流の表記と出典 |
| `target/`, `build/`, `dist/`, `output/` | Ignored generated outputs / Git対象外の生成物 |

Optional directories are added when they have a role. PLE keeps its SDK adapters and remote-server boundary rather than importing TRKR's GPIB helpers. No cross-repository runtime dependency is introduced. / 任意のフォルダーは役割がある場合に追加します。PLEのSDK・リモートサーバー境界を維持し、リポジトリ間の実行時依存は導入しません。

Keep the landing README short; detailed procedures belong under `guides/`, design under `development/`, stable contracts under `reference/`, immutable evidence under `validation/`. Update consumers when moving paths. Existing flat document paths remain compatibility entries. / READMEは短い入口とし、操作・設計・仕様・検証を分類します。移動時は参照側も更新し、旧フラットパスは案内ページとして残します。
