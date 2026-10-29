# Changelog / 変更履歴

## Unreleased / 未リリース

- Derive macOS bundle versions from Cargo and stage fresh bundles transactionally. / Mac版の版番号をCargoから取得し、古いファイルを残さず安全に再作成。

- Preserve upstream license bytes across Windows checkout and verify provenance before compiling. / Windowsで上流ライセンスの改行を変換せず保持し、ビルド前に照合。

- Wait for Windows GUI-subsystem CLI reports in PowerShell and test with a real GUI-subsystem fixture. / PowerShellからGUIアプリの設定確認を行う際、終了と出力を待つよう修正し、Windows用fixtureで検証。

- Align repository landing page, documentation categories, issue/PR templates and release entry points with KohdaLab TRKR. / TRKRとREADME・文書分類・Issue/PR・リリース入口を統一。
- Add official SDK download links and the pending CONEX CC server integration directory. / SDK入手先と後日提供されるサーバーの受け入れ先を追加。
- Fix a Windows-only unused test import exposed by GitHub CI. / GitHub CIで判明したWindows限定の未使用importを修正。
- Preserve executable permissions when extracting macOS release ZIPs and verify the extracted app. / Mac版ZIPの展開時の実行権限を保持し、展開後のアプリを検証。

## 0.2.1 — 2026-10-29

- Confirm Stop/idle after SDK errors and timeouts; retain failed connection owners and cleanly shut down SDKs. / SDK異常後の停止確認、失敗接続の保持、終了処理を強化。
- Add recovery UI/API guards, CLI validation, stronger run verification and repeatable package staging. / 復帰操作、起動検証、保存検査、配布作成を強化。

## 0.2.0 — 2026-10-29

- Add power-wavelength tracking, connected spectrometer Apply/readback, raw-first persistence and Windows cross-build. / 波長追従、分光器設定適用、生データ先行保存、Windowsクロスビルド。
