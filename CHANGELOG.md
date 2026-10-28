# Changelog / 変更履歴

## Unreleased / 未リリース

- Align repository landing page, documentation categories, issue/PR templates and release entry points with KohdaLab TRKR. / TRKRとREADME・文書分類・Issue/PR・リリース入口を統一。
- Add official SDK download links and the pending CONEX CC server integration directory. / SDK入手先と後日提供されるサーバーの受け入れ先を追加。
- Fix a Windows-only unused test import exposed by GitHub CI. / GitHub CIで判明したWindows限定の未使用importを修正。

## 0.2.1 — 2026-10-29

- Confirm Stop/idle after SDK errors and timeouts; retain failed connection owners and cleanly shut down SDKs. / SDK異常後の停止確認、失敗接続の保持、終了処理を強化。
- Add recovery UI/API guards, CLI validation, stronger run verification and repeatable package staging. / 復帰操作、起動検証、保存検査、配布作成を強化。

## 0.2.0 — 2026-10-29

- Add power-wavelength tracking, connected spectrometer Apply/readback, raw-first persistence and Windows cross-build. / 波長追従、分光器設定適用、生データ先行保存、Windowsクロスビルド。
