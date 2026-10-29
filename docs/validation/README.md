# v0.2.3 実機検証用プレビュー — 2026-10-29

自動機器識別を含む版です。[Release v0.2.3](https://github.com/woolen-yarn/kohdalab-ple/releases/tag/v0.2.3)の`VALIDATION.json`に今回のソフト検証、`BUILD-ORIGIN.json`にソースcommitとZIP／実行ファイルのSHA-256を記録します。WindowsではZIPを展開した実行ファイルの版番号・設定検査を実施し、PE依存関係と静的CRTを確認します。実機SDK接続と実際の機器は現地の[実機確認](../guides/hardware-qualification.md)で検証します。

以下は過去の各時点での検証履歴です。旧JSONと旧件数をv0.2.3の結果として読み替えません。

---

# v0.2.1 追加検証 — 2026-10-29

- Rust：75件成功（desktop 55、core 20）。SDKの失敗／タイムアウトからStop→idle確認への順序、確認失敗時の次操作拒否、失敗接続の制御ハンドル保持、再接続前の旧接続終了を回帰試験に追加。
- Python：69件成功。保存データ検査の異常系と、実PowerShellでの5試験を含む。`PLE_PWSH` に一時展開した公式PowerShell 7.6.9のパスを指定して実行。
- PowerShellはmacOS上で構文解析、preflight成功・失敗、空白を含むパス、JSONの単一出力、配布物の2回連続作成・古いファイル除去・失敗時の前版保持を実行検証した。外部ビルド・SDKコマンドは試験用に置換しており、Windows実行の検証とは区別する。
- 実GUI API一連試験を更新版で再実行。値なし・未知の起動オプションがGUI起動前に失敗すること、`--version`、測定・保存・lease失効停止を確認。
- 保存検査は完了した6取得の新しいAPI Demo runでも成功。`completed` の点数不一致、CSVとJSONの取得取り違え、時刻逆転、空SPE、破損BG、Windowsのdrive相対パスと点番号の別表記による重複を拒否。
- クロスパッケージ作成は既存のインストール済みtoolchain/targetを使ってオフラインで実行。実GUI API検証スクリプトも配布物へ同梱。

Windows上での起動、Windows固有のPowerShell・ファイル操作・SDK・実機の動作は未検証。Windowsで実行するCIの定義を更新したが、クラウドへは送信・実行していない。

現行の版・件数・ハッシュは [release-readiness.json](release-readiness.json)。以下は前版の履歴。

---

# v0.2.0 検証記録 — 2026-10-29

実機確認へ渡すリリース候補。Windows x64用exeのクロスビルドとmacOS Apple silicon版の実行を確認した。Windows実行、PowerShellラッパーの実行、メーカーSDK・機器への接続はこの環境では検証していない。

- Rust：70件成功（desktop 50、core 20）。Clippy警告なし、fmt成功。
- Python：ブリッジ・リモートCONEX・SDK環境検査・GUIクライアント・ライセンス収集・保存データ検査を自動試験。最終件数は `validation/release-readiness.json` に記録。
- 実GUIのDemo API一連試験：接続、分光器の接続中Applyと読戻し、0/25 mm移動、範囲外拒否、BG/信号確認、初回PLを保持して解析選択、6スペクトル×641ビン保存、同一ID再送、実時間10秒のlease失効停止、切断まで成功。
- 波長追従：パワー読取り前に位置校正からセンサー波長を設定。保存された実値・時刻・検出励起波長の許容差を照合。異常時も取得済みraw CSVと取得記録が残る回帰試験を含む。
- LightField偽SDK：IsReadyToRun待ち、Stop完了確認と失敗、ベンダーBG補正無効と外部変更検出を確認。SDK署名の実機互換性は未確認。
- W/dBm変換、波長・平均時間の読戻し、通常の波長丸めと物理範囲外の拒否を確認。負の暗時パワーは監視に表示でき、非正値での測定開始は拒否。
- 通常nm・eV/Counts/W画面を最終バイナリで起動し、4グラフ・Inferno atlas・ライブモニターを確認。Demo 22スペクトル＋BGを保存。
- 読取り専用 `verify_run.py` で最終API測定のraw/BG/processed/JSON、eV・正規化の整合を検査。DemoにはSPEを作らない。
- Windows exeはPE形式とDLL importを静的検査。追加のlibgcc/libwinpthread/libstdc++ DLLに依存しない。これはWindowsでの起動成功を意味しない。
- 固定lockfileのライセンス本文と上流出典・ハッシュを配布へ添付。メーカーDLLは含めない。

[GUI API証跡](gui-api.json) · [実機確認・合否記録](../guides/hardware-qualification.md)

以下は以前のv0.1の検証履歴。当時の「未作成」「未対応」は上のv0.2記録と[設計監査](../development/design-audit.md)を優先する。

---

# v0.1 検証記録 — 2026-10-29

対象：KohdaLab PLE 0.1.0、macOS / Apple silicon。

- `cargo test --workspace --locked`：66件成功。計画・BG差分・eV換算・励起ピークの雑音排除・積分・Heatmap平均・JSON/CSV保存・開始直後停止・模擬一連測定・CONEXローカル模擬通信と接続断・明示パワー取得のライブ通知を含む。
- `cargo clippy --workspace --all-targets --locked -- -D warnings`：成功。
- `cargo fmt --all -- --check`：成功。
- `python3 -m unittest discover -s tests -v`：30件成功。SDKは偽物で、実機へ接続しない。
- リリースビルド：成功。ローカル実行用のApple silicon `.app` を作成し、アドホック署名。
- `.app` の模擬測定：11位置×2回、22スペクトル＋新規BGの保存完了を確認。
- 通常nm表示とeV＋パワー正規化表示を実際に起動し、両ライブ監視と4グラフの画面を確認。
- 保存済みCSVの全641ビンについて、`corrected = raw - BG` と `emission_eV * emission_nm = hc` の一致を確認。
- Demoで偽のSPEファイルを作らないことを確認。

画面：`images/ple-demo.png`、`images/ple-demo-energy.png`。自動デモでは実際の露光・安定待ち時間を短縮しており、表示されるハードウェア測定時間見積もりとは一致しない。

未検証：Windowsビルド、LightField実機API・カメラ・SPEエクスポート、CONEX実機サーバー、Thorlabs SDK・パワー校正。GitHub ActionsのWindowsビルド定義は作成したが、この作業でクラウド実行していない。

既存zipのCONEXサーバーにはSTOPがない。移動中の停止は次点以降の中止であり、移動の即時停止や移動中の位置ストリーミングを提供しない。Remote JSON方式は付属サーバーで移動中の監視・Stopに対応し、MFF自動遮光または手動確認を選べる。実機では未検証。

## TRKR / IV GUI統一

TRKRの `066f89686efc1155156b0d079fcb1766d897d0f4`、IVの `2908b47d54a0dc35927f508e51345bb568ec83df` を参照。フォームの26px入力、12px文字、左右384px／300pxの固定列、中央の可変幅、背景・枠・カード・Start／Stopの色を共通化した。WindowsはSegoe UI／Yu Gothic UIの同じ選択処理を使う。

ライブモニターの数値・バー・塗りつぶしトレンド、30秒の相対時間窓、SIM／MEASUREDとWAITING／LIVE／STALE／HELD、接続・取得元の履歴分離を検証した。画面内に4プロットが収まること、Device開閉で中央が動かないこと、測定中もStopが押せること、フォーカス中の数値入力でもロック中は変更できないことをUIテストで確認した。Heatmapは強度向けInfernoに変更し、公式LUTの端点・負値のスケール保持を検証した。

共通UIコードとライセンスの来歴は `third_party/NOTICE.md` に記録。この更新ではBG→最初のPL→解析確認→残り取得の流れと、解析確定設定・物理名付き出力を追加した。

## PLE操作の更新

- 手動BG確認と自動シャッターの両フロー、解析確認までの停止、無効な解析区間の修正、最初のPLの再利用、中断時のpreview保持を模擬機器で確認。
- Remote JSONの接続時の非移動・非原点復帰、移動中のstatus/stop、応答ID・有限値・機器エラー・範囲チェックをローカル模擬サーバーで検証。
- raw CSVは一時ファイルを保存してから確定し、previewのraw CSVは解析確定時に再書込みしない。
- 位置・試料上推定パワー入りの小数ファイル名、point/repeat衝突回避、波長回帰のR²（一定波長では未定義）、位置ごとの平均、8本の代表スペクトルを検証。
- `images/ple-review.png` は実際の確認待ち画面。確認中はピーク・積分区間だけを編集でき、走査・取得設定はロックされる。

追加レビューで見つかった遅延Stop・raw CSV再書込み・シャッター切断前確認を修正。遅延Stop後の再接続待ちと、旧世代Stop失敗時の復帰禁止を回帰テストで確認。

CONEXの0–25 mmの固定可動範囲、分光器・パワーメーター未接続時の手動移動、測定中の手動操作ロックを検証。

## 外部API・Rust最適化の監査

- Rust 66件、Python 30件、Clippyの警告なし、fmtを確認。
- 実GUIを1つ起動し、GuiPLEから接続、0/25 mm移動、手動BG・光路確認、preview解析選択、6取得×641ビン保存、同じUUIDの再送、実時間10秒のheartbeat失効停止、切断・lease解放まで確認。
- 外部APIの要求IDとrun IDをautomation.jsonに保存し、raw/processed CSV・BG・受理済み設定との対応を検証。
- 個別レビューで、応答喪失時Stopと確認要求のID管理を確認し修正した。
- 描画準備のキャッシュ・全色LUT一致・表示用bucket極値保持・エネルギー座標順序を確認。

保存した結果：[GUI API検証](gui-api.json)、[CPU準備処理の比較](plot-preparation.csv)。比較方法・未対応範囲は[設計監査](../development/design-audit.md)。

新しい機能もWindows実機検証を意味しない。LightFieldの全experiment設定、停止完了、ベンダーSDK署名、パワー校正は実機での確認が必要。
