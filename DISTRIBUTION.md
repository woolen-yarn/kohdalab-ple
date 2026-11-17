# Distribution / 配布

Open **Actions → Distribute → Run workflow** on the default branch. A successful run builds and validates the selected commit, publishes a build-numbered release, and makes it the latest release. Select **verify_only** to build and test before publication.

既定ブランチの **Actions → Distribute → Run workflow** を1回実行します。ビルド・検証に成功すると配布番号を自動で付け、最新版として公開します。公開前のビルド確認には **verify_only** を選択します。

[Latest download / 最新版ZIP](https://github.com/woolen-yarn/kohdalab-ple/releases/latest/download/kohdalab-ple-windows.zip) · [Previous distributions / 過去の配布](https://github.com/woolen-yarn/kohdalab-ple/releases) · [SHA-256](https://github.com/woolen-yarn/kohdalab-ple/releases/latest/download/SHA256SUMS.txt)

The fixed URL becomes available after the first Distribute publication. Windows CI tests and package validation. Physical-device acceptance follows the separately documented model and connection matrix.

固定URLはこのワークフローの初回公開後に利用できます。各版にはコミット・実行URL・SHA-256とソフトウェア検証の範囲を記録します。装置別の実機検証範囲は各アプリの記録を参照してください。

This job sends its distribution directly to Releases. Existing workflows may retain their own review artifacts. The application version stays in the package and the distribution number is recorded in BUILD-INFO.json.

配布ファイルは同じジョブからReleasesへ添付します。既存のテストや正式版作成ワークフローの成果物は、それぞれの保持期間に従います。アプリ内の版番号と配布番号の対応はBUILD-INFO.jsonで確認できます。

Build-numbered distributions update GitHub’s latest release. They record CI validation only and do not establish new hardware acceptance. README stable-version downloads remain pinned to the qualified version; the one-click distribution ZIP is the separate latest-build download.

配布番号付きの版はGitHubのlatestを更新します。実機検証済みとは扱いません。READMEの安定版ダウンロードは検証済みの版に固定し、最新ビルドのZIPはこのページの固定リンクで案内します。
