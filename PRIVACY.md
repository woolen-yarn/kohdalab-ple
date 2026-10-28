# Privacy / データの扱い

The application stores spectra, settings, device addresses and logs in the selected local run directory. It does not upload measurements or provide telemetry. The optional GUI API listens on loopback; the separately operated CONEX server uses its configured interface/token.

スペクトル・設定・機器アドレス・ログを指定したローカル測定フォルダーへ保存します。測定データの自動送信・テレメトリーはありません。GUI APIはloopback限定で、別運用のCONEXサーバーは指定interface/tokenを使用します。

GitHub CI downloads dependencies and tests simulated devices. Remove private profiles, addresses, tokens, experiment data and personal information before submitting logs/issues. / GitHub CIは依存物を取得し模擬機器を試験します。ログやIssueを提出する前に私有設定・アドレス・トークン・実験データ・個人情報を除いてください。
