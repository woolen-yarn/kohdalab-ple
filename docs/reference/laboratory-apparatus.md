# Laboratory apparatus / 装置リストと接続準備

2026-10-29、ユーザー指定の機器とメーカー一次資料を照合。カメラ・実装グレーティング・センサーヘッドは接続時にSDKから取得し、SDKが返さない情報だけ現地で補足します。未接続の時点で型番や測定可能範囲を推定しません。

## 接続時の自動取得と記録

| 対象 | 自動で読む情報 | 画面と保存先 |
| --- | --- | --- |
| LightField機器 | experimentに接続された機器の種別、Model、SerialNumber | Device詳細、`hardware.json`、各スペクトルの`settings.hardware.lightfield` |
| グレーティング | 現在の選択名、SDKの`GetCurrentCapabilities(Grating)`が返す選択候補 | Deviceの候補リスト、設定読戻し、スペクトルとrunの実機記録 |
| メーター本体 | Model、serial、メーカー、使用resource、helperのドライバー版 | Device詳細、GUI APIの`power_hardware`、`hardware.json` |
| センサーヘッド | 名称、serial、種別、SDKの校正メッセージ、現在のセンサーの波長上下限 | パワー読取り時にも更新し、各点の`acquisition_started`イベントに`power_hardware`として保存 |

接続して現在のグレーティングが読めたとき、指定欄が空なら実値を反映します。候補名はSDKの文字列をそのまま使い、固定の装置固有プリセットを使いません。露光や中心波長など、利用者が指定した測定条件は従来どおりApply・読戻しで確認します。センサーの全対応波長範囲をレーザーの走査範囲に自動転用しません。

`settings.json`は指定した条件と手入力の`apparatus`情報、`hardware.json`はrun開始時に取得した実機情報と有効な分光器設定です。実機記録を手入力のラベルで上書きしません。Demoの情報は`simulated`として記録します。旧runは実機情報を持たない場合があります。

SDK未対応・取得失敗は`available`、`null`、`errors`に明示します。Modelやserialが取得できないことだけでは測定を止めません。ただしセンサー未接続と明示されたときはパワー値を採用しません。両方で識別できた単一カメラの型番・serialや、BGに対応する測定設定が変わったときは生スペクトルを保持して測定を停止します。SDKの列挙順や、欠損していた識別項目が追加されたことだけでは停止しません。同じ種別が複数存在するなど対応が曖昧な場合、識別情報だけで交換を断定しません。

グレーティング名に溝数・ブレーズが含まれる場合は、その生の選択名を保存します。SDKが公開しないスリット幅・レーザー光学セット・物理フィルター構成は手入力で補足します。センサー校正メッセージの保存は、校正の有効性の認定ではありません。

## 装置リスト

| 役割 | 機器・既定値 | 接続・設定の所有者 | 現状 |
| --- | --- | --- | --- |
| 励起レーザー | Spectra-Physics 3900S | 光学系の波長調整をCONEXで走査。ポンプレーザーや本体の遠隔APIは扱わない | 使用する光学セット・実際の発振範囲は未確認 |
| 分光器 | SpectraPro HRS-300 | 取得PCのLightField Automation | USB接続、experiment内でカメラと組み合わせる |
| 検出カメラ | 未確定 | LightField | 型番・serial・冷却・ROI・読み出し条件を現地で確認 |
| グレーティング | 未確定 | LightField experiment、PLEのDevice設定 | 実装品の溝数・ブレーズ・正確な選択名を確認 |
| CONEX X | COM3、使用装置の範囲0–25 mm | CONEX制御PC | 既定の対応。現行PLEは自動でXを動かさない |
| CONEX Y | COM4、使用装置の範囲0–25 mm | CONEX制御PC、remote JSON経由 | 現行PLEの走査対象 |
| パワーメーター | Thorlabs PM100USB または PM100D | USBで取得PCへ接続、旧TLPM .NET helper | 使用機種・serial・USB resourceは未確定 |
| パワーセンサーヘッド | 未確定 | パワーメーター | 型番・serial・波長／パワー上限・校正日を確認 |
| BG遮光 | 手動を既定、任意のMFF | 操作者または明示設定したシャッター | 自動用の機種・配線・状態番号は未確定 |

