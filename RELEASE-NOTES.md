# KohdaLab PLE 0.3.0 — External control and automatic BG

Control the running PLE GUI with the standard-library Python/Jupyter `GuiClient` and acquire BG through kohdalab-shutter without opening a second Pico connection.

- Ordinary GUI startup publishes an authenticated local API. Existing `GuiPLE` remains supported; `GuiClient.launch`, executable discovery, `set_background` and `shutter_status` simplify automation. `--no-gui-api` disables the local endpoint.
- Native Rust protocol-2 client owns a renewable exclusive lease of an already-connected kohdalab-shutter GUI. It waits for job completion and settled controller Open/Close before acquisition.
- Stop/error requests Close; verified cleanup releases ownership. Uncertain mutations, lease loss and unresolved closure prevent the next measurement. Controller state is explicitly not optical-sensor confirmation.
- Preserve manual BG, MFF support, raw SPE/CSV, mandatory BG subtraction, first-PL analysis review, four plots and SDK apparatus discovery. Demo never connects a physical shutter service.

## Downloads

Extract `KohdaLab-PLE-v0.3.0-windows-x64.zip` completely and read `START-HERE.md`. The ZIP includes the client, SDK adapters, configuration examples and verification scripts. Rust is not needed to run it. Windows instruments require separately installed 64-bit Python, LightField and matching manufacturer software/helper. A pinned, compatible kohdalab-shutter Windows GUI/CLI companion is included with corresponding source and license texts. Start `companions/kohdalab-shutter/Start-Shutter.cmd` on the same acquisition PC, then connect the physical Pico before automatic BG.

macOS arm64 supports Demo/results. Windows binaries are unsigned; macOS uses ad hoc signing. Download the SHA-256 file with the packages. No physical camera, Pico, servo, optical closure or calibration qualification is claimed. See `VALIDATION.json` and the hardware checklist.

## 日本語

通常起動のGUIをPython/Jupyterから操作できるよう整え、kohdalab-shutterによる自動BGを追加しました。Pico接続はshutter GUIが所有し、PLEはRustからローカルAPIへ接続します。Close → 整定待ち → BG → Open → 整定待ち → 初回PLと進み、peak・積分区間を確認して残りを取得します。終了時は閉状態を確認して操作権を解放します。

通信断や操作権喪失を遮光成功とみなしません。STATEはコントローラーの状態であり、実光路は現地で確認します。従来の手動BG・MFFと生データ保存は維持しています。
