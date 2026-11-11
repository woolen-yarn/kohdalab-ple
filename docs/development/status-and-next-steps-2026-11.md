# 現状・問題点・次の作業の提案（2026-11-11）

[文書一覧](../README.md) · [設計監査（0.2.0時点）](design-audit.md) · [配布手順](releasing.md)

対象は `origin/main`（aa55f35、v0.3.11）。ソース・文書・CI・GitHub上のRelease履歴を読み直し、このMacで実機なしの検証を実行した結果です。実機・Windows実行・GUI画面は今回確認していません。コード上の指摘は「ソースを追って確認した」もので、再現試験はしていません。

> **その後の対応（2026-11-11追記）**：4節のAは [PR #12](https://github.com/woolen-yarn/kohdalab-ple/pull/12)、Bは [PR #13](https://github.com/woolen-yarn/kohdalab-ple/pull/13) で実装済みです。以下の本文は調査時点（aa55f35）の記録で、3.1と3.3の指摘はこの2本で解消しています。C〜Fと5節の未決事項は未着手です。4節Fの「最小化中にAPIとlease失効の処理が止まるか」は、0.3.10で制御・API処理を描画から分離済みとRelease本文にあり、確認だけが残っています。

## 1. 要点

- ソフトウェアとしては健全です。CIは緑、ローカルでもRust 124件が成功し、Pythonは116件中109件が成功・7件がskipでした。ソケット・パイプの読み取り上限、タイムアウト、生データの上書き防止は一貫しています。
- 最大の未完了は**実機確認が一度も記録されていないこと**です。0.2.2以降の公開は13日間で10回、すべて「実機確認用Preview」ですが、合否記録はrepoにありません。
- 測定PCで最初に読む文書が古いままです。`START-HERE.md` は v0.3.8、操作ガイドは v0.2.1、実機確認チェックリストは v0.2 を名乗っています。
- 実機で踏みそうなコード上の問題が4件あります（3.1〜3.4）。実機確認の前に直す価値があります。
- 公開手順が文書と実態で食い違っています。0.3.6・0.3.7・0.3.11はタグCIを中断し、PRブランチの手動ビルド成果物を公開しています。

推奨する順番は、**A（実機前の修正）→ B（入口文書の更新と版ずれの自動検査）→ 実機確認 → C以降**です。

## 2. 現状

### 2.1 構成

| 場所 | 役割 | 規模 |
| --- | --- | --- |
| `crates/ple-core` | 設定と検証（`Config::plan`）、BG差引・解析、run保存（`recording.rs`）、表示用の間引き | 約2,100行、テスト23件 |
| `crates/ple-desktop` | egui GUI、測定の進行（`engine.rs`）、機器クライアント（`backend.rs`・`remote_stage.rs`・`shutter_client.rs`）、ローカルGUI API | 約12,000行、テスト91件 |
| `crates/kohdalab-control-protocol` | 5アプリ共通の制御プロトコル1とパス解決 | テスト10件 |
| `bridge/` | LightField・Thorlabs用のPython bridge、CONEX参照サーバー | 約1,600行 |
| `bindings/` | PLE用Pythonクライアントと共通制御SDK | 約900行 |
| `scripts/`・`tests/` | 配布物作成・インストーラー検証・保存run検証と、その実機なしテスト | Pythonテスト100件＋共通SDK 16件 |

スレッドは、UI（機器I/Oなし）、測定を進める調整スレッド1本、ステージとパワーメーターの各アクター、bridgeごとの読み取り2本、APIリスナーです。run保存は `recording.rs` だけが行います。生CSVは解析より先に一時ファイル＋renameで確定し、既存ファイルは置き換えません。

### 2.2 今回の検証結果

| 検証 | 結果 |
| --- | --- |
| `cargo test --workspace --locked`（rustc 1.99.1） | 124件成功（protocol 10、desktop 91、core 23） |
| `python3 -m unittest discover -s tests`（Python 3.14.6） | 100件中93件成功、7件skip（PowerShellがこのMacにないため。CIのWindows・macOSでは実行されている） |
| `python3 -m unittest discover -s bindings/control -p "test_*.py"` | 16件成功 |
| `python3 scripts/check_docs.py` | 成功（48文書、ローカルリンク133件） |
| `cargo fmt --check`・`cargo clippy` | **未実行**。作業用toolchainにrustfmt・clippyが入っていない。CIでは成功している |
| GitHub Actions（main、2026-11-10） | Test・Documentationとも成功 |

Rustは `agent-kit/scripts/with-toolchain.sh` 経由で実行できます。`CARGO_TARGET_DIR` をrepo外に向けたので作業ツリーは汚れていません。

### 2.3 公開の状態

- repoは公開、Releaseは10件すべてpre-release。タグは14個で、v0.2.2・v0.3.0・v0.3.1・v0.3.3はReleaseなし（CHANGELOGに経緯あり）。
- Issueは0件、PRは10件すべてマージ済み。
- v0.3.11の `VALIDATION.json` は `physical_hardware_tested: false`、`native_gui_window_tested: false`。

## 3. 問題点

### 3.1 実機で踏みそうなコード上の問題

**(1) シャッターの閉確認失敗が、別の成功通知で消える。**
`api_cleanup_error` は1個の枠で、`Event::Cleanup{confirmed:true}` が届くと無条件に消えます（[main.rs:435](../../crates/ple-desktop/src/main.rs)）。自動BG（MFF）で測定終了時の閉操作が失敗すると、engineが「未確認」を送ります（[engine.rs:557](../../crates/ple-desktop/src/engine.rs)）。同じ失敗がbridgeのStopも起動し、LightFieldがidleになるとそちらが「確認済み」を送ります（[backend.rs:184](../../crates/ple-desktop/src/backend.rs)、同242行）。後から届いた方が勝つので、シャッターが閉じたか不明のままStartが有効になります。
保険の分岐も効いていません。`Event::Finished` は「shutter closed state could not be confirmed」を探しますが（main.rs:462）、engineが出す文言は「shutter cleanup could not be confirmed」です（engine.rs:567）。「Retry Stop / verify idle」もLightFieldのidleだけを確かめてシャッターの未確認を消します。

**(2) 校正範囲の端で、ステージの読み戻しが1 µmずれるとrunが失敗する。**
`Config::plan` は指令位置が校正点の範囲内かを確かめますが、取得時と解析時は実位置で補間し、範囲外の許容は1e-9 mmです（[lib.rs:531](../../crates/ple-core/src/lib.rs)）。一方ステージの位置許容差は既定1e-3 mmで、既定設定は走査開始点と最初の校正点が同じ18.0 mmです。読み戻しが17.9997 mmなら「lies outside excitation calibration」で止まります。Demoは指令値どおりに返すので、実機でだけ起きます。

**(3) bridgeのstderrにUTF-8以外が1行出ると、以後のログが読めなくなる。**
[backend.rs:126](../../crates/ple-desktop/src/backend.rs) は `lines().map_while(Result::ok)` で読んでいて、UTF-8として読めない行で読み取りスレッドが終わります。bridgeはSDKの出力をstderrへ流しますが、Python側の出力エンコーディングを指定していません。日本語WindowsでSDKやOSのメッセージがcp932で出ると、パイプの読み手がいなくなり、以後のbridge側の書き込みが失敗し得ます。発生条件は未確認です。

**(4) 位置監視が1回失敗するとリモートステージを切断し、runが落ちる。**
ステージのアクターは400 msごとに位置を読み、1回でも失敗（5秒タイムアウトなど）すると接続を捨てます（[backend.rs:597–619](../../crates/ple-desktop/src/backend.rs)）。次の移動は「CONEX not connected」になります。再試行はありません。制御PCとの間が研究室LANなので、数時間の走査では現実的に起きます。

### 3.2 その他のコード上の問題

| # | 内容 | 場所 |
| --- | --- | --- |
| 5 | `atomic_json` は置換時に「旧ファイルを退避→新ファイルをrename」の2段で、その間は対象が存在しない。`status.json` は取得ごとに書き換えるので、外部から読むと欠けて見えることがある。コメントの前提（Windowsのrenameは置換できない）は現在の `std::fs::rename` には当てはまらない。runフォルダーに `*.previous.json` も残る | `recording.rs:38–47` |
| 6 | `Recording::sample` は processed CSV → metadata → `analysis.csv` → `points.csv` の順に書き、途中で失敗すると不揃いのまま残る。それでも `finish` は `hash_verification.status = "verified"` と書く。ハッシュの対象は3ファイルだけで、`points.csv`・`raw/`・`processed/`・`metadata/` は含まない | `recording.rs:326–420`、`442–491` |
| 7 | GUI APIの `stop` はleaseなしで通るが、capabilitiesは `stop_requires_lease: true` と返す。安全側の挙動だが、宣言と実装が逆 | `api_ui.rs:191`、`289–297` |
| 8 | 自動BGでシャッターGUIに接続できなかった場合も、終了処理で閉操作を試みて「cleanup could not be confirmed」になり、設定ミスが機器復帰ダイアログとして出る | `engine.rs:551–567` |
| 9 | BG照合に必要なLightField設定が読めない場合、最初のPL取得後に初めて判定され、文言は「settings changed after BG」。BG取得の時点では分からない | `engine.rs:805–880` |
| 10 | descriptorの0600はUnixだけ。実機で使うWindowsではACLを設定せず、ユーザープロファイル配下であることに依存している。文書はこの差を書いていない | `remote_gui.rs:81–86` |
| 11 | 終了時にUIスレッドが調整スレッドの終了を待つ。実機接続中は最長でStop 30秒＋shutdown 25秒＋パワー21秒、ウィンドウが固まる | `engine.rs:482–490` |
| 12 | CONEX参照サーバーは、ループバック以外にbindしてもtokenなしで起動できる。別PCから使う構成では、LAN上の誰でも移動指令を送れる | `bridge/conex_server.py:257–262` |

Demoが実機やシャッターGUIに接続しないこと、W/dBmの換算をネイティブ値と突き合わせていること、生CSVを解析前に確定して書き換えないことは、ソース上で成り立っています。

### 3.3 文書の版ずれ

| 文書 | 現状 | 影響 |
| --- | --- | --- |
| `START-HERE.md` | 表題・ファイル名がv0.3.8 | Windows配布物に同梱され、`VALIDATION.json` が「次に読む文書」として指している |
| `docs/guides/user-guide.md` | 「v0.2.1：リリース候補」。起動方法が `dist/` の作業フォルダー前提 | Setup.exe・`~/.kohdalab/ple`・自動BG・4ファイルbundleが載っていない |
| `docs/guides/hardware-qualification.md` | 表題が「PLE v0.2」 | 0.3で増えた自動BG（kohdalab-shutter）、BG照合の必須設定、bundleの確認項目がない |
| `docs/validation/README.md` | 先頭がv0.3.0。0.3.5〜0.3.11の記録なし | READMEの「検証記録」リンク先が8版前 |
| `docs/validation/release-readiness.json` | v0.2.1 | 文書一覧が「配布検証のJSON」として現行扱いで案内している |
| `CHANGELOG.md` | 0.3.9・0.3.10の項目なし。「Unreleased」が0.3.5と0.3.4の間にある | 2版分の変更が追えない |
| `bindings/python/pyproject.toml` | 0.3.8 | 0.3.11の配布物に0.3.8のクライアントが入る |
| `docs/guides/install.md` | 例と自動BGのリンクが0.3.4 | 古いReleaseへ誘導する |
| `docs/development/design-audit.md` | GUI APIは「`--gui-api` で有効化」 | 0.3.0から既定で有効。現行は `reference/gui-api.md` が正しい |

`check_docs.py` はリンク切れだけを見るので、これらは検査を通ります。

### 3.4 公開手順のずれ

[releasing.md](releasing.md) は「タグのビルドだけがprereleaseを公開する」と書いています。実際は次のとおりです。

- v0.3.6・v0.3.7・v0.3.11は、タグpushで走った `release.yml` を1分ほどで中断し、PRブランチの `workflow_dispatch` 成果物を手で公開している。
- v0.3.11の `BUILD-ORIGIN.json`・`VALIDATION.json` の `commit` はPRの先頭（0e21426）で、タグ（aa55f35）ではない。差を埋める `RELEASE-VERIFICATION.json`（treeが同一であることの記録）は、このrepoにないツールで作られている。
- `release.yml` の `publish` jobは残っているので、中断し忘れると `gh release create` が既存Releaseと衝突する。

成果物そのものの検証は丁寧ですが、手順がrepoから再現できず、人が中断する操作に依存しています。

### 3.5 CIと保守

- `dtolnay/rust-toolchain@stable` は追従型。desktopは `rust-version = "1.92"` を宣言しているが、その版でのビルドはどこでも確かめていない。新しいstableのclippy lintで、変更していないコードのCIやリリースが止まり得る。
- 依存の更新通知（Dependabot等）と `cargo audit`／`cargo deny` がない。
- 作業用Macのtoolchainにrustfmt・clippyがなく、push前に手元で確認できない。
- 測定の進行と保存のテストが、リスクに対して薄い。`recording.rs` は763行に3件、`engine.rs` は約1,400行に8件。
- 100行を超える関数が10個（`device_card` 約430行、`api_dispatch` 約390行、`Controller::spawn` 約340行など）。上限付きJSON行の送受信が5か所、Stop待ちの仕組みが3か所に別々に実装されている。run結果の分類はエラー文言の一致に頼っている（3.1(1)の原因）。
- TRKR由来の残り：`TRKR_DIALOG_*` 環境変数名と未使用分岐（`file_dialog.rs`）、`#[allow(dead_code)]` 2件（`precision.rs`）。

### 3.6 方針として未決のこと

- ローカル `main` の指示書commit（7246861、`.agents/`・AGENTS.md・CLAUDE.md）を公開repoへ出すか。
- CONEX CCサーバーの提供待ち（ROADMAPの1番目）。届くまで参照アダプター `bridge/conex_server.py` で実機確認を進めるか。
- `ROADMAP.md` は3行で、Previewを外す条件が書かれていない。

## 4. 次にやるとよいこと

AとBはこのrepoだけで決められます。Dは5アプリ共通なので、統括repo（`kohdalab-github`）で先に決めます。各項目は別ブランチ・別PRにし、`origin/main` から切ります。測定の記録仕様（列・単位・ファイル構成）は変えません。

### A. 実機確認の前に直す（Codex、1 PR）

| 作業 | 受入条件 |
| --- | --- |
| A1. 復帰ラッチを原因別に持つ（分光器Stop／シャッター／ステージ）。確認済み通知は同じ原因の未確認だけを消す。`Event::Finished` の文言一致をやめ、engineから型で渡す | 「シャッター閉が未確認→LightField idle確認済み」の順でイベントを流してもStartが無効のまま、というテストが通る。「Retry Stop / verify idle」後もシャッター未確認が残る |
| A2. 実位置での補間に、ステージの位置許容差ぶんの余裕を認める。範囲外へ出た分は端の校正点の値に丸めず、端の区間の傾きで求める | 既定設定で読み戻しが 18.0 − 0.0005 mm のときrunが続く。許容差を超える読み戻しは今までどおり拒否する。指令位置の検証（`plan`）は変えない |
| A3. bridgeのstderrをバイト列で読み、UTF-8でない部分は置換して表示する。1行の長さに上限を設ける。bridge起動時に `PYTHONIOENCODING=utf-8` を渡す | cp932のバイト列を含む行の後も、後続の行がログに出るテストが通る |
| A4. 位置監視の失敗は、連続N回（例：3回）で切断とする。移動中の失敗は移動側の結果を優先する | 監視が1回タイムアウトしても次の `move_to` が成功するテストが通る。連続失敗では今までどおり切断を表示する |
| A5. 自動BGでシャッターに一度も接続できていない場合は、終了時の閉操作を試みず、元の接続エラーをそのまま返す | シャッターGUI未起動で開始したとき、エラーが接続失敗の文言で、復帰ダイアログが出ない |

**A2の方針は決定済み（2026-11-11、ユーザー承認）：許容差内で外挿する。** 実装の範囲は次のとおりです。

- 対象は実位置で補間する2か所だけ：`Acquisition::analyze`（`ple-core/src/lib.rs:652`）と、パワーセンサー波長の追従（`ple-desktop/src/engine.rs:770`）。同1129行のテストは期待値の計算を同じ関数に合わせる。指令位置を検証する `Config::plan`（`lib.rs:512`）は、従来どおり余裕なしの `interpolate_excitation` を使う。
- 余裕は設定済みのステージ位置許容差（`tolerance_mm`、既定1e-3 mm）。新しい設定項目は足さない。
- 実位置が校正範囲の外かつ端から許容差以内なら、端の区間（最初の2点、または最後の2点）の直線を延ばして励起波長を求める。範囲内の計算は変えない。
- 端から許容差を超えた実位置は、従来と同じ文言で拒否する。
- 保存ファイルの列・項目は足さない。保存される実位置と校正点から、外挿だったことは後から判定できる。外挿した取得は `analysis.log` に1行残す。
- [measurement.md](../reference/measurement.md) の励起波長の節に、この扱いを1文で書く。

テスト：端ちょうど、端−許容差の半分（外挿値が端の区間の傾きと一致）、端−許容差の2倍（拒否）、上端側の同じ3件。

### B. 入口文書を現行版に合わせ、版ずれを検査で止める（Codex、1 PR）

- B1. `START-HERE.md`・`user-guide.md`・`hardware-qualification.md`・`install.md` を0.3.11の内容に更新する。版番号を本文に埋めず、どうしても必要な箇所だけにする。
- B2. `hardware-qualification.md` に、自動BG（kohdalab-shutter）、BG照合の必須設定の読み戻し、4ファイルbundleと `verify_run.py` の確認項目を足す。合否を書く記録用の雛形 `docs/validation/hardware-qualification-template.md` を置く（PC・SDK版・機器serial・項目ごとの担当者／時刻／結果／証跡）。
- B3. `CHANGELOG.md` に0.3.9・0.3.10を補い、「Unreleased」を先頭へ移す。内容はGitHub Releaseの本文とPR #6・#9から起こす。
- B4. `docs/validation/README.md` の先頭に0.3.5〜0.3.11の要約を足す。`release-readiness.json` は「0.2.1時点の履歴」と明記し、文書一覧の案内を各Releaseの `VALIDATION.json` へ向ける。
- B5. `bindings/python/pyproject.toml` の版をworkspaceに合わせる。
- B6. `scripts/check_docs.py` に版の整合検査を足す：`Cargo.toml` の版と、`START-HERE.md` の表題、`RELEASE-NOTES.md` の表題、READMEのDownloads表、`pyproject.toml`、CHANGELOG先頭の版付き見出しが一致すること。

受入条件：B6の検査が現状のmainでは失敗し、B1〜B5の後に通る。`check_docs.py` のテストに、ずれを検出する例を1件ずつ入れる。過去の検証記録の本文は書き換えない。

### C. 保存まわりの堅牢化（Claudeが設計を確定 → Codex）

- C1. `atomic_json` を一時ファイル＋renameの1段にする。`*.previous.json` は作らない。旧runに残っている分は読み込みで無視する。
- C2. `finish` が「verified」と書く前に、`points.csv` の行数と `metadata/` のサンプル数が一致することを確かめ、合わなければ `incomplete` と理由を書く。
- C3. bundleのハッシュ対象に `points.csv` と `raw/`・`processed/`・`metadata/` の一覧を加えるかを決める。`analysis.run.json` の項目追加になるので、[measurement-output.md](../reference/measurement-output.md) の変更として扱い、`verify_run.py` と旧runの互換を先に書く。
- C4. `recording.rs` のテストを足す：`sample` の途中失敗、`finish` の失敗、置換中の読み取り、`load_run` の欠損run。

C1・C2・C4は記録仕様を変えません。C3だけが設計判断です。

### D. 公開手順を一本化する（統括repoで決定）

推奨は**タグ駆動だけにする**ことです。PRをマージし、mainのコミットにタグを打ち、`release.yml` がそのコミットからビルド・検証・公開する。`BUILD-ORIGIN.json` のcommitがタグと一致し、repo外のツールも中断操作も不要になります。代償はタグからの待ち時間（約12〜16分）と、タグ後に失敗したら版を上げ直すことです。

代案は現在の運用を正式にする形です。`release.yml` からタグ起動の `publish` を外し、「検証済みrunの成果物を昇格する」`workflow_dispatch`（run番号を入力し、tree一致を確かめてReleaseを作る）をrepoに入れる。マージ前に検証した成果物をそのまま出せますが、workflowが1本増えます。

どちらでも、`releasing.md` を実態に合わせ、`RELEASE-VERIFICATION.json` を作る手順をrepoから再現できるようにします。

### E. 小さな整備（Codex、まとめて1 PR）

- GUI APIの `stop_requires_lease` を実装に合わせて `false` にし、`gui-api.md` に「tokenがあればleaseなしで止められる」と書く。共通SDKは値を読んで動くので変更不要（`bindings/control/kohdalab_control.py:473`）。5アプリ共通の約束に関わるなら統括repoで確認する。
- CONEX参照サーバーは、ループバック以外へのbindでtokenがなければ起動を拒否する。`install.md` に設定手順を書く。
- `gui-api.md`・`control-api.md` に、descriptorの保護がWindowsではユーザープロファイルの権限に依存することを書く。
- BG取得の直後にLightField設定の読み戻しを判定し、読めない項目名を挙げて止める。文言を「変更された」と「読めない」で分ける。
- `rust-toolchain.toml` でCIと手元のRust版を固定し、`rust-version` でのビルドをCIに1 job足す。Dependabot（cargo・actions）を有効にする。
- TRKR由来の未使用分岐と `allow(dead_code)` を消す。

### F. 後回しでよいもの

- 大きな関数の分割、JSON行の送受信とStop待ちの共通化。A1で結果を型で渡す形にした後、テストを足しながら進める。実機確認の前に構造を動かす利点は小さい。
- 終了時にUIが固まる問題（3.2の11）。終了処理を別スレッドへ移し、進行を表示する。
- 最小化中にAPIとlease失効の処理が止まるか（eframeがフレームを配らない場合）。Demoで確かめてから要否を決める。

## 5. ユーザーが決めること

1. ~~A2の方針~~ → 決定済み：許容差内で外挿する（4節A）。
2. D：タグ駆動に戻すか、成果物の昇格を正式化するか（統括repoで）。
3. C3：bundleのハッシュ対象を広げるか。
4. 指示書commit（7246861）を公開repoへpushするか。
5. CONEX CCサーバーが届く前に、参照アダプターで実機確認を始めるか。
6. Previewを外す条件。案：B2の雛形で1構成（3900S・HRS-300・PM100系・CONEX Y軸）の合否記録が揃い、Aが入った版であること。
