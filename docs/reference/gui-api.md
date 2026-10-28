# PLE GUI API v1

GUIがLightField、CONEX、パワーメーターを所有します。Python/JupyterはRust GUIへ命令を送り、機器との別接続を作りません。IV/TRKRと同じ操作名・占有方式を採用していますが、PLE固有のAPI v1です。IVのv2クライアントとの直接互換を意味しません。

## 起動と接続

APIは既定で無効です。通常のGUIでの測定方法は変わりません。

```sh
kohdalab-ple --gui-api --config experiment.json
python -m pip install ./bindings/python
```

`127.0.0.1`の空きポートだけで待ち受け、`~/.kohdalab/run/ple-gui-<pid>-<instance>.json`へ発見用ファイルを作ります。`--api-dir DIRECTORY`で専用フォルダーを指定できます。Unixの発見ファイルは0600です。トークンはファイルにのみ記録し、通常ログに出しません。正常終了時に発見ファイルを除去します。クラッシュ後の古いファイルはクライアントが読取り専用の疎通確認で除外します。

```python
from kohdalab_ple import GuiPLE

observer = GuiPLE()  # 状態の確認だけでは操作権を取得しない
print(observer.capabilities())
print(observer.status())
observer.close()

# GUIが複数あるときはendpointを明示する
ple = GuiPLE(endpoint="/path/to/ple-gui-....json")
```

`GuiPLE.launch(executable, profile=..., endpoint_directory=...)` は指定プロファイルで新しいGUIを起動し、そのPIDのendpointへ接続します。明示したプロファイルが不正ならGUI起動を失敗させ、別の既定設定へ切り替えません。

## 操作権と停止

`with GuiPLE(...) as ple:` または `acquire()` で排他的なleaseを取得します。GUI側は設定・接続・Start・手動移動をロックし、プロットの閲覧、Stop、Take controlを残します。ハートビートの有効期限は10秒です。失効・Take controlは所有者を恒久的に失効させ、Stopを要求します。同じクライアントは再取得せず、新しいクライアントを作ります。正常なrelease後は同じクライアントを再利用できます。

Stopは認証トークンがあればleaseなしでも要求できます。Stopの返事は停止要求の受理であり、物理的な停止完了ではありません。`stop_and_wait()` は測定ワーカー・接続処理・CONEXとLightFieldのStop応答待ちを監視し、未確認の状態を成功扱いしません。自動シャッターは閉鎖読戻しを確認します。手動BG方式の遮光はユーザーの操作です。

Pythonの既定動作は、待機の期限切れ、変更要求の応答喪失、失敗した測定の待機終了時にStopを要求します。応答を失った操作を自動再送しません。`close()` / context終了は停止完了を待ってleaseを解放します。`stop_on_timeout=False` はPython側の期限切れ停止を明示的に無効化する選択肢ですが、lease失効時のGUI Stopは残ります。

GUIが応答しない場合はハートビートの監視も遅れます。APIの停止確認は、ハードウェアインターロックの代替ではありません。旧CONEX TCPサーバーにはSTOPがなく、進行中の移動の完了／タイムアウトを待ちます。Remote JSONのStopが未確認なら、遅延したStopが新しい移動へ干渉しないよう再接続・移動を禁止します。

## 主な操作

| 操作 | 内容 |
|---|---|
| `capabilities`, `status`, `devices` | 版、対応操作、phase、機器接続、操作権、停止処理の状態 |
| `config`, `plan`, `output` | 設定、走査計画、保存先を取得 |
| `spectrum`, wireの`preview` / `background` | 最新／最初のPL、生counts、BG差分、BGを取得。描画用に間引いた値ではない |
| `apply_spectrometer(settings, request_id=…)` | 接続済み分光器に露光・合成frames・中心波長・grating・timeoutを適用し読戻すjob。測定・移動中は拒否 |
| `configure(config)` | 全機器切断中にプロファイルを置換。GUIと同じ検証を実施 |
| `set_plan(scan)`, `set_output(directory=..., sample=...)` | アイドル中に走査・保存先を変更 |
| `connect`, `disconnect` | `spectrometer`, `stage`, `power`を順番に処理。途中失敗・Stopで残りを中止 |
| `move(position_mm)` | アイドル中のCONEX絶対移動。0〜25 mm内の設定範囲に限定 |
| `start`, `result(id)`, `wait(id)` | 測定ジョブの開始、対象IDの照会・待機 |
| `confirm_background(True)`, `confirm_signal(True)` | 手動光路を確認した後、そのphaseだけを進める |
| `select_analysis(analysis)` | 最初のPLのピーク・積分範囲を確定し、残りの測定へ進む |
| `stop`, `stop_and_wait` | 停止要求／停止完了待ち |

