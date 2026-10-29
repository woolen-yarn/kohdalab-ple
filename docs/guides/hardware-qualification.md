# Windows 実機認定チェックリスト — PLE v0.2

この文書は、ビルド済みパッケージを実機で使う前に必要な確認と、証跡の残し方を定めます。未検証の PC・カメラ・ステージ・シャッター・パワーメーターを安全または校正済みとみなすものではありません。

先に[装置リスト・メーカー仕様と接続準備](../reference/laboratory-apparatus.md)を確認します。未確定のカメラ／grating／センサーは現地で記録し、メーカーソフトの単独試験から進めます。

## 確認済みの範囲と未確認の範囲

| 証跡 | 意味 | 確認していないこと |
| --- | --- | --- |
| `x86_64-pc-windows-gnu` のクロスビルド | ビルド PC 上で Windows PE 実行ファイルをリンクできた。 | Windows での起動、UCRT、Python/.NET、実機動作。 |
| `sdk_check.py` / `preflight.ps1` | 設定と SDK/API の導入状態を、機器操作なしで検査できる。 | LightField experiment、COM/VISA/TCP 接続、校正、移動、取得、export、Stop。 |
| 以下の実機認定 | 指定した PC・光学系・機器で記録した試験結果。 | 別 PC、別ドライバ、別カメラ、別アライメントへの一般化。 |

以下の必須項目ごとに、担当者・時刻・結果・証跡を残すまで「実機認定済み」と表示しません。

## インストールとパッケージ確認

1. 取得 PC にパッケージをコピーします。`kohdalab-ple.exe`、`bridge/sdk_check.py`、`bridge/ple_bridge.py`、`scripts/preflight.ps1`、`scripts/verify_run.py`、本書、第三者ライセンス一式があることを確認します。
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

5. `examples/ple-hardware-settings.json` から実機設定を作成し、全ての `enter_...` を実値に置換します。移動中の独立した Stop を確認したい場合は `remote_json` を選びます。`legacy_tcp` は利用可能ですが、リモートから移動停止を確認できる方式ではありません。

## 非操作 preflight

ビームを開ける前・測定を開始する前に実行します。保存設定をアプリで論理検証し、Python と SDK の型/API を検査します。`-SdkImports` を付けても LightField Automation や CONEX controller を生成せず、COM/VISA/TCP 接続、デバイス探索、移動は行いません。

```powershell
.\scripts\preflight.ps1 `
  -ConfigPath .\hardware-settings.json `
  -AppPath .\kohdalab-ple.exe `
  -SdkImports
```

CONEXが別PCの場合、取得PCにNewport SDKは不要です。CONEX制御PC側のSDK型検査を行う場合のみ `-ConexSdkDir` を指定してください。取得PCの環境変数 `CONEX_SDK_DIR` は、そのPCにもSDKを入れた場合だけ設定します。

内部で用いるアプリの論理設定検証は次です。

```powershell
.\kohdalab-ple.exe --check-config --config .\hardware-settings.json
```

preflight JSON と失敗内容を保存してください。成功は「導入・設定の検査成功」に過ぎません。固定センサー波長設定や位置校正プロファイルも、この段階では論理検証のみで、物理的な波長・パワー応答は次項で確認します。

## 実機認定の順序

研究室のレーザー安全手順、インターロック、ビームブロック、ローカル SOP に従います。本書はそれらを代替しません。

1. **LightField / 分光器**：指定 PC で experiment、grating、中心波長、露光、合成フレームを接続し、読戻し値を記録します。dark と signal を取得し、生 `.spe` と LightField CSV を保存します。1 ROI・合成フレームの CSV 解析、取得中 Stop、完了または timeout を確認します。
2. **CONEX-CC**：安全な光路・可動域で Remote JSON server を controller PC に起動します。接続が原点復帰・enable をしないことを確認します。limits/位置を読取り、0–25 mm 外を拒否すること、小移動、読戻し、timeout、移動中の独立 Stop を確認します。controller/SDK version を記録します。`legacy_tcp` の場合はその方式で可能な Stop だけを記録し、独立した監視 Stop を主張しません。
3. **パワーメーター**：VISA resource、センサー波長読戻し、平均、auto-range、単位、安定値をローカル基準と照合します。sample/sensor correction factor を決めて記録します。位置校正モードでは予定範囲を覆う anchor を測定し、独立した波長値と補間結果を照合します。
4. **BG / シャッター**：[kohdalab-shutter](../reference/shutter.md)の場合はGUI API、物理CH、STATE対応firmware、実光路の遮光・開放を確認します。移動・整定後の状態を待ち、通信断・操作権喪失時のClose未確認を成功と扱わないことも試験します。STATEは光センサーの確認ではありません。手動 BG ではビーム遮断・復帰の操作者確認を残します。自動 MFF ではまず安全に遮光した状態で open/closed state 番号を実証し、Stop/error 後に fail-closed となることを確認します。state 番号はセットアップ依存です。
5. **短い PLE**：本番と同じ grating、範囲、peak 探索、解析区間、出力先で保守的な短い scan を行います。BG → 最初の PL → 解析確認 → 残り取得を確認し、別 run で安全な Stop も確認します。どちらの run も上書きしません。

## 保存 run の整合性検査

各認定 scan の後に、読み取り専用の検証を実行します。

```powershell
python .\scripts\verify_run.py C:\MeasurementData\PLE\sample_<run-id> --json
```

この検証器は raw/BG/metadata/processed CSV の対応、`corrected = raw - background`、eV 変換、パワー正規化、point identity、可動範囲、hardware run に必要な生 SPE の存在を確認します。SPE の画素内容や物理校正の妥当性までは検査しません。JSON 出力を証跡とともに保存します。

## 証跡フォーム

| 項目 | 記録 |
| --- | --- |
| 日時、操作者、PC hostname | |
| package version、exe SHA-256、`windows-imports.txt` | |
| Windows edition/build、UCRT 状態 | |
| Python path/version/bitness、`pip freeze` | |
| LightField/camera/Automation version、experiment、grating | |
| CONEX serial、firmware、SDK version、Remote JSON revision | |
| power-meter serial、VISA resource、sensor wavelength、correction factor | |
| shutter serial と確認済み open/closed state（使用時） | |
| config SHA-256、preflight JSON の保存先と結果 | |
| raw run directory、verifier JSON、保存 SPE/CSV 数 | |
| LightField Stop、CONEX Stop、shutter fail-closed の証跡 | |
| 例外、対処、reviewer sign-off | |

このフォームは変更しない run directory と一緒に保存します。失敗・中断・部分完了 run も重要な証跡です。検証器を通すためにファイルを書き換えません。
