# Changelog / 変更履歴

## Unreleased / 未リリース

- Keep separate recovery latches for spectrometer Stop, shutter cleanup, stage Stop and a run interrupted with unconfirmed device state. A LightField idle confirmation no longer clears an unconfirmed shutter close; a shutter latch is cleared by a spectrometer disconnect that confirms closure. / 分光器Stop・シャッター・ステージStop・未確認のまま中断した測定の復帰ラッチを個別に保持します。LightFieldのidle確認ではシャッター閉の未確認を解除せず、シャッターのラッチは分光器切断時の閉確認で解除します。
- Convert a stage readback just outside the calibration range by extrapolating the end segment within the stage position tolerance, and log each such acquisition in `analysis.log`. Commanded positions outside the calibration range are still rejected. / 校正範囲をわずかに外れた読み戻し位置は、ステージ位置許容差以内に限り端の区間の傾きで外挿し、該当する取得を `analysis.log` に記録します。校正範囲外の指令位置は従来どおり拒否します。
- Read bridge stderr as bytes, replace invalid UTF-8, bound line length, and start the bridge with `PYTHONIOENCODING=utf-8`. / bridgeのstderrをバイト列で読み、不正なUTF-8を置換し、行長を制限します。bridgeは `PYTHONIOENCODING=utf-8` で起動します。
- Disconnect the stage only after three consecutive position-monitor failures; a move result takes precedence over an overlapping monitor failure. / 位置監視が3回連続で失敗した場合だけステージを切断し、移動と重なった監視失敗より移動結果を優先します。
- Report the original shutter connection error when automatic BG never acquired a shutter, without raising device recovery. / 自動BGでシャッターを確保できなかった場合は元の接続エラーを返し、機器復帰の表示にしません。

## 0.3.11 — 2026-11-10

- Save analysis CSV, raw CSV, lifecycle log and conditions JSON as a linked, hashed measurement bundle; retain old data and configuration compatibility. / 解析CSV・raw CSV・ライフサイクルlog・条件JSONを参照・ハッシュで結び、既存データ・設定の互換性を維持します。
- Clarify stored units, native versus corrected readings, timestamps and original-data references without changing measurement control. / 保存単位・生値と補正後値・時刻・元データ参照を明確化し、測定制御は維持します。
- Add flattened spectral-bin analysis with mm/nm/W/counts units and explicit spectrum/metadata references; retain SPE, background and full-run Open Results. / mm/nm/W/countsを明記したスペクトルbin解析と元データ参照を追加し、SPE・背景・フォルダー全体のOpen Resultsを維持します。
- Preserve actual/target position, native power value/unit, wavelength and acquisition times in raw events, including partial/error runs. / 部分保存・異常終了時も、実位置／指令値・native power値／単位・波長・取得時刻をrawイベントに残します。

## 0.3.10 — 2026-11-10

- Update the GUI to eframe/egui 0.34 and `read-fonts`, retain numeric-input feedback, and keep controller/API processing active when the window is covered. The minimum desktop Rust version is 1.92. / eframe/egui 0.34系と `read-fonts` に更新し、数値入力の反映表示を維持。ウィンドウが隠れても制御・API処理が進むよう描画から分離。デスクトップの最低Rust版は1.92。
- Serialize shared Python control lease transitions and ignore retired heartbeats during reacquisition. Genuine lease loss remains an error. / 共通Python制御クライアントの制御権解放・再取得を直列化し、以前のheartbeatによる再取得の誤失敗を防止。実際の制御権喪失は引き続きエラー。

[Release v0.3.10](https://github.com/woolen-yarn/kohdalab-ple/releases/tag/v0.3.10) · [PR #9](https://github.com/woolen-yarn/kohdalab-ple/pull/9)

## 0.3.9 — 2026-11-06

- Preserve native meter values and units with converted watts, wavelength and acquisition timing. Verify duplicated observations exactly, including picowatt readings and timestamps. / ネイティブ電力値・単位を換算W・波長・取得時刻と共に保存し、pW領域の値や時刻を含めた同一観測の転記値を厳密に照合。
- Match BG using ROI/binning, ADC/readout, temperature setpoint and online correction settings, including hashes for enabled flatfield/blemish references. Keep raw acquisitions when later validation fails. / ROI・binning・ADC・読出し・温度設定値・オンライン補正条件でBGを照合。有効なflatfield／blemish参照のハッシュを記録し、検証失敗時も取得済み生データを保持。
- Bundle the pinned Shutter 0.6.6 companion with its operation journal. / 操作履歴を保存するShutter 0.6.6固定版をWindowsパッケージに同梱。
- Publish the Preview and correct the Shutter attribution in the repository notice. / Previewを一般公開し、リポジトリの第三者表記にあるShutterの出典を訂正。

[Release v0.3.9](https://github.com/woolen-yarn/kohdalab-ple/releases/tag/v0.3.9) · [PR #6](https://github.com/woolen-yarn/kohdalab-ple/pull/6)

## 0.3.8 — 2026-11-05

- Ignore empty home-directory environment values and use the valid platform fallback consistently in the GUI and common Python client. / GUIと共通Pythonクライアントで、空のホーム環境変数を除外して有効な代替先を使います。
- Search only newly received bytes for the common API response delimiter, preserving existing size limits and error handling. / 共通APIの受信済みデータを繰り返し走査せず、新しい断片だけで区切りを検出します。サイズ制限とエラー処理は維持します。

## 0.3.7

- Use the common app-owned directory layout: config/config.json, run, results and logs beneath ~/.kohdalab/ple.
- Align normal startup, file selection, external API/Notebook discovery and documentation. Preserve operator-selected paths and all old files without automatic migration.
- Common control SDK 1.0.1 discovers app-owned run directories; wire protocol and instrument safety are unchanged.


## 0.3.6

- Automatically create the missing application-specific default configuration on first normal GUI launch.
- Preserve existing/corrupt profiles with atomic no-clobber creation and report initialization errors. Do not connect hardware or enable outputs.


## 0.3.5 — 2026-10-29

- Add the authenticated local Control Protocol 1 adapter and shared Python SDK. Camera exposes read-only status/configuration; other apps retain their native ownership and safety policies.
- Align common discovery, receipts, structured errors, PR templates and release categories. Existing measurement data and native clients remain supported.
- GUI共通API・独立したPython SDK・PR/Releaseの記載項目を統一。既存測定とデータ形式を維持します。

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
