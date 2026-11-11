# KohdaLab PLE — Windows実機確認チェックリスト

この文書は、ビルド済みパッケージを実機で使う前に必要な確認と、証跡の残し方を定めます。未検証の PC・カメラ・ステージ・シャッター・パワーメーターを安全または校正済みとみなすものではありません。

先に[導入手順](install.md)と[装置リスト・メーカー仕様と接続準備](../reference/laboratory-apparatus.md)を確認し、[実機確認記録の雛形](../validation/hardware-qualification-template.md)をコピーします。未確定のカメラ／grating／センサーは現地で記録し、メーカーソフトの単独試験から進めます。

## 確認済みの範囲と未確認の範囲

| 証跡 | 意味 | 確認していないこと |
| --- | --- | --- |
| [配布CI](../validation/README.md) | Windows CIで展開したexeの版・設定、import、隔離環境でのSetupの導入・更新・削除を確認。 | 測定PCでのWindows GUI起動、Python/.NETとメーカーSDKの組み合わせ、実機動作。 |
| `sdk_check.py` / `preflight.ps1` | 設定と SDK/API の導入状態を、機器操作なしで検査できる。 | LightField experiment、COM/VISA/TCP 接続、校正、移動、取得、export、Stop。 |
| 以下の実機認定 | 指定した PC・光学系・機器で記録した試験結果。 | 別 PC、別ドライバ、別カメラ、別アライメントへの一般化。 |

以下の必須項目ごとに、担当者・時刻・結果・証跡を残すまで「実機認定済み」と表示しません。

## インストールとパッケージ確認

1. 取得PCでPreviewのSetup.exeをインストールするか、portable ZIPを全体展開します。SHA-256を照合し、配布版の版と起動したexeの場所を記録します。`kohdalab-ple.exe`、`bridge/sdk_check.py`、`bridge/ple_bridge.py`、`scripts/preflight.ps1`、`scripts/verify_run.py`、本書、第三者ライセンス一式があることを確認します。
2. 対応する 64-bit Windows で、`windows-imports.txt` に示される UCRT 要件を満たします。LightField Automation、CONEX SDK、Thorlabs helper/DLL はそれぞれ正規のベンダーインストーラーで導入します。PLE パッケージからベンダー DLL をコピー・再配布しません。
3. レビュー済みの 64-bit Python と bridge 依存パッケージを入れます。

   ```powershell
   C:\Path\Python\python.exe -m pip install -r .\bridge\requirements.txt
   ```

4. SDK の場所は明示します。既定のインストール先には依存しません。

   ```powershell
   $env:LIGHTFIELD_ROOT = 'C:\Program Files\Princeton Instruments\LightField'
   # CONEX制御PCで使用するSDKの場所
   $env:CONEX_SDK_DIR = 'C:\Vendor\Newport\CONEX-SDK'
   ```

5. [Windows実機検証の入口](../../START-HERE.md)に従い、`examples/ple-hardware-settings.json` をユーザーの設定フォルダーへ `hardware-settings.local.json` としてコピーし、すべての `enter_...` を実値に置換します。移動中の独立した Stop を確認したい場合は `remote_json` を選びます。`legacy_tcp` は利用可能ですが、リモートから移動停止を確認できる方式ではありません。

## 非操作 preflight

ビームを開ける前・測定を開始する前に実行します。保存設定をアプリで論理検証し、Python と SDK の型/API を検査します。`-SdkImports` を付けても LightField Automation や CONEX controller を生成せず、COM/VISA/TCP 接続、デバイス探索、移動は行いません。

インストール先またはportable版の展開先から実行します。`$profilePath` は、PLEで読み込む設定ファイルのパスに合わせます。

```powershell
$profilePath = Join-Path $env:USERPROFILE '.kohdalab\ple\config\hardware-settings.local.json'
.\scripts\preflight.ps1 `
  -ConfigPath $profilePath `
  -AppPath .\kohdalab-ple.exe `
  -SdkImports
