# Hardware and transport / 機器と通信

[装置リスト・メーカー仕様と接続準備](laboratory-apparatus.md)

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

## Laboratory setup / 使用予定の構成

| Role / 役割 | Model / default / 型番・既定値 |
| --- | --- |
| Laser / レーザー | 3900S |
| Spectrometer / 分光器 | SpectraPro HRS-300, controlled through LightField / LightField経由 |
| Power meter candidates / パワーメーター候補 | PM100USB or PM100D / 使用機種は接続時に確認 |
| CONEX X default port / X既定ポート | COM3 on the controller PC / 制御PC側 |
| CONEX Y default port / Y既定ポート | COM4 on the controller PC / 制御PC側 |

The `apparatus` profile section records this setup and is saved with the run's settings JSON. It describes models and port defaults; it does not select a driver, identify an attached meter, or start laser control. The authoritative active port is `stage.com_port`, which remains COM4 (Y) by default. The current PLE runner controls one selected stage; X is reserved as setup information and is not moved automatically.

`apparatus`は設定JSONと測定時の設定記録へ保存します。型番・ポートの既定値を記録する項目で、ドライバー選択・接続機種の識別・レーザー制御は行いません。実際の接続先は`stage.com_port`で、現在の既定はYのCOM4です。現行PLEは選択した1軸を制御し、Xの情報は保持しますが自動で動かしません。既存の波長校正点はDemo用の例であり、3900Sの実測校正としては扱いません。
