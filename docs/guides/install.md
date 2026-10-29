# Installation and SDK downloads / 導入・SDKダウンロード

[Documentation / 文書一覧](../README.md) · [Hardware / 対応機器](../reference/hardware.md)

## Application / アプリ

Download Setup.exe (recommended) or the whole portable ZIP and its SHA-256 file from [GitHub Releases](https://github.com/woolen-yarn/kohdalab-ple/releases). Extract all files together. Rust is not needed. Demo needs neither Python nor instruments; hardware acquisition uses a Windows PC, 64-bit Python, and the SDK adapters below.

[GitHub Releases](https://github.com/woolen-yarn/kohdalab-ple/releases)からSetup.exeを取得してインストールしてください（管理者権限が必要です）。portable ZIPを使う場合はSHA-256も取得し、全体を展開してください。Rustは不要です。DemoはPython・機器不要ですが、実機取得にはWindows PC・64-bit Python・以下のSDKが必要です。macOS版はDemo・結果確認用です。

## Application locations

The suite uses one application directory per product. PLE settings default to
`~/.kohdalab/ple/config.json` (`%USERPROFILE%\.kohdalab\ple\config.json` on Windows).
A normal launch loads that file if present. Demo screenshots and explicit
`--config` profiles do not read it. Save writes `application: "kohdalab-ple"`;
foreign or damaged profiles cannot be silently overwritten. Default results go to `~/.kohdalab/ple/results`. Example files remain
examples and are never the default Save target.

On macOS, copy `KohdaLab PLE.app` into `/Applications`. The Windows installer
uses `C:\Program Files\Kohdalab\KohdaLab PLE` and creates application-specific
Start menu/desktop shortcuts. Updates activate a fully staged application directory, so removed helpers do not remain active. The previous application folder is retained as a sibling backup (its location is shown in Setup details). Uninstall removes only its packaged files and leaves user settings and measurements.
The portable ZIP remains usable without application installation.

Build Setup on Windows from a reviewed portable folder:

```powershell
python scripts/build_windows_installer.py --payload-dir dist/kohdalab-ple-windows --version 0.3.3 --output dist/KohdaLab-PLE-0.3.3-Setup.exe
```

Pass the actual Cargo package version. NSIS is required; the builder can obtain
a pinned, checksum-verified copy if it is not installed.

## Manufacturer downloads / メーカー公式ダウンロード

Links checked 2026-10-29. Use the official download page rather than copying DLLs from an unrelated installation. Downloading does not establish that a specific version matches the current adapter.

2026-10-29確認。版ごとに変わる直接インストーラーURLではなく、メーカーの配布入口を掲載しています。入手しただけでは現在のアダプターとの互換性確認にはなりません。

| Device / 機器 | Download / ダウンロード | What to obtain / 必要なもの | Install on / 導入先 |
| --- | --- | --- | --- |
| LightField | [LightField: Resources & Support → Software & Firmware](https://www.teledynevisionsolutions.com/products/lightfield/) | LightField installation including Add-in and Automation SDK; manufacturer account/license may be needed / Automation SDKを含むLightField。メーカーのログイン・利用資格が必要な場合あり | Acquisition PC / 取得PC |
| CONEX CC | [Newport CONEX-CC: Resources → Software](https://www.newport.com/f/trb-actuator-conex-kits) | CONEX-CC installer and `Newport.CONEXCC.CommandInterface.dll`; see [official installation manual](https://www.newport.com/medias/sys_master/images/images/hc3/h1d/9044105035806/CONEX-CC-Controller-GUI-Manual.pdf) / インストーラー・Command Interface DLL | Controller PC only for remote operation / リモート運用ではCONEX制御PCのみ |
| Thorlabs power meter | [Legacy power-meter software](https://www.thorlabs.com/software_pages/ViewSoftwarePage.cfm?Code=PM100x) / [Optical Power Monitor](https://www.thorlabs.com/software_pages/ViewSoftwarePage.cfm?Code=OPM) | Driver compatible with legacy `ThorlabsPowerMeter.py`, `TLPM_64.dll`, `.NET Interop`; newer TLPMX is a different API / 元PythonとTLPM系DLLに合う版。TLPMX系をそのまま差し替えない | Meter-connected PC / パワーメーター接続PC |
| Automatic BG / 自動BG | [PLE Windows同梱版](https://github.com/woolen-yarn/kohdalab-ple/releases/tag/v0.3.3) | GUI API protocol 2対応の同梱GUI、STATE対応firmware。PLEはPicoへ直接接続しない | Acquisition PC / 取得PC |
| Optional MFF shutter / 任意のMFF | [Thorlabs motion software](https://www.thorlabs.com/software_pages/ViewSoftwarePage.cfm?Code=Motion_Control) | Appropriate USB/Kinesis prerequisites for the MFF/pylablib setup / 使用するMFFとpylablibの方式に合うUSB・Kinesis環境 | Shutter-connected PC / シャッター接続PC |

LightField SDK information: [manufacturer automation note](https://www.teledynevisionsolutions.com/learn/learning-center/scientific-imaging/instrument-automation-via-national-instruments-labview/). Power-driver version information: [Thorlabs OPM manual](https://media.thorlabs.com/contentassets/012d23129bc54ff2bfdbf4c6d4a306fa/tl_opm_v7.0.pdf). / SDKの所在とドライバーの版の違いは、これらのメーカー資料も参照してください。

Manufacturer DLLs and the original Thorlabs helper are not redistributed here. Keep the reviewed helper in a local driver folder and set `power.driver_path`. The original input ZIP contained the two TLPM DLLs; that is not a full installer or a verified redistribution grant.

メーカーDLLと元のThorlabs helperは本リポジトリでは再配布しません。確認済みhelperをローカルに置き、`power.driver_path`で指定します。元zipのTLPM DLL 2本はインストーラー一式や再配布許諾を意味しません。

## Laboratory apparatus / 使用装置

3900S・HRS-300・PM100USB/PM100D・X=COM3/Y=COM4の[装置リストと設定準備](../reference/laboratory-apparatus.md)を先に確認してください。カメラ、実装グレーティング、センサーヘッドは接続時にSDKから自動取得します。SDKが公開しない項目は現地で補足します。旧TLPM DLLの存在はpreflightで検査しますが、DLLのbitness・依存関係・USBドライバー割当の互換性は実機で確認します。

## Setup / 設定

From the extracted package root / 展開したフォルダーのルートで実行:

```powershell
python -m pip install -r bridge/requirements.txt
```

Set `spectrometer.python` to the actual 64-bit Windows Python executable. Set `LIGHTFIELD_ROOT` to its reviewed installation. On the separate CONEX controller PC, set `CONEX_SDK_DIR` or use the server's `--sdk-dir`; the acquisition PC does not need the Newport SDK for remote CONEX operation. Review grating names, COM/VISA resource, limits and optical state mapping.

`spectrometer.python`は実際の64-bit Windows Pythonを指定し、`LIGHTFIELD_ROOT`はLightFieldの導入先へ設定します。CONEX制御PCでは`CONEX_SDK_DIR`またはサーバーの`--sdk-dir`を指定します。リモートCONEX運用時、取得PCにNewport SDKは不要です。グレーティング名、COM/VISA、可動範囲、遮光状態を確認してください。

Run [non-motion preflight and physical qualification](hardware-qualification.md). The SDK checker probes Python/imports with a 30-second timeout and does not open COM/VISA/TCP or move devices. / [機器を動かさない導入検査と実機確認](hardware-qualification.md)へ進んでください。Python/import検査には30秒の期限があり、COM/VISA/TCP接続や移動は行いません。
