# kohdalab-shutter companion

PLEの自動BG用に同梱した、API protocol 2対応のWindows GUIです。

1. `Start-Shutter.cmd`を開きます。Local control APIを有効にして起動します。
2. shutter GUIで対象のPicoへ接続し、使用するCHとOpen/Close角度を確認します。
3. PLEでAutomatic shutter → kohdalab-shutterを選びます。CH0は`0`です。

このGUIの実行にRustやPythonは不要です。Picoへのfirmware転送は行いません。Pico側にはSTATE対応firmwareと正しい校正が必要です。STATEは指令位置の状態で、光センサーによる遮光確認ではありません。

固定元：[kohdalab-shutter](https://github.com/woolen-yarn/kohdalab-shutter/tree/8d22245899c36206622341ed5957919508aac0a1)。`companion-origin.json`に版・commit・exeハッシュ、`source.zip`に対応するソース、`LICENSE`と`third_party/rust`にライセンスを収録しています。

GUIとCLIはWindows上でビルドし、依存DLLを検査しています。Pico・Wi-Fi・サーボ・光路の実機確認は現地で行います。切断時は最後の要求状態を保持するため、通信断を遮光成功と扱わないでください。
