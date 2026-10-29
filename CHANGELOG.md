# Changelog / 変更履歴

## 0.3.5 — 2026-10-29

- Add the authenticated local Control Protocol 1 adapter and shared Python SDK. Camera exposes read-only status/configuration; other apps retain their native ownership and safety policies.
- Align common discovery, receipts, structured errors, PR templates and release categories. Existing measurement data and native clients remain supported.
- GUI共通API・独立したPython SDK・PR/Releaseの記載項目を統一。既存測定とデータ形式を維持します。

## Unreleased / 未リリース

## 0.3.4 — 2026-10-29

- Accept the application identity marker of saved PLE profiles through the external configure API, while rejecting foreign application profiles.
- Restrict Mac executable discovery to non-Windows hosts so Windows installed-application discovery works independently of a home-directory lookup.

## 0.3.3 — 2026-10-29 (source preparation; no published assets)

- Add NSIS Setup.exe with suite-standard Program Files installation, PLE/shutter shortcuts and an uninstaller; keep portable ZIP. Verify actual install, update, locked binaries, user-data preservation and uninstall on Windows.
- Separate the default profile into `~/.kohdalab/ple/config.json` and results into `~/.kohdalab/ple/results`. Refuse foreign/damaged profiles on load/save and discover the installed executable from the Python client.
- Stage complete updates, retain the previous app folder as a backup, and bind Setup evidence to the portable payload hashes.

## 0.3.2 — 2026-10-29

- Collect the companion’s omitted dependency license texts from the existing package/version/hash-verified PLE provenance without modifying shutter source. Validate the actual Windows dependency graph before release.

## 0.3.1 — 2026-10-29 (packaging retry cancelled before publication)

- Exclude the independently pinned shutter source checkout from the PLE Cargo workspace so the Windows companion builds without modifying upstream source. v0.3.0 packaging failed before publication.

## 0.3.0 — 2026-10-29

- Enable authenticated local GUI API on ordinary startup, add the common `GuiClient` alias and executable discovery, BG configuration and shutter-state API operations. / 通常起動のGUI外部APIとPython/Jupyter利用を整備。
- Add native Rust kohdalab-shutter protocol-2 integration for automatic BG, with exclusive ownership, settled controller state, retained uncertain requests and verified Close/release cleanup. / kohdalab-shutterで自動BG・停止・復旧と測定記録に対応。
- Keep manual BG, legacy MFF profiles and mandatory subtraction; Demo never contacts a physical shutter GUI. / 手動BG・MFF・BG差分を維持し、Demoは実機シャッターへ接続しない。

## 0.2.3 — 2026-10-29

- Compute the Windows binary checksum using .NET so packaging does not depend on optional PowerShell hash cmdlets. / Windowsのハッシュ計算を標準.NETへ変更。

## 0.2.2 — 2026-10-29 (source preparation; no release assets)

- Package automatic apparatus discovery for Windows instrument qualification; add first-run guidance, extracted-package checks, binary dependency inspection and release origin/validation assets. / 自動取得をWindows版へ同梱し、初回案内・ZIP展開後検証・依存関係・配布証跡を追加。

- Automatically discover SDK-exposed camera/grating/meter/sensor information; replace fixed grating presets with reported capabilities and retain per-run and per-point hardware provenance. / 実機情報を自動取得し、グレーティング候補と測定記録へ反映。

- Audit the laboratory apparatus against manufacturer documents, document unknown camera/grating/sensor setup, and add a physical connection sequence. / メーカー仕様との照合・未確定のカメラ等・実機接続手順を追加。
- Fix the original TLPM helper averaging tuple, apply grating before center wavelength, and detect missing DLLs/SDK APIs before acquisition. / 元helperの平均時間読戻し・設定順序・接続前検査を修正。

- Record the 3900S / SpectraPro HRS-300 lab setup, PM100USB or PM100D candidates, and CONEX X=COM3 / Y=COM4 defaults in profiles and saved settings. / 使用予定の機器構成とCONEXの既定ポートを設定・測定記録へ保存。

- Explicitly restore blocking reads in a Windows TCP test fixture without changing Stop assertions. / Windowsの模擬TCP接続の読み取り設定を明示し、Stopの検証条件を維持。

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
