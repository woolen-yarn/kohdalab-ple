# Hardware and transport / 機器と通信

[SDK downloads and installation / 入手・導入](../guides/install.md) · [Physical qualification / 実機確認](../guides/hardware-qualification.md)

| Role / 役割 | Current contract / 現行仕様 |
| --- | --- |
| Spectrometer / 分光器 | Owned LightField Automation instance; grating/center/exposure/combined frames readback, single ROI/frame SPE+CSV / LightField所有インスタンス・設定読戻し・単一ROI/frameのSPE/CSV |
| CONEX CC | Absolute 0–25 mm; actual position readback, manual Move/Stop, scan interlock / 絶対位置0–25 mm・読戻し・手動移動停止・走査との排他 |
| Remote JSON server / サーバー | Revision 1, newline-delimited JSON, separate status/Stop connections / JSON一行通信・独立した監視と停止 |
| Legacy TCP / 旧サーバー | Existing GET/move interface; monitored Stop unavailable / 既存GET・移動方式。監視付きStopは提供しない |
| Power meter / パワーメーター | Existing Thorlabs TLPM .NET helper; W/dBm conversion, wavelength/averaging readback / TLPM系helper・単位換算・波長/平均の読戻し |
| BG / 背景 | Manual optical confirmation or explicit-state MFF shutter / 手動遮光確認または状態番号を明示したMFF |

The GUI owns measurement planning; the controller-PC service owns COM/Newport SDK access. The reserved [CONEX CC server directory](../../servers/conex-cc/README.md) documents how a later supplied server will be incorporated. Its actual program is pending, and no undocumented protocol is assumed compatible. The current [reference bridge](../../bridge/README.md) remains usable.

GUIは測定計画を、制御PCサービスはCOMとNewport SDKを所有します。後日提供されるサーバーの取り込み先は[servers/conex-cc](../../servers/conex-cc/README.md)です。その実プログラムは未収録で、未確認の通信方式を互換とみなしません。現行の[参照ブリッジ](../../bridge/README.md)は維持しています。

Implemented support is distinct from physical qualification. For device/firmware combinations and SDK signatures, retain actual-PC evidence. / 実装対応と実機確認を区別し、機器・ファームウェア・SDK署名の組合せは実際のPCで証跡を残します。