`wait` と `stop_and_wait` はPythonのポーリングヘルパーです。ネットワークスレッドは機器I/Oをせず、要求はGUIスレッドで検証して既存エンジンへ渡します。HTTP/REST、任意文字列を評価するshell／機器コマンドのaction、headlessモード、C ABIはこの版では提供しません。プロファイルで指定するPython実行ファイル・bridge・ドライバーはGUIと同じく信頼したユーザーが管理します。プロファイル検証はコード実行のsandboxではありません。

## 測定例

```python
from kohdalab_ple import GuiPLE
import time
import uuid

with GuiPLE(endpoint="/path/to/endpoint.json") as ple:
    ple.wait(ple.connect()["id"])
    request_id = str(uuid.uuid4())  # 応答を失っても同じIDを保持する
    job = ple.start(request_id=request_id)
    while True:
        status = ple.status()
        if status["awaiting"] == "background_confirmation":
            # 実際に遮光してから確認する
            input("遮光を確認してEnter: ")
            ple.confirm_background(True)
        elif status["awaiting"] == "signal_confirmation":
            # 実際に光路を戻してから確認する
            input("信号光路の復帰を確認してEnter: ")
            ple.confirm_signal(True)
        elif status["awaiting"] == "analysis_selection":
            preview = ple.spectrum()
            # previewのスペクトルを確認して条件を選ぶ
            input("スペクトルと下記の解析条件を確認してEnter: ")
            ple.select_analysis({
                "peak_nm": 780.0, "peak_half_width_nm": 15.0,
                "integral_min_nm": 765.0, "integral_max_nm": 795.0,
            })
            break
        time.sleep(0.1)
    result = ple.wait(job["id"], timeout=600)
    print(result["output"])
```

この例の数値は例示であり、実際のスペクトルから選びます。自動シャッター方式では手動BG／光路の確認phaseは省略されます。解析選択のACK後も、励起ピーク判定が不正ならphaseが`analysis_selection`に戻り、`last_error`を返します。ACKは測定完了ではなく要求受理です。

## 要求IDと保存済み結果

開始・接続・切断・移動にはジョブreceiptを返します。`status`は`running`, `completed`, `stopped`, `error`です。receiptは`id`, `kind`, `rows`, `output`, `error`, `error_code`, 最終`snapshot`を持ちます。停止・失敗した測定を完了とみなしません。

設定変更・手動確認・解析選択にもUUIDの`request_id`を付けます。これらはコマンドのACKで、`request_id`, `kind`, `run_job`を付加します。`result(request_id)`で受理内容を照会し、測定の完了は`run_job`に対して待機します。

同じIDと同じpayloadの再送は保存した返事を返し、同じIDで別の要求は`request_id_conflict`にします。Pythonは送信前にIDを作り、`last_request_id`と例外の`request_id`へ保持します。失われた返事を解決する場合、IDを保持したまま`result()`を照会してください。

最後の32ジョブreceiptと32コマンドACKを保持します。保持期間を過ぎたIDでも同じGUIセッション中は再実行せず、`request_id_expired`を返します。総ID数の上限は100,000で、上限後は新しい操作を開始しません。receiptとleaseはGUI再起動を越えて保持しません。外部APIで開始した測定には`automation.json`と`external_start`イベントを保存し、要求IDとrun IDを対応付けます。測定の継続的な証拠はSPE・CSV・JSON・eventsです。

## 通信形式と検証

1 TCP接続につきUTF-8 JSONを1行送ります。`action`または`method`、トークン、変更時のlease、必要なパラメーターをトップレベルに置きます。成功はTRKR式の`ok`, `result`を返し、大きなスペクトルを重複コピーする`data`別名は付けません。失敗は`error`, `error_code`を返します。最大要求1 MiB、同時接続8、待ち行列8、読書き・GUI受付待ち各2秒。期限切れでまだ処理されていない要求は実行しません。

```sh
python scripts/verify_gui_api.py target/release/kohdalab-ple --output-dir /tmp/ple-api-check
```

この検証は専用のDemo GUIを1つだけ起動し、認証・占有、0/25 mm移動、確認要求と開始要求の再送、BG・CSV・JSON、実時間でのheartbeat失効停止を確認します。実機を検証するものではありません。

## v0.2.1の復帰制御

SDKへの取得・設定・シャッター要求が失敗、または応答が期限切れになった場合、Rust側もStop→待機状態・遮光の確認を開始します。確認中は変更操作を拒否し、確認できなければ `start` / `move` / `apply_spectrometer` は `cleanup_unconfirmed` を返します。照会とStop再要求は利用できます。停止待ちが完了した後の切断は、SDKのStop・遮光・Disposeを再度確認する復帰操作です。未確認の接続は保持し、確認済みの旧接続終了前に新しいLightField所有者を起動しません。

`--version` はGUIを起動せず版を返します。未知のオプション、値を欠く `--config` / `--screenshot` / `--api-dir`、設定なしの `--check-config` は非ゼロで終了します。
