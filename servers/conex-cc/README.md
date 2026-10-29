# CONEX-CC service slot

This folder intentionally contains no replacement controller implementation
yet.  It is the insertion point for the future CONEX-CC server program.  The
current, working reference adapter remains
[`bridge/conex_server.py`](../../bridge/conex_server.py); it is a legacy path
only in the sense that a supplied service may supersede it later, not that it
is disabled today.

## Controller-PC boundary

Run the service only on the Windows PC that has the Newport SDK and physical
CONEX-CC connection.  The PLE GUI is a TCP client and must never load a vendor
SDK or access the COM port itself.  The service is responsible for:

- loading the vendor SDK locally;
- opening the explicitly requested COM port;
- enforcing its physical/software limits (the PLE profile additionally limits
  the user-facing range to 0–25 mm);
- rejecting unsafe motion, and servicing `stop` independently of an active
  move; and
- reporting a confirmed readback, rather than a requested position.

The present adapter uses one newline-terminated JSON object per TCP connection
(maximum 16 KiB).  Requests have `revision: 1`, an optional `id`, an `op`, and
an `args` object.  Replies are
`{"id": ..., "ok": true, "result": ... , "error": null}` or the matching
failure with `ok: false`.  The compatibility operations are:

| Operation | Required behavior |
| --- | --- |
| `hello` | Report protocol `kohdalab-conex`, revision `1`, and capabilities. |
| `connect` | Open the explicit `com_port`; do not home or enable implicitly. |
| `status` | Read position, limits, readiness, and controller error state. |
| `move` | Move to absolute `position_mm`, only when the controller is ready and within its reported limits. |
| `stop` | Abort an active move and send the device stop command. |

Optional shared-token authentication is part of the current protocol.  Bind to
loopback by default; any remote binding needs the lab network controls and a
token configured outside version control.

## Adding the supplied server later

1. Add the program and its runtime-specific files below this directory (for
   example `implementation/`), preserving the protocol above or intentionally
   versioning a documented replacement.
2. Keep `bridge/conex_server.py` intact until a controller-PC qualification
   confirms the replacement can connect, read status, move within limits, and
   stop under load.
3. Add a non-hardware `--help` / configuration validation path, tests with a
   fake controller, and an operator startup guide before declaring it the
   default adapter.
4. Never make the desktop package auto-start, auto-connect, home, enable, or
   move a controller service.

## Reference entrypoint

`run_reference.py` is a deliberately inert launcher.  Its help path makes no
network connection and performs no device action.  Starting the current
reference adapter requires an explicit `--run-reference` acknowledgement:

```sh
python3 servers/conex-cc/run_reference.py --help
python3 servers/conex-cc/run_reference.py --run-reference --host 127.0.0.1 --port 9000
```

The second command delegates to `bridge/conex_server.py`; it is for a
controller PC after the Newport SDK and safety checks have been completed.

## 日本語

### CONEX-CC サービス用の予約場所

このフォルダには、CONEX-CC コントローラを置き換える実装をまだ入れて
いません。後日提供されるサーバープログラムの追加場所です。現在動作して
いる参照アダプタは [`bridge/conex_server.py`](../../bridge/conex_server.py)
のままです。将来別実装が提供される可能性を示すためにレガシーパスと呼んで
いますが、現在無効化されているという意味ではありません。

### コントローラ PC との境界

このサービスは Newport SDK と物理的な CONEX-CC 接続を持つ Windows PC 上で
のみ動作させます。PLE GUI は TCP クライアントであり、メーカー SDK をロード
したり COM ポートへ直接アクセスしたりしてはいけません。サービスは次を担当
します。

- メーカー SDK をローカルでロードすること。
- 明示的に指定された COM ポートを開くこと。
- 物理およびソフトウェア上のリミットを強制すること（PLE プロファイルでも、
  ユーザーが指定できる範囲を 0–25 mm に制限します）。
- 危険な移動を拒否し、実行中の移動とは独立して `stop` を処理すること。
- 要求値ではなく、確認済みの実測位置を返すこと。

現行アダプタは、TCP 接続ごとに改行で終端された JSON オブジェクトを一つだけ
受け取ります（最大 16 KiB）。リクエストには `revision: 1`、任意の `id`、
`op`、および `args` オブジェクトを含めます。応答は
`{"id": ..., "ok": true, "result": ..., "error": null}`、または
`ok: false` の同形式の失敗応答です。互換性のための操作は以下です。

| 操作 | 必要な動作 |
| --- | --- |
| `hello` | protocol `kohdalab-conex`、revision `1`、capability を返す。 |
| `connect` | 明示した `com_port` を開く。暗黙のホーム・有効化はしない。 |
| `status` | 位置、リミット、ready 状態、コントローラエラー状態を読む。 |
| `move` | コントローラが ready で、報告したリミット内の場合に絶対座標 `position_mm` へ移動する。 |
| `stop` | 実行中の移動を中断し、デバイスの stop コマンドを送る。 |

任意の共有トークン認証も現行プロトコルの一部です。既定では loopback に bind
してください。リモート bind には、研究室ネットワークのアクセス制御と、
バージョン管理外で設定するトークンが必要です。

### 提供されるサーバーを後で追加する手順

1. このディレクトリ以下（例: `implementation/`）にプログラムと実行環境固有の
   ファイルを追加します。上記プロトコルを保持するか、意図的な置換であれば
   バージョンと変更点を文書化します。
2. 置換候補が接続、status 読み出し、リミット内での移動、負荷中の stop を
   controller-PC 上で検証するまでは、`bridge/conex_server.py` を維持します。
3. 既定アダプタとする前に、ハードウェアを触らない `--help` / 設定検証経路、
   fake controller を使うテスト、オペレータ用起動ガイドを追加します。
4. デスクトップ配布物からコントローラサービスを自動起動・自動接続・ホーム・
   有効化・移動してはいけません。

### 参照用エントリポイント

`run_reference.py` は意図的に何もしないランチャーです。help 表示ではネット
ワーク接続もデバイス操作も行いません。現行の参照アダプタを起動するには、
明示的な `--run-reference` の確認が必要です。

```sh
python3 servers/conex-cc/run_reference.py --help
python3 servers/conex-cc/run_reference.py --run-reference --host 127.0.0.1 --port 9000
```

2 行目は `bridge/conex_server.py` に処理を委譲します。Newport SDK の導入と
安全確認が完了したコントローラ PC 上でのみ使用してください。

## Lab port defaults / 研究室の既定ポート

CONEX X: **COM3**, CONEX Y: **COM4**, on the controller PC. The current PLE profile uses Y/COM4 through `stage.com_port`. These are defaults, not device discovery results; changing the profile does not command motion. The future supplied server should preserve the axis/port distinction. / 制御PC上でX＝COM3、Y＝COM4を既定とします。現在のPLEは`stage.com_port`でY／COM4を指定します。これは既定値であり機器を検出した結果ではありません。後日統合するサーバーでも軸とポートを区別します。
