# KohdaLab PLE v0.3.11 — Windows実機検証の入口

[ダウンロード表](README.md#downloads--ダウンロード)からWindows x64のSetup.exeとSHA-256を取得し、照合してから管理者権限でインストールします。スタートメニューの **KohdaLab → PLE → KohdaLab PLE** から起動します。portable ZIPの場合は全体を展開して `kohdalab-ple.exe` を起動します。配布版の起動にRustは不要で、DemoにはPythonも機器も不要です。

アプリ本体は`C:\Program Files\Kohdalab\KohdaLab PLE`、設定は`%USERPROFILE%\.kohdalab\ple\config\config.json`、既定の測定結果は`%USERPROFILE%\.kohdalab\ple\results`です。実行時APIの検出ファイルは`run`、ログは`logs`に保存します。初回の通常起動時に、既定の設定ファイルがなければ作成します。実機用の設定はこのフォルダーへ別ファイルとして置き、GUIで読み込みます。更新時の旧アプリは隣接フォルダーに退避し、設定・測定結果はアンインストール後も残ります。

## 実機PCへ用意するもの

- 取得PC：64-bit Windows Python、LightFieldとAutomation SDK、PM100USB/PM100D用の旧TLPM helperと対応ドライバー。
- CONEX制御PC：Newport SDKとCONEXサーバー。既定はX=COM3、Y=COM4。今回のPLE走査はY、0–25 mmです。
- レーザー3900S、分光器HRS-300。カメラ・グレーティング・パワーセンサーは接続時に自動取得します。SDKが公開しない情報だけ補足します。

メーカーSDK・ドライバー・外部Thorlabs helperは別途導入します。[導入・公式ダウンロード](docs/guides/install.md)と[装置リスト・接続準備](docs/reference/laboratory-apparatus.md)を確認してください。

## 最初の接続と測定

1. アプリのインストール先、またはportable版の展開先にある `examples/ple-hardware-settings.json` を設定フォルダーへ `hardware-settings.local.json` としてコピーします。Python、LightField experiment、CONEXのhost/port、パワーメーターのhelperフォルダーとVISA resourceを実値に置き換えます。位置・波長の校正点には実測値を使います。
2. PowerShellでアプリのインストール先、またはportable版の展開先を開き、実機取得に使う64-bit Pythonで `python -m pip install -r bridge/requirements.txt` を実行します。`LIGHTFIELD_ROOT` は[導入手順](docs/guides/install.md)に従って設定し、サーバーが認証を使う場合は[CONEXサーバー手順](servers/conex-cc/README.md)に従って設定します。
3. 同じフォルダーから、機器を動かさずに設定とSDKの導入状態を確認します。

   ```powershell
   $profilePath = Join-Path $env:USERPROFILE '.kohdalab\ple\config\hardware-settings.local.json'
   powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\preflight.ps1 -ConfigPath $profilePath -AppPath .\kohdalab-ple.exe -SdkImports
   ```

4. CONEXサーバーを制御PCで起動します。PLEで実機設定を読み込み、Deviceから各機器を明示的に接続します。読戻した型番・serial・グレーティング候補を確認します。
5. [実機確認チェックリスト](docs/guides/hardware-qualification.md)に沿って、各機器とStopを個別に確認します。その後、手動BGまたは設定済みの自動BGで、最初のPLのpeak/積分区間を決めて少数点のPLEを測定します。
6. `analysis.csv`、`analysis.raw.csv`、`analysis.log`、`analysis.run.json` の4ファイルと、生SPE・背景・設定を含むrunフォルダー全体を保管します。`python scripts/verify_run.py <runフォルダー> --json` で保存整合性を検査します。

同じ取得スペクトルに励起ピークとPLが入ることが必要です。搭載光学系、フィルター、検出器範囲、飽和、実測位置–波長校正、センサーの補正波長は現地で確認します。Connectはレーザー始動やCONEX原点復帰を行いません。

この配布版は実機確認用Previewです。[配布検証記録](docs/validation/README.md)にはWindowsの版・設定検査や隔離環境でのSetup試験があります。測定PCでのGUI起動、メーカーSDKとの接続、実測と停止は現地で確認してください。実行ファイルの依存関係は同梱の `windows-imports.txt` を参照してください。

## 外部APIとkohdalab-shutter

通常のPLE GUIには同じPCの`GuiClient()`から接続できます。`python -m pip install ./bindings/python`でクライアントを導入します。[外部API](docs/reference/gui-api.md)と[kohdalab-shutter自動BG](docs/reference/shutter.md)を参照してください。

自動BGを使う場合は、スタートメニューの **Shutter (automatic BG)**、または同梱の`companions/kohdalab-shutter/Start-Shutter.cmd`でGUIを起動し、Picoを接続してLocal control APIを有効化します。PLEで`examples/ple-shutter-settings.json`を基に実接続先を設定し、Automatic shutter → kohdalab-shutterと物理CHを選びます。GUI/APIが複数の場合だけendpointファイルを明示します。