0–25 mm、X/Y、COM3/COM4はこの研究室の指定です。CONEX-CC全製品共通のストロークや自動検出結果ではありません。

## メーカー仕様から決まる準備

**3900S：** メーカー資料ではCW Ti:sapphire、光学セット全体で675–1100 nm。Broadbandは700–1000 nm、BlueとX-Longは別の範囲です。設置済みの光学セット・ポンプ条件を確認して探索範囲を決めます。位置と波長の対応を仕様書から作りません。[Spectra-Physics公式データシート（2024年6月版）](https://www.spectra-physics.com/medias/sys_master/resources/h89/h9c/9954704359454/3900S/3900S.pdf)

**HRS-300：** 300 mmのトリプルグレーティング分光器で、USB/RS232制御を備えます。今回の経路はLightFieldに統一し、PLEから別のシリアル接続を同時に開きません。搭載グレーティングと検出器の組合せを確認してexperimentを作成します。単一の固定波長範囲をHRS-300の型番だけで決めることはできません。[Princeton Instruments公式HRS-300マニュアル](https://www.princetoninstruments.com/wp-content/uploads/2020/04/HRS-300-Manual-V1b.pdf)、[LightField公式製品情報](https://www.teledynevisionsolutions.com/products/lightfield/)

**CONEX-CC：** メーカーのUSB通信設定は921600 baud、USBごとのcontroller addressは1です。参照サーバーではNewport SDKが通信を所有します。COM番号は制御PCのDevice Managerで確認し、X/Yラベルと実際の軸を照合します。接続と原点復帰は別操作です。[Newport公式ドライバーマニュアル](https://www.newport.com/mam/celum/celum_assets/resources/CONEX-CC_-_LabVIEW_Drivers_Manual.pdf)

**PM100USB / PM100D：** 本体だけでは測定範囲が確定しません。CW光のパワー測定に適したセンサーと、その波長・入力範囲を確認します。PM100USBはUSBで読取り、PM100Dも今回の接続はUSBです。補正波長は励起光の波長です。[Thorlabs公式PM100USB仕様](https://www.thorlabs.us/newgrouppage9.cfm?objectgroup_id=4037)、[PM100D公式仕様](https://www.thorlabs.com/catalogpages/Obsolete/2025/PM100D.pdf)

**ドライバーの区別：** OPM 7の資料ではTLPMXとNI-VISA系PM100Dドライバーの切替が説明されています。本アプリは元zipの`ThorlabsPowerMeter.py`と`Thorlabs.TLPM_64.Interop.dll`／`TLPM_64.dll`を使う旧TLPM経路です。OPMで動くだけで互換とは判定しません。使用する旧SDKに合うUSBドライバー割当を、実際のPCで確認してください。`PM100D`という本体名と`PM100D.dll`というドライバー名は別のものです。[Thorlabs公式OPM 7マニュアル §9.1](https://media.thorlabs.com/contentassets/012d23129bc54ff2bfdbf4c6d4a306fa/tl_opm_v7.0.pdf)

## 接続前に埋める設定

[実機設定例](../../examples/ple-hardware-settings.json)をコピーし、実機で確認した値を記入します。例の中心780 nm、励起680–740 nm、PL765–795 nm、校正点は導入例で、3900Sの実測値ではありません。

| 設定 | 確認内容 |
| --- | --- |
| `spectrometer.python` / `LIGHTFIELD_ROOT` | 実際の64-bit Windows PythonとAutomation SDKの場所 |
| `spectrometer.experiment` | 同じWindowsユーザーで保存したexperiment名。HRS-300と正しいカメラを登録 |
| `spectrometer.grating` / `center_nm` | LightFieldの正確な選択名。空欄ならexperimentのグレーティングを保持し、読戻しで確認 |
| LightField側の設定 | 入出力ポート、スリット幅、ROI、縦方向binning、読み出し／gain、冷却・安定状態、波長校正を記録。PLEはこれらを自動変更しない |
| `stage.host` / `port` / `com_port` | サーバーPCの実アドレスと待受port、制御PC側のY=COM4。PC間通信とUSB通信を混同しない |
| `stage.protocol` / `token_env` | 実機設定例は`remote_json`。両PCで認証トークンを揃える。旧設定の`legacy_tcp`は独立Stop非対応 |
| `power.driver_path` / `resource` | 確認済みhelperフォルダーと使用機器の完全なUSB resource名。複数台接続時はserialで照合 |
| `power.wavelength_mode` | 波長依存補正には実測の位置校正を登録した`position_calibration`。`fixed`のまま広い波長走査を校正済みと扱わない |
| `power.average_s` / `scan.settle_s` | センサーの応答・波長変更・出力安定・auto-rangeを実測して決める。例の時間は保証値ではない |
| `power.correction_factor` | センサー位置から試料位置への光路損失を実測。機器内の補正との二重適用を避ける |
| `background` | 最初はmanual。分光器の検出器シャッターを閉じただけで励起光も遮断したとみなさない |
| `apparatus` | Device詳細の手入力補足。SDKの自動取得情報は別に表示・保存するため、型番の転記は不要。レーザー光学セットなど自動取得できない構成を補足 |

現在は1つの取得スペクトルから励起ピークとPLを解析します。初回CSVで両方の波長領域に画素があることを確認してください。励起光カットフィルターで励起ピークが消える、飽和する、または検出器範囲外の場合は、この方法を成立と判定しません。光路を見直すか、独立に測定した位置校正を使う運用を選びます。波長区間に画素があるだけでは波長校正の確認にはなりません。

PLEのBGは分光器の背景で、パワーメーターのゼロ調整とは別です。パワーのゼロ・単位・飽和・機器内補正はメーカーソフトで確認します。装置をPLEに渡す前に、LightField/OPM/CONEX GUIなどによる同じ機器の占有を解消します。

## 実機で進める順序と記録

1. メーカーソフトでカメラ／HRS-300、メーター／センサー、CONEX各軸の認識を確認し、型番・serial・SDK版を記録する。
2. [導入手順](../guides/install.md)に従って非操作preflightを実行。SDKファイルと型の確認であり、USBドライバー、センサーやネットワークの実接続試験ではない。
3. [実機確認手順](../guides/hardware-qualification.md)に従って接続、設定読戻し、単発取得・SPE/CSV export、CONEXの小移動とStop、power読取りを個別に確認する。原点復帰やレーザーの始動はPLEのConnectで実行しない。
4. 遮光してBG、復帰して初回PL。励起／PLの包含、飽和、BG差分、検出波長とpower補正波長の一致を確認する。
5. 少数点のPLEと別runのStop試験を実施し、`scripts/verify_run.py`の結果と実機確認フォームを保存する。

| 現地で追記する事項 | 記録 |
| --- | --- |
| カメラ型番／serial、冷却・ROI・binning・gain | 未確定 |
| 各gratingのLightField名／溝数／ブレーズ、スリット、ポート | 未確定 |
| 3900Sの搭載optics、ポンプレーザー、使用励起範囲 | 未確定 |
| メーター型番／serial／resource、センサー型番／serial／校正日 | 未確定 |
| 制御PC名、実port、X/Y・COM・serial対応、サーバー版 | 未確定 |
| 実測位置–波長校正、power応答／安定時間、試料への補正係数 | 未確定 |

Manufacturer specifications do not establish qualification of the installed optical setup. The model list is confirmed by the user; the unknown entries must be filled on the instrument PC. / メーカー仕様の照合と、設置済み構成の実機確認を区別します。
