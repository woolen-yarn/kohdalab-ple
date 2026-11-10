# KohdaLab PLE 0.3.11 preview

## Changes / 変更内容

- Save analysis CSV, raw CSV, lifecycle log and conditions JSON as a linked, hashed measurement bundle; retain old data and configuration compatibility. / 解析CSV・raw CSV・ライフサイクルlog・条件JSONを参照・ハッシュで結び、既存データ・設定の互換性を維持します。
- Clarify stored units, native versus corrected readings, timestamps and original-data references without changing measurement control. / 保存単位・生値と補正後値・時刻・元データ参照を明確化し、測定制御は維持します。
- Add flattened spectral-bin analysis with mm/nm/W/counts units and explicit spectrum/metadata references; retain SPE, background and full-run Open Results. / mm/nm/W/countsを明記したスペクトルbin解析と元データ参照を追加し、SPE・背景・フォルダー全体のOpen Resultsを維持します。
- Preserve actual/target position, native power value/unit, wavelength and acquisition times in raw events, including partial/error runs. / 部分保存・異常終了時も、実位置／指令値・native power値／単位・波長・取得時刻をrawイベントに残します。

## Compatibility / 互換性

Keep all four bundle files together. Existing profiles remain usable without automatic migration. Missing/unqualified readings remain empty/null rather than being relabelled or replaced with zero. Raw values preserve available precision; rounding does not imply calibrated uncertainty.

4ファイルは一緒に保管してください。既存設定は自動移行不要です。欠測・単位未確認値を別単位やゼロに置き換えず、rawは取得精度を保持します。丸め桁数は校正済み不確かさを意味しません。

Preserve the complete native run folder for existing Open Results and reanalysis, including background, SPE/spectra and accepted analysis settings. The Preview channel remains unchanged. Windows physical acquisition requires manufacturer SDKs; the pinned Shutter companion is unchanged.

既存Open Results・再解析には、背景・SPE／スペクトル・採用解析条件を含むフォルダー全体を残してください。Preview扱いは維持します。Windows実機取得にはメーカーSDKが必要で、固定版Shutter companionは変更しません。

## Packages and validation / 配布と検証

Windows x64 Setup and portable ZIP, macOS Apple-silicon application, corresponding source and SHA-256 checksums are provided. Release validation covers software tests, extracted packages, isolated installer lifecycle and simulation. No new physical-device fault/stop qualification is claimed. Windows binaries are unsigned; Mac apps are ad hoc signed and not notarized.

Windows x64 Setup／portable ZIP、macOS Apple siliconアプリ、対応ソース・SHA-256を提供します。ソフトウェア試験・展開後の配布版・隔離環境のインストーラー動作・模擬測定を確認します。新たな実機異常・停止試験の合格は主張しません。Windowsは未署名、Macはアドホック署名・Apple公証なしです。
