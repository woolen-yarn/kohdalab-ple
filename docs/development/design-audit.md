# Rust実装・外部API・機能監査

監査日：2026-10-29。対象：PLE 0.2.0と、公開されている以下のmain HEAD。

- [TRKR 066f8968](https://github.com/woolen-yarn/kohdalab-trkr/tree/066f89686efc1155156b0d079fcb1766d897d0f4)
- [IV 2908b47d](https://github.com/woolen-yarn/kohdalab-iv/tree/2908b47d54a0dc35927f508e51345bb568ec83df)

これは全実機動作の保証ではなく、ソース照合、独立レビュー、自動試験、Demo GUIの実行に基づく記録です。

## 監査前と更新後

監査前のPLEには機器通信用のJSON RPCとリモートCONEXがありましたが、GUIをPython/Jupyterから操作する外部APIはありませんでした。両者を区別して確認しました。

| 項目 | IV / TRKRで確認した設計 | 更新後のPLE |
|---|---|---|
| 機器接続の所有 | GUI／Rustエンジンが所有 | 同じ。Pythonクライアントは機器を直接開かない |
| GUI外部API | opt-inのloopback TCP | `--gui-api`で有効化するPLE API v1 |
| 発見・認証 | private descriptor、random token | 専用descriptor、UUIDv4 token、Unix 0600、正常終了で削除 |
| 実行場所 | GUIで要求を検証、既存エンジンへ送る | 同じ。ネットワークスレッドは機器I/Oをしない |
| 所有権 | lease、heartbeat、GUI Take control | 排他的lease、10秒の失効、GUI Stop／Take control |
| 重複実行防止 | UUIDとpayloadの照合 | start/connect/disconnect/moveにjob receipt。設定・BG確認・解析選択にも再送用ACK |
| 応答喪失 | IDを保持、Stop policy | 自動再送しない。既定でStopを要求し、例外にもIDを保持 |
| 長時間処理 | receiptのID別result/wait | job単位のrunning/completed/stopped/error。ACKにはrun_jobを関連付ける |
| 停止確認 | IVはOutput OFF、TRKRはmotion cleanup | PLEはワーカーとStop応答、自動MFF閉鎖読戻し。手動遮光はユーザー操作 |
| 履歴 | 最後の32 receipt | 32 job + 32 command ACK。退役IDも同セッション中は再実行しない |
| 外部ジョブと生データ | 保存データによる追跡 | `automation.json`とeventsでAPI要求IDとrun IDを関連付ける |
| Python配布 | 専用Python client | 標準ライブラリだけを使う`GuiPLE`、pip用設定と検証スクリプト |

比較元：[TRKR transport](https://github.com/woolen-yarn/kohdalab-trkr/blob/066f89686efc1155156b0d079fcb1766d897d0f4/crates/trkr-desktop/src/remote.rs)、[IV GUI API](https://github.com/woolen-yarn/kohdalab-iv/blob/2908b47d54a0dc35927f508e51345bb568ec83df/docs/reference/gui-api.md)。IV/TRKRと操作思想・用語を揃えていますが、PLEのメソッド・戻り値を持つ別APIであり、両製品のクライアントの差し替え互換は主張しません。

## RustとSDKアダプターの境界

Rustが担当する処理：

- GUIとライブ監視、走査計画、0–25 mm制限、測定状態の調整、キャンセル。
- BG適合性検証・差し引き、励起光ピークの検出、PLピーク追跡、独立した区間の積分。
- エネルギー換算、パワー正規化、代表スペクトル選択、回帰、Heatmapの集約と描画。
- raw／processed CSV、設定・取得JSON、statusとイベントの保存・再読込。
- 外部GUI API、認証、lease、要求ID、receipt、リモートCONEXクライアント。

Pythonを残す処理：

- LightFieldの.NET Automation、取得完了・SPEエクスポート待ち、ベンダーCSVの単一ROI／frame読取り。
- 元zipのThorlabsドライバーとMFFへの接続。
- WindowsのNewport .NET SDKを使うCONEXリモートサーバー。

この境界は既存SDK・機器資産を引き継ぐためのものです。Python側へ走査計画や解析を重ねて実装していません。現状の計測ボトルネックを測らずにSDK接続を全面Rustへ置換することは行っていません。LightFieldのCSV解釈や各SDKのネイティブアダプターをさらにRustへ移す場合、まず実機のAPI・ファイル形式を固定して同じデータで比較する必要があります。

## 今回の最適化

1. スペクトル・代表8曲線・位置対励起波長・回帰の表示配列をキャッシュし、描画では借用sliceを使う。取得元、選択データ、nm/eV、パワー正規化が変わると更新する。
2. 長い表示用スペクトルだけを最大8,192頂点へ間引く。各bucketの最小・最大と両端を元座標の順序で残す。小さなスペクトルは全点を描く。生データ・解析・積分・CSVは全ビンを使う。
3. Infernoの256色LUTをコンパイル時に復号し、画素ごとの16進変換を除去。全色一致を試験する。
4. 位置から励起波長への区分線形補間を二分探索へ変更。校正点の順序・重複・外挿拒否は維持。
5. APIの大きなスペクトルを`result`と別名`data`へ二重複製しない。標準の`ok/result`だけを返す。

再現用：`cargo run --release -p ple-core --example plot_preparation_bench`。

Apple silicon / releaseでのCPU準備処理の例。旧処理は毎frameの正規化配列・plot配列の生成、新処理は最初のキャッシュ構築と以後のslice参照を含む。GPU描画、全GUIのframe時間、取得速度の測定ではない。

| detector bins | 曲線数 | frame相当 | 旧準備 ms | キャッシュ版 ms | 1曲線の表示頂点 |
|---:|---:|---:|---:|---:|---:|
| 4,096 | 8 | 120 | 2.830 | 0.062 | 4,096 |
| 65,536 | 8 | 60 | 20.795 | 1.007 | 8,192 |
| 1,000,000 | 8 | 10 | 84.151 | 20.371 | 8,192 |

数値は入力・機器・ビルド環境で変わる。間引きは各bucketの極値保持であり、全ての局所ピークを保持する保証ではない。詳細解析には全ビンのSPE/CSV/APIを使う。

## 発見して修正した不整合

- GUI用APIがなかったため、機器用RPCとは別に明示的なGUI API層を追加。
- シャッター状態をRust/UIだけが0/1に固定していた。異なる0–255の機器状態を指定できるよう統一し、7/3の試験を追加。
- CONEX移動失敗で接続を無効化する際も、保持したendpointからStopを要求して完了を待てるようにした。
- LightField Stopの非同期要求が次のconfigureより遅れて届き得た。Stop待ちのbarrierと未確認エラーのラッチを追加。CONEXの既存barrierと合わせ、未確認Stopを無視して次の移動・取得へ進まない。
- LightFieldの切断失敗をDropだけで捨て得た。明示的なdisconnect結果を扱い、MFF閉鎖確認・SDK Dispose失敗を成功扱いしない。
- 大きさの制限がないbridge stdoutの行読取りを64 MiB以内へ制限し、不完全なJSON行を拒否。
- `--config`で指定した設定ファイルをConfig欄・Saveの保存先にも反映し、別の既定ファイルへ保存しないようにした。
- 不正な明示プロファイルが既定の実験設定へフォールバックし得た。起動前に検証し、その検証済み値だけを使う。
- Pythonの変更要求の応答喪失、BG確認・解析選択の再送についてレビューを受け、Stopと要求ID・ACKの照会経路を追加。

raw CSVを再書込みしないこと、BGと測定の設定・軸の一致、最初のPLの再利用、停止時のpreview保持、位置・パワー入りの小数ファイル名も再確認した。

## v0.2での仕上げ

- 取得直後にraw CSVと取得時メタデータを確定し、BG・設定照合・解析が失敗しても保持する。
- LightField Stop受付後にIsReadyToRunと自動シャッター閉鎖を確認する。未確認は次の測定を禁止する。
- 接続中の分光器ApplyをGUI/Python APIで統一し、実値を返す。
- パワーのW/dBm単位・波長・平均時間の読戻しを検証する。センサーの通常の波長丸めは設定した許容差内で受け入れ、実際の値を記録する。

## 残っている範囲と導入優先順位

### 実機導入前に確認する項目

- Windows版exeのクロスビルド・配布は作成済み。Windows上の起動と実機LightField・MFF・Newport・Thorlabsでの正常取得、停止、タイムアウト、通信断を[実機確認手順](../guides/hardware-qualification.md)で確認する。
- LightFieldのexperiment、ROI・readout・ROI・readoutなど。読戻し照合は露光・合成frames・中心波長・grating・ベンダーBG補正無効と波長配列で、全SDK設定を網羅しない。
- パワーセンサーの校正・光路係数。センサー波長は固定または事前の位置校正に追従する。設定読戻しと取得後の励起波長の許容差を確認する。ファイル名のパワーは露光直前の読取りに係数を掛けた値で、露光中の時間平均ではない。
- CONEXのSDK署名・ready state・可動範囲・原点。APIを付けても未検証の実機特性は解決しない。

### 明示的に未提供の機能

- GUIなしのheadless実行、IVのNativeIV相当のC ABI、HTTP/REST、WebSocket。
- Remote JSONのHome／自動初期化、TRKRの複数motion modelのcapability層。接続は読取りのみで、必要な初期化はベンダー側で事前に行う。旧TCPのCHECKも明示許可した場合に限る。
- APIの`load_config(path)`。Python側でJSONを読み`configure(config)`、または`launch(profile=...)`を使う。
- 中断測定の自動再開、receiptの再起動越し永続化、任意の多次元レシピ。
- 元zipの偏光回転台、XY mapping、撮影、ELN連携などPLE以外の機能。
- 複数ROI／複数frameの任意合成。現状は未対応形状を拒否する。
- 宇宙線・重なった励起ピークの高度な判別、PLピークのサブピクセルfit。
- 全SDKエラーの細分類、全製品共通のAPI Rust crateへの切出し。
- 大規模Heatmap向けの増分集約。texture atlasはGPUの最大texture寸法に合わせてページ化済み。現在の全行集約の処理はCPU準備比較には含めていない。

これらを「対応済み」に含めない。対応機能は`capabilities()`と[API仕様](../reference/gui-api.md)、測定・保存の意味は[測定仕様](../reference/measurement.md)で確認できる。
