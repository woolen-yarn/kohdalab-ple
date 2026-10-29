# Python / Jupyter client

Install the optional client from this folder / 任意のクライアントを導入:

```sh
python -m pip install ./bindings/python
```

The client uses only the Python standard library and controls the GUI through its authenticated local API. It does not open devices itself. / クライアントは標準ライブラリのみを使い、通常起動時に有効なローカルAPIでGUIを操作します。機器は直接開きません。

[API and complete usage / API・使用例](../../docs/reference/gui-api.md) · [SDK installation / SDK導入](../../docs/guides/install.md)

PLE defaults to requesting Stop on client timeout, lost ownership or an escaping client error. This product-specific policy differs from TRKR's default; method/receipt schemas are not interchangeable. / PLEはクライアントの期限切れ・所有権喪失・例外時にStopを要求する既定です。この製品固有の方針はTRKRの既定と異なり、メソッドやreceiptをそのまま差し替える互換性はありません。

## Open GUI / 起動中のGUI

```python
from kohdalab_ple import GuiClient
with GuiClient() as ple:
    print(ple.status())
    print(ple.devices())
    print(ple.shutter_status())
```

`GuiClient`は既存の`GuiPLE`と同じクラスです。新しいGUIを起動する場合は`GuiClient.launch(profile=...)`、配置先を明示する場合は`executable=...`または`KOHDALAB_PLE_EXE`を使います。`set_background(...)`から[kohdalab-shutter自動BG](../../docs/reference/shutter.md)を設定できます。