```

CONEXが別PCの場合、取得PCにNewport SDKは不要です。CONEX制御PC側のSDK型検査を行う場合のみ `-ConexSdkDir` を指定してください。取得PCの環境変数 `CONEX_SDK_DIR` は、そのPCにもSDKを入れた場合だけ設定します。

内部で用いるアプリの論理設定検証は次です。

```powershell
.\kohdalab-ple.exe --check-config --config $profilePath | Out-String
```

preflight JSON と失敗内容を保存してください。成功は「導入・設定の検査成功」に過ぎません。固定センサー波長設定や位置校正プロファイルも、この段階では論理検証のみで、物理的な波長・パワー応答は次項で確認します。

## 実機認定の順序

研究室のレーザー安全手順、インターロック、ビームブロック、ローカル SOP に従います。本書はそれらを代替しません。

1. **LightField / 分光器**：指定 PC で experiment、grating、中心波長、露光、合成フレームを接続し、読戻し値を記録します。dark と signal を取得し、生 `.spe` と LightField CSV を保存します。1 ROI・合成フレームの CSV 解析、取得中 Stop、完了または timeout を確認します。
2. **CONEX-CC**：安全な光路・可動域で Remote JSON server を controller PC に起動します。接続が原点復帰・enable をしないことを確認します。limits/位置を読取り、0–25 mm 外を拒否すること、小移動、読戻し、timeout、移動中の独立 Stop を確認します。controller/SDK version を記録します。`legacy_tcp` の場合はその方式で可能な Stop だけを記録し、独立した監視 Stop を主張しません。
3. **パワーメーター**：VISA resource、センサー波長読戻し、平均、auto-range、単位、安定値をローカル基準と照合します。sample/sensor correction factor を決めて記録します。位置校正モードでは予定範囲を覆う anchor を測定し、独立した波長値と補間結果を照合します。
4. **BG / シャッター**
   - 手動BGではビーム遮断・復帰を操作者が確認して記録します。
   - [kohdalab-shutter](../reference/shutter.md)による自動BGでは、取得PCで同梱GUIを起動し、Local control API、STATE対応firmware、Picoと物理CHを確認します。PLEで `Automatic shutter → kohdalab-shutter` を選びます。自動検出で対象を一意に選べない場合はendpointを指定します。安全な条件でBG時のClose、PL前のOpen、完了・Stop・異常時のCloseと制御権解放を確認します。通信断・lease喪失でCloseの読戻しが得られなければ「未確認」と記録します。STATEで確認したコントローラー状態と、実光路で確認した遮光・開放を別々に残します。
   - MFFを直接使う場合は、安全に遮光した状態でシリアル番号、開閉stateと実光路の対応を確認し、終了・Stop・異常時の閉要求と読戻しを記録します。
5. **LightFieldのBG照合**
   - BGと最初のPLについて、露光・合成フレーム・中心波長・グレーティング、LightFieldのBG補正OFF、波長配列を確認します。読戻しの証跡は `metadata/background.json` と取得別JSONに残します。
   - `background_settings` で、順序付きROIとX/Y binning、ADCのgain・speed・quality・bit depth、readout control mode、温度setpoint、オンライン補正（cosmic-ray、flatfield、blemish、orientation）の有効状態と関連設定を照合します。有効なflatfield／blemish補正では参照ファイルのハッシュも確認します。変動する温度の実測値・statusは別観測です。
   - 必須項目の明示的な `unsupported` はその状態を記録します。欠落・不明・読戻し不能やBGとPLの不一致がある場合は、解析・正規化が止まり、取得済みの生SPEと配列が残ることを確認します。[測定・保存仕様](../reference/measurement.md#bgと元データ)
6. **短いPLE**：本番と同じグレーティング、範囲、ピーク探索、解析区間、出力先で短い走査を行います。BG → 最初のPL → 解析確認 → 残り取得を確認し、別runで安全なStopも確認します。両方のrunを証跡として保管します。

## 保存 run の整合性検査

完了した短いPLE runでは、4ファイルがそろい、同じ `run_id` に対応することを確認します。

| ファイル | 確認内容 |
| --- | --- |
| `analysis.csv` | 点・繰り返し・発光ビン、mm／nm／W／countsの列、元配列・取得JSONへの参照。 |
| `analysis.raw.csv` | 取得・移動・終了のイベント、指令／実位置、ネイティブ電力値と単位、換算W、時刻、生データへの参照。 |
| `analysis.log` | 測定の開始・進行・終了と、発生したエラー。 |
| `analysis.run.json` | 条件・終了状態・ファイル一覧と、上の3ファイルに対応するSHA-256。 |

読み取り専用の `verify_run.py` にrunフォルダーを指定します。

```powershell
python .\scripts\verify_run.py 'C:\MeasurementData\PLE\sample_example' --json
```

終了コードが0、JSONの `ok` が `true` であることを確認し、`verified_acquisitions` を計画点数×繰り返し数と照合します。警告があれば内容と判断を記録します。検証器は3ファイルのハッシュ、平坦化したビン数と参照先、raw/BG/metadata/processed CSVの対応、BG差引き・eV・パワー正規化、可動範囲と実機runの生SPEの存在を確認します。SPEの画素内容、物理校正、実光路の遮光は別途確認します。

中断・失敗runもフォルダー全体と検証出力を保管し、完了runと区別して記録します。再解析には4ファイルに加えて、SPE・スペクトル、背景、確定設定を含む元のrunフォルダー全体を使います。

## 証跡の記録

[実機確認記録の雛形](../validation/hardware-qualification-template.md)に、使用したPC・SDK・機器と各確認項目の担当者、時刻、合否、証跡を記録します。preflightとverifierのJSON、Stopの読戻し、runフォルダーを参照できるようにします。失敗時の原本はそのまま保管し、再試験は新しいrunに保存します。
