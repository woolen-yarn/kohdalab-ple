# kohdalab-shutter and automatic BG / 自動BG

PLEの測定エンジンがBGの取得順序を管理し、[kohdalab-shutter](https://github.com/woolen-yarn/kohdalab-shutter)がPico接続を所有します。PLEはRustから同じPC上のshutter GUI APIへ接続します。PicoへのTCP接続やUDP探索を二重に開きません。

対応契約は[shutter GUI API protocol 2](https://github.com/woolen-yarn/kohdalab-shutter/blob/7e8d1c953905ad92a49a07cd75b916c38a3a518d/docs/gui-api.md)です。任意の旧シャッターサーバーを互換とみなしません。

## 準備

1. GitHubのPLE Windows ZIPにある`companions/kohdalab-shutter/Start-Shutter.cmd`で、API対応の同梱GUIを起動し、対象のPicoへ接続します。既存GUIを使う場合はprotocol 2対応版が必要です。旧v0.1.0 Releaseには今回のAPIがありません。
2. shutter GUIの **Local control API** を有効にします。
3. 使用する物理CHのOpen/Close角度と光路の対応を確認します。`STATE`を返すfirmwareが必要で、Unknownの旧firmwareは使用しません。
4. PLEの **Analysis / BG settings → Automatic shutter → kohdalab-shutter** を選びます。
5. CH0は`0`と入力します。shutter GUIの表示順を入れ替えても、物理CH番号は変わりません。

endpointは通常空欄です。`~/.kohdalab/shutter/run/shutter-*.json`を優先して探索し、稼働GUIが1つだけの場合に接続します。Windowsではユーザープロファイル下です。複数GUIがある場合は、対象の接続情報ファイルを明示します。ポートやtokenを手入力する必要はありません。tokenは測定データに保存しません。

実機設定例は[ple-shutter-settings.json](../../examples/ple-shutter-settings.json)です。LightField、CONEX、パワーメーターの接続先は実値へ置き換えます。

```json
{
  "mode": "automatic",
  "backend": "kohdalab_shutter",
  "endpoint": "",
  "channel": 0,
  "timeout_s": 10.0
}
```

これは設定JSONの`background`項目です。旧プロファイルで`backend`を省略した場合は従来のMFF方式を維持します。MFF directも引き続き利用でき、その場合だけserial・open/closed状態番号とpylablibが必要です。

## BG取得の順序

1. shutter GUIの排他的操作権を取得します。
2. 対象CHにCloseを指定し、ジョブ完了と、移動・整定後の`STATE=Close`を待ちます。
3. LightFieldでBGを取得し、生SPE/CSVを保存します。
4. Openを指定し、ジョブ完了と整定後の`STATE=Open`を待ちます。
5. 初回PLを取得します。ユーザー／外部APIがpeak・積分区間を決め、残りのPLEへ進みます。
6. 正常終了、Stop、例外の後は明示的にCloseを要求します。閉状態を確認して操作権を解放し、shutter GUIの通常操作へ戻します。未確認の場合は制御ハンドルを保持して復旧を要求します。

状態と操作結果は`metadata/shutter.json`、`events.jsonl`、GUI/API状態へ記録します。BG差分処理・初回PLの保持・生データ保存は手動BGと共通です。

`STATE`はPicoの指令位置・移動待ち・整定情報です。**光センサーによる遮光確認ではありません。** 現地で遮光状態と光路を確認します。記録にも`physical_optical_sensor=false`を明示します。実機PLEはDemoのshutter GUIを拒否し、PLE自身のDemoは実機shutterへ接続しません。

## 停止・切断

操作権はheartbeatで維持します。通信断・操作権喪失時はPLEを停止し、閉鎖を試みます。LightFieldの取得中に操作権を失った場合もStopを要求します。

応答を失ったOpenを新しいIDで再送しません。対象UUIDの結果と整定状態を解決してから次の操作へ進めるため、遅れて到着するOpenをClose完了と取り違えません。解決できなければ遮光完了を主張せず、次の測定を禁止します。

kohdalab-shutter自体は、切断・lease失効時に最後の要求状態を保持する仕様です。通信や操作権が失われた後のCloseは成功を保証できません。GUIの未確認状態を確認し、必要ならshutter GUIや現地の操作で遮光します。自動再接続や他の操作者からの操作権取得は行いません。

## 外部APIから設定

```python
from kohdalab_ple import GuiClient

with GuiClient() as ple:  # 開いているPLE GUIへ接続
    ple.set_background({
        "mode": "automatic", "backend": "kohdalab_shutter",
        "endpoint": "", "channel": 0, "timeout_s": 10.0,
    })
    print(ple.shutter_status())
    job = ple.start()
    # 初回PLでanalysis_selectionになったらpreview()を確認し、
    # select_analysis(...)で解析区間を確定してからwait(job["id"])。
```

設定変更はアイドル時のみです。shutter操作のための別Pythonクライアントを同時に取得せず、PLEエンジンへBG手順を任せます。

## 同梱版の出所

Windows Releaseには、公開upstreamの固定commit `7e8d1c953905ad92a49a07cd75b916c38a3a518d`（package 0.6.6）からビルドしたGUI／CLIを同梱します。`companions/kohdalab-shutter/companion-origin.json`、対応する`source.zip`、MIT本文、Cargo依存ライセンスを保持します。GUIの起動用スクリプトは`--gui-api`を指定するだけで、Picoへのfirmware転送は行いません。同梱版のGUI実行にRustやPythonは不要です。
