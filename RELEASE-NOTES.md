# KohdaLab PLE 0.2.2 — Automatic apparatus discovery

This preview prepares the Windows portable application for instrument qualification with a 3900S laser, SpectraPro HRS-300, remote CONEX CC (X=COM3, Y=COM4; Y scan, 0–25 mm), and PM100USB or PM100D.

- Automatically read SDK-exposed camera/spectrometer identities, exact grating choices, meter identity and current sensor information. Retain per-run `hardware.json` and per-point power hardware provenance; preserve manual declarations separately.
- Use grating capabilities reported by the connected LightField experiment. Fix grating/center application order and the original TLPM helper's averaging readback tuple.
- Include `START-HERE.md`, official SDK links, hardware profiles, preflight, SDK bridge, reference CONEX server and qualification checklist in the portable ZIP.
- Verify the extracted Windows executable's version/configuration and its external runtime dependencies. Retain checksums, build origin and validation evidence alongside the packages.

## Downloads

Use `KohdaLab-PLE-v0.2.2-windows-x64.zip` for Windows instruments. Extract the whole archive and open `START-HERE.md`. macOS arm64 is for Demo/results; the source ZIP supports rebuilding. Rust is not required for portable runtime. Windows hardware operation requires separately installed 64-bit Python, LightField Automation and matching manufacturer drivers/helper; vendor DLLs are not bundled.

Windows binaries are unsigned; macOS uses ad hoc signing. This is an instrument-qualification preview, with no new physical camera, laser, stage or power-sensor qualification claimed. See `VALIDATION.json` for the exact software checks.

## 日本語

カメラ・グレーティング・メーター／センサーヘッドの自動取得をWindows ZIPへ反映しました。機器構成を自動記録し、未取得情報は推測で埋めません。取得・接続まわりの修正と、実機検証の入口となる手順・設定例・確認用スクリプトを同梱しています。

まずZIPを全体展開して`START-HERE.md`を開いてください。メーカー環境と実接続先の設定は必要です。実機の初回確認はBG・初回PL・少数点走査から進めます。
