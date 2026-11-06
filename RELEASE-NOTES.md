# KohdaLab PLE 0.3.9 preview

## Reproducible recording / 再現可能な記録

The run verifier no longer applies a shared absolute tolerance to power and epoch timestamps. It checks duplicated observations exactly and allows relative/ULP rounding only in derived arithmetic. Tests now reject 1 pW versus 2 pW and a ten-second timestamp substitution.

検証ツールの電力・epoch時刻に共通の絶対許容差を使う処理を修正しました。同一観測の転記は厳密一致、計算結果だけ相対／ULP許容差で確認し、1 pWと2 pWの取り違え・10秒の時刻改変を検出します。

Native meter values/units now survive bridge-to-Rust acquisition and are saved alongside converted W, wavelength and timing. LightField BG matching now checks ordered ROI/binning, ADC gain/speed/quality/depth, readout mode, temperature setpoint and documented online corrections. Enabled flatfield/blemish reference files are hashed. Missing critical evidence blocks normalization; volatile temperature readings are recorded separately. Settings are queried read-only through documented SDK interfaces.

メーターのネイティブ値・単位をRust取得経路まで保持し、換算W・波長・時刻と共に保存します。LightFieldのBG照合にROI・binning・ADC・読出し・温度設定値・オンライン補正を追加し、有効なflatfield／blemishの参照ファイルをハッシュ記録します。重要情報が欠けた場合は正規化せず、変動する実温度は別記録とします。メーカー文書で確認した読取り専用APIを使用します。

The Windows companion is updated to the fixed, reviewed Shutter 0.6.6 source, including its persistent operation journal. Protocol 2 and the existing ownership flow remain in use. / Windows同梱Shutterもレビュー済みの0.6.6固定ソースへ更新し、操作履歴保存を含めます。protocol 2と既存の制御権の流れは維持します。

## Compatibility and distribution / 互換性と配布

Existing profiles and saved runs remain readable. Raw vendor files and acquired arrays are retained before post-acquisition matching/analysis. Windows Setup/portable, Mac Apple-silicon app and source remain a private preview; Windows hardware acquisition still needs the manufacturer SDKs. This release does not claim new physical qualification. README storage details are bilingual.

既存設定・保存データの読込は維持します。ベンダーの生ファイルと取得配列を照合・解析前に保持します。Windows Setup／portable・Mac Apple silicon app・sourceをprivate previewとして配布し、Windows実機取得には引き続きメーカーSDKが必要です。新たな実機検証済みとは主張しません。READMEの保存仕様を英日更新しました。
