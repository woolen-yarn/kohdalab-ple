# KohdaLab PLE 実機確認記録（記入用）

[実機確認チェックリスト](../guides/hardware-qualification.md)に沿って、この雛形を試験ごとにコピーします。結果は **合格／不合格／未確認／対象外** のいずれかを記入します。未確認・対象外は理由も記録します。時刻にはタイムゾーンを含め、証跡欄にはファイルのパスまたはリンクを記入します。

## 試験構成

| 項目 | 記録 |
| --- | --- |
| 実施場所、日付、担当者、確認者 | |
| 取得PCのホスト名、Windows edition/build、UCRT | |
| CONEX制御PCのホスト名、OS、サーバー版・プロトコル、COM | |
| Releaseタグ、Setup／portable、exeの版とSHA-256、`windows-imports.txt` | |
| Pythonのパス・版・bitness、`pip freeze` | |
| LightField・Automation SDK・カメラfirmwareの版、experiment、grating | |
| Newport SDK・CONEX firmwareの版、軸・可動範囲 | |
| Thorlabs SDK／TLPM helper・ドライバーの版、VISA resource | |
| シャッターGUI・API・Pico firmwareの版、物理CHまたはMFF state番号 | |
| 設定JSONのパスとSHA-256、位置・波長校正とパワー補正係数の出所 | |

## 機器の識別

| 機器 | 型番 | シリアル番号 | 備考 |
| --- | --- | --- | --- |
| レーザー | | | |
| 分光器・カメラ | | | |
| CONEXコントローラー・ステージ | | | |
| パワーメーター・センサーヘッド | | | |
| シャッター・コントローラー | | | |

## 項目別結果

| 確認項目 | 担当者 | 時刻 | 結果 | 証跡・備考 |
| --- | --- | --- | --- | --- |
| Windows版の起動・配布物照合 | | | | |
| 非操作preflight | | | | preflight JSON |
| LightFieldの読戻し・取得・生SPE／CSV | | | | |
| LightFieldのStop／timeout | | | | |
| CONEXの接続時非移動・範囲・小移動・読戻し | | | | |
| CONEXのStop／timeout | | | | プロトコルと確認できた範囲 |
| パワーメーターの単位・読戻し・位置／波長校正 | | | | |
| 手動BGの遮光・復帰 | | | | |
| 自動BGのClose／Open・整定・終了後のCloseと解放 | | | | STATEと実光路を別記 |
| 自動BGのStop・通信断／lease喪失時の状態 | | | | 閉未確認も記録 |
| BG照合の必須読戻しと異常時の生データ保持 | | | | BG・PL取得別JSON |
| 短いPLEの完了と別runでの停止 | | | | 各runのパスと終了状態 |
| 保存セットの4ファイルと `verify_run.py` | | | | JSON、終了コード、確認済み取得数 |

総合結果、未確認事項・対処、確認者と確認日時を記入します。失敗・中断runも保存し、再試験の記録から参照します。
