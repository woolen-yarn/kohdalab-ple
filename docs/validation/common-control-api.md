# Common control API validation / 共通APIの検証

The 1.0.0 local verification passed 154 Rust tests, 132 Python tests (7 platform-specific skips), 16 shared Python SDK tests and 9 distribution tests. The native Demo GUI acceptance completed six rows with 641 bins, manual and automatic BG, lease-expiry Stop, and disconnect/release. / 1.0.0のローカル検証はRust154件、Python132件（OS依存7件skip）、共通Python SDK16件、配布検証9件に合格しました。Demo GUIでは641 bins・6行の測定、手動／自動BG、lease失効Stop、切断・解放を確認しました。

These are device-free software checks. Windows/macOS package CI evidence is attached to the release; hardware evidence is recorded separately in [hardware qualification](hardware-1.0.0.md). / 実機を使わないソフトウェア検証です。Windows/macOS配布物のCI証跡はReleaseに添付し、実機確認は[別記録](hardware-1.0.0.md)に分けます。

The existing native API, scientific storage and product-specific safety policy remain separate from the additive common namespace. See [the common contract](../reference/common-control-api.md). / 既存API・科学データの保存形式・製品固有の安全方針を維持します。[共通仕様](../reference/common-control-api.md)を参照してください。
