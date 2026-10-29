# KohdaLab PLE v0.3.3 — Windows実機検証の入口

推奨は`KohdaLab-PLE-0.3.3-Setup.exe`です。実行して管理者権限を許可し、インストール後はスタートメニューの **KohdaLab → PLE → KohdaLab PLE** から起動します。portable ZIPを使う場合は全体を展開し、`kohdalab-ple.exe`を起動します。アプリの利用にRustは不要です。まずDemoで画面を確認できます。

アプリ本体は`C:\Program Files\Kohdalab\KohdaLab PLE`、設定は`%USERPROFILE%\.kohdalab\ple\config.json`、既定の測定結果は同フォルダーの`results`です。ユーザー用の実機設定はこの設定フォルダーへコピーしてください。インストール先のexamplesを直接編集しません。更新時の旧アプリは隣接フォルダーに退避し、設定・測定結果はアンインストール後も残ります。

## 実機PCへ用意するもの

- 取得PC：64-bit Windows Python、LightFieldとAutomation SDK、PM100USB/PM100D用の確認済み旧TLPM環境。
- CONEX制御PC：Newport SDKとCONEXサーバー。既定はX=COM3、Y=COM4。今回のPLE走査はY、0–25 mmです。
- レーザー3900S、分光器HRS-300。カメラ・グレーティング・パワーセンサーは接続時に自動取得します。SDKが公開しない情報だけ補足します。

メーカーSDK・ドライバー・外部Thorlabs helperは別途導入します。[導入・公式ダウンロード](docs/guides/install.md)と[装置リスト・接続準備](docs/reference/laboratory-apparatus.md)を確認してください。

## 最初の接続と測定

1. `examples/ple-hardware-settings.json`を`hardware-settings.local.json`へコピーします。Python、LightField experiment、CONEXの実host/port、powerのhelperフォルダーとUSB resourceを実値へ置き換えます。`enter_...`と校正例をそのまま使いません。
2. PowerShellで展開フォルダーに移動し、`python -m pip install -r bridge/requirements.txt`を実行します。`LIGHTFIELD_ROOT`と必要なサーバー認証を[導入手順](docs/guides/install.md)に従って設定します。
3. 機器を動かさない検査：`powershell -NoProfile -ExecutionPolicy Bypass -File scripts/preflight.ps1 -ConfigPath hardware-settings.local.json -AppPath kohdalab-ple.exe -SdkImports`。
4. CONEXサーバーを制御PCで起動します。PLEで実機設定を読み込み、Deviceから各機器を明示的に接続します。自動取得した型番・serial・グレーティング候補を確認します。
5. [実機確認チェックリスト](docs/guides/hardware-qualification.md)に沿って、各機器の単独確認とStopを試します。その後、手動BG → 最初のPL → peak/積分区間の確認 → 少数点PLEへ進みます。
6. runに`settings.json`、`hardware.json`、生SPE/CSV、解析結果があることを確認し、`python scripts/verify_run.py <runフォルダー> --json`で保存整合性を検査します。

同じ取得スペクトルに励起ピークとPLが入ることが必要です。搭載光学系、フィルター、検出器範囲、飽和、実測位置–波長校正、センサーの補正波長は現地で確認します。Connectはレーザー始動やCONEX原点復帰を行いません。

この版は実機検証用プレビューです。ソフトウェアの自動検査と、設置済み機器の動作・校正確認を区別します。詳細な結果はReleaseの`VALIDATION.json`、実行ファイルの依存関係は`windows-imports.txt`を参照してください。

## 外部APIとkohdalab-shutter

通常のPLE GUIには同じPCの`GuiClient()`から接続できます。`python -m pip install ./bindings/python`でクライアントを導入します。[外部API](docs/reference/gui-api.md)と[kohdalab-shutter自動BG](docs/reference/shutter.md)を参照してください。

自動BGを使う場合は、スタートメニューの **Shutter (automatic BG)**、または同梱の`companions/kohdalab-shutter/Start-Shutter.cmd`でGUIを起動し、Picoを接続してLocal control APIを有効化します。PLEで`examples/ple-shutter-settings.json`を基に実接続先を設定し、Automatic shutter → kohdalab-shutterと物理CHを選びます。GUI/APIが複数の場合だけendpointファイルを明示します。
