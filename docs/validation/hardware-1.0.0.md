# Hardware evidence for 1.0.0 / 実機確認の範囲

The recorded Windows device tests used the spectral icon and the LightField W/I, native TLPMX and canonical-path patches integrated in fec060f, b3019b9 and 4c64344. These device tests preceded final 1.0.0 packaging and are distinct from package verification. / Windowsでの実機記録は、アイコンとLightField W/I・native TLPMX・パス修正（統合コミットfec060f・b3019b9・4c64344）を適用したものです。最終1.0.0梱包前の実機試験であり、配布物の検証とは区別します。

| Component / 対象 | Observed / 確認内容 | Limit / 範囲 |
| --- | --- | --- |
| 3900S shutter CH00 | Close → Open → Close, settled controller feedback / 開閉・整定の読戻し | No optical blocking sensor / 光学遮断センサーなし |
| CONEX CC, COM4, legacy TCP | 18.720 → 18.715 → 18.720 mm, independent position readback / 5 µm往復と位置照合 | Immediate Stop during movement unverified / 移動中の即時Stopは未検証 |
| PM100USB / S130C, native TLPMX | Connect, identify, 800 nm readback, three finite dark readings, disconnect / 接続・識別・800 nm読戻し・暗読値3回・切断 | No illuminated power accuracy or calibration acceptance / 光照射時の精度・校正合格ではない |
| LightField, PIXIS 100BR eXcelon / HRS-300-SS | 100 ms, one frame, center 815 nm; 1340 bins, 788.4312–840.3543 nm; SPE/CSV/array saved / BG取得と原本保存 | BG only; no signal normalization in retained run / 記録runはBGのみ、信号正規化なし |

On 2026-11-20, the one-point run stopped at the signal stage because no optical input was present (operator confirmed). Its status remained stopped with zero completed signal acquisitions. Final state: shutter closed, stage 18.7200 mm, all devices disconnected and cleanup complete. Local raw records and 15 file hashes were retained. Machine-specific paths and experiment data are excluded from the public package. / 2026-11-20の1点runは、操作者が光を入れていないと確認して信号段階で停止しました。信号取得0件・stoppedを維持し、最終状態はシャッター閉・ステージ18.7200 mm・全機器切断・後処理完了です。原本と15ファイルのハッシュはローカル保存し、端末固有パス・実験データは公開配布へ含めません。

On 2026-11-23, the operator reported hardware confirmation complete and authorized review and publication. No additional illuminated-run artifact was supplied in this release-preparation session. Operator acceptance is distinct from independently verified end-to-end acquisition or a calibration certificate. / 2026-11-23に操作者から実機確認済み・レビューと公開を進める指示を受けました。この公開準備では追加の光照射run原本は受領していません。操作者確認として記録し、独立した通し測定検証や校正証明とは分けます。
