# Measurement output bundles / 測定データの保存セット

This contract describes the working implementation, not a claim about older released packages. / この仕様は本変更の実装を対象とし、過去の配布版に遡って適用されるものではありません。

Keep files from one run together. `run_id` connects records; filenames and SHA-256 hashes in the conditions JSON identify the saved payloads. A completed simulation is not hardware qualification. / 同じ測定のファイルは一緒に保管してください。`run_id`で記録を関連付け、条件JSONのファイル名とSHA-256で保存内容を識別します。模擬測定の成功は実機検証を意味しません。

| File / ファイル | Purpose / 用途 |
| --- | --- |
| `<name>.csv` | Analysis-ready quantities and coordinates, quality flags and units. TRKR uses this file alone for normal voltage-data replay. / 解析用の値・座標・品質フラグ・単位。TRKRの通常の電圧データは、このCSV単体で再生できます。 |
| `<name>.raw.csv` | Instrument responses, native units/coordinates, timing and errors, or explicit references to preserved native payloads. Never infer unobserved device values. / 装置応答、生の単位・座標、取得時間・エラー、または保存されたネイティブデータへの明示的な参照。未取得値を推測で埋めません。 |
| `<name>.log` | Human-readable run lifecycle, progress and failures; not a second dump of every instrument reply. / 測定の開始・進行・終了・失敗を読むためのログ。全通信応答の重複保存ではありません。 |
| `<name>.run.json` | Requested/accepted conditions, observed configuration or references, software identity, simulation/terminal state, bundle manifest and file hashes. / 指定・採用条件、確認済み設定または参照先、ソフトウェア情報、模擬／終了状態、保存ファイル一覧・ハッシュ。 |

## Units and precision / 単位と精度

- Numeric column suffixes name stored units, not display prefixes: `V`, `A`, `W`, `ps`, `mm`, `um`, `deg`, `nm`, `eV`, `s`, `ms`, `px`. GUI `mV`/`µW` labels do not change SI storage values. / 列名の単位は保存値の単位です。GUIの`mV`・`µW`表示に合わせて保存値を無断で変換しません。
- Keep requested targets separate from actual readbacks and sample-corrected values separate from native readings. Unit conversion and calibration are recorded; missing values remain empty/null, not zero. / 指令値・実測値、補正前・試料位置換算後を区別し、換算・校正を記録します。欠測は空欄／nullとし、ゼロで代用しません。
- UTC timestamps identify host observations; elapsed seconds use the application's run clock. Neither establishes the instrument's exposure or ADC instant. / UTC時刻はホストでの観測、経過秒はアプリの測定時計です。装置の露光・ADC時刻と同一とは扱いません。
- Display/export rounding does not imply calibrated uncertainty. Raw values preserve available precision. Counts are detector code/count values, not calibrated photon numbers. / 表示・保存時の丸めは校正済み不確かさを意味しません。rawは得られた精度を保ち、countsを校正済み光子数とは扱いません。

## Application-specific payloads / アプリ固有のデータ

- **TRKR:** signal and sample coordinates in main CSV; native scanner `mm`/`deg`, delay `pulse` and converted `mm`, original power and correction factors in raw evidence. Main lock-in columns are voltage-only; non-voltage/unqualified values stay in raw and are not falsely written under `_V`. / mainは信号と試料座標、rawはscannerの`mm`/`deg`、delayの`pulse`と換算`mm`、元のパワーと補正値です。ロックインのmain列は電圧用で、電圧以外・単位未確認の値を`_V`列に偽って記録しません。
- **IV:** main electrical quantities already carry explicit units; source-terminal values and external-meter values keep their distinct meanings. / 電気量の列名に単位を付け、ソース端子と外部メーターの測定値を混同しません。
- **PLE:** `analysis.csv` contains spectral-bin curves/heatmap values. Preserve the complete run folder, including background, accepted analysis settings and native SPE/spectra, for reanalysis and the existing Open Results workflow. / `analysis.csv`はスペクトル各binの曲線・マップ値です。再解析と既存のOpen Resultsには、背景・採用条件・SPE／スペクトルを含む測定フォルダー全体を残してください。
- **Camera:** PNG/TIFF remains the primary visual result, exact RAW remains the source payload, and CSV contains frame-level metadata rather than millions of pixel rows. Preserve legacy metadata and Bayer TIFF where present. / 主成果物はPNG/TIFF、生データはRAWのままです。CSVは大量の画素行ではなくフレームの記録とし、既存metadata・Bayer TIFFも維持します。
- **Shutter:** this change documents the common convention only; it adds no measurement/export controls. Existing diagnostics retain their own purpose. / 今回は共通仕様の文書化のみで、測定・エクスポート機能を追加しません。既存の診断記録はその用途を維持します。

Do not overwrite prior evidence. Preserve partial files and reported failures when a save cannot finish. Verify completion status and hashes before treating a set as complete; hashes alone do not prove physical accuracy. / 過去の証跡を上書きしません。保存が完了しない場合は部分ファイルと失敗情報を確認してください。完了状態とハッシュを確認してから完全なセットとして扱い、ハッシュだけで物理的な正しさを判断しません。
