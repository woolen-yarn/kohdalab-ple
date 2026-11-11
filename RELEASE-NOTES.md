# KohdaLab PLE 0.3.12 preview

## Changes / 変更内容

- Keep separate recovery latches for spectrometer Stop, shutter cleanup, stage Stop and a run interrupted with unconfirmed device state. A LightField idle confirmation no longer clears an unconfirmed shutter close. When shutter closure is unconfirmed, the recovery dialog asks for a spectrometer disconnect, which confirms closure, instead of offering Retry Stop. / 分光器Stop・シャッター・ステージStop・未確認のまま中断した測定の復帰ラッチを個別に保持します。LightFieldのidle確認ではシャッター閉の未確認を解除しません。シャッター閉が未確認のときは、復帰ダイアログでRetry Stopの代わりに分光器の切断（切断時に閉を確認）を案内します。
- Convert a stage readback just outside the calibration range by extrapolating the end segment within the configured stage position tolerance, and log each such acquisition in `analysis.log`. Commanded positions outside the calibration range are still rejected. / 校正範囲をわずかに外れた読み戻し位置は、設定済みのステージ位置許容差以内に限り端の区間の傾きで外挿し、該当する取得を `analysis.log` に記録します。校正範囲外の指令位置は従来どおり拒否します。
- Read bridge stderr as bytes, replace invalid UTF-8 and bound line length, so SDK messages in a non-UTF-8 code page no longer end bridge logging. The bridge starts with `PYTHONIOENCODING=utf-8`. / bridgeのstderrをバイト列で読み、不正なUTF-8を置換して行長を制限します。UTF-8以外のコードページで出たSDKメッセージでbridgeのログが止まらなくなります。bridgeは `PYTHONIOENCODING=utf-8` で起動します。
- Disconnect the stage only after three consecutive position-monitor failures; a move result takes precedence over an overlapping monitor failure. / 位置監視が3回連続で失敗した場合だけステージを切断し、移動と重なった監視失敗より移動結果を優先します。
- Report the original shutter connection error when automatic BG never acquired a shutter, without raising device recovery. / 自動BGでシャッターを確保できなかった場合は元の接続エラーを返し、機器復帰の表示にしません。
- Bring `START-HERE.md`, the user guide, the installation guide and the hardware qualification checklist up to the current package, add a qualification record template, and check release version references in CI. / `START-HERE.md`・操作ガイド・導入手順・実機確認チェックリストを現行の配布版に合わせ、実機確認記録の雛形を追加し、版表記の整合をCIで検査します。

## Compatibility / 互換性

Configuration and GUI API schemas, saved file columns, fields and units are unchanged. Existing profiles and saved runs remain readable without migration. The only change to derived values is the excitation wavelength of an acquisition whose stage readback lies outside the calibration range by no more than the stage position tolerance: such a run previously stopped with an error and now continues with an extrapolated value and a log line.

設定とGUI APIのスキーマ、保存ファイルの列・項目・単位は変更していません。既存設定と保存済みrunは移行なしで読めます。導出値の変更は、読み戻し位置がステージ位置許容差以内で校正範囲を外れた取得の励起波長だけです。従来はエラーで停止していた測定が、外挿値と `analysis.log` の記録を伴って続行します。

The Preview channel remains unchanged. Windows physical acquisition requires manufacturer SDKs; the pinned Shutter companion is unchanged.

Preview扱いは維持します。Windows実機取得にはメーカーSDKが必要で、固定版Shutter companionは変更しません。

## Packages and validation / 配布と検証

Windows x64 Setup and portable ZIP, macOS Apple-silicon application, corresponding source and SHA-256 checksums are provided. Release validation covers software tests, extracted packages, isolated installer lifecycle and simulation. The recovery, readback and stage-monitor changes were exercised with simulated devices only; no physical-device fault/stop qualification is claimed. Windows binaries are unsigned; Mac apps are ad hoc signed and not notarized.

Windows x64 Setup／portable ZIP、macOS Apple siliconアプリ、対応ソース・SHA-256を提供します。ソフトウェア試験・展開後の配布版・隔離環境のインストーラー動作・模擬測定を確認します。復帰・読み戻し・位置監視の変更は模擬機器でのみ確認しており、実機異常・停止試験の合格は主張しません。Windowsは未署名、Macはアドホック署名・Apple公証なしです。
