# Controller-PC services

This directory is reserved for programs that run on a **controller PC** with
physical instrument access.  They are not started by the PLE desktop app and
are packaged only so the operator has the matching service documentation next
to the application.

The client application remains responsible for presenting controls, recording
the requested settings, and requiring an explicit operator action before a
move.  A controller service owns the vendor SDK, serial/USB connection, and
all motion interlocks.  Do not place vendor DLLs, SDK installers, credentials,
or laboratory-specific tokens in this repository or a release archive.

`conex-cc/` is a staging point for the dedicated CONEX-CC server source that
will be supplied later.  Until then, `bridge/conex_server.py` is the maintained
reference adapter and the only implementation used for laboratory operation.

## 日本語

このディレクトリは、物理的な装置に接続できる**コントローラ PC**上で動作
するプログラムのために確保しています。PLE デスクトップアプリはこれらを
起動しません。アプリと同じ配布物に入れるのは、対応するサービスの説明を
オペレータが参照できるようにするためです。

クライアントアプリは、操作画面の提示、要求した設定の記録、および移動前の
明示的なオペレータ操作を担当します。コントローラサービスは、メーカー SDK、
serial/USB 接続、およびすべてのモーションインターロックを担当します。
メーカー DLL、SDK インストーラ、認証情報、または研究室固有のトークンは、
このリポジトリや配布アーカイブに置かないでください。

`conex-cc/` は、後日提供される専用 CONEX-CC サーバーソースの受け入れ場所
です。それまでは `bridge/conex_server.py` が保守済みの参照アダプタであり、
研究室の運用に使用する唯一の実装です。
