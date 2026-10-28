# Reporting security issues / セキュリティ問題の報告

Report sensitive findings to the repository owner privately; use GitHub private vulnerability reporting if available. Do not include live API tokens or laboratory network credentials in public issues. / 機密を含む問題はリポジトリ所有者へ非公開で報告し、利用可能ならGitHubのprivate vulnerability reportingを使ってください。生のトークン・研究室の接続情報を公開Issueへ載せないでください。

The GUI API is opt-in and loopback-only. Remote CONEX service deployment and firewall access are administered separately on the controller PC. / GUI APIは明示的に有効化するloopback通信です。CONEXサービスの公開範囲・firewallは制御PC側で別管理します。
