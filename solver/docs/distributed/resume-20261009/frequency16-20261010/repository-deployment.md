# VOID1へのリポジトリ全体配置

2026年10月10日06:19:38 JST、VOIDLINUX-1の `/home/voidlinux-1/procon37` にGit管理ファイル全体と `.git` を配置し、通常の入口からC013へ接続した。ブランチは `feat/parallel-distributed-solver`、originは `https://github.com/Kyopronitac/procon37.git`。作業PC上の未追跡の開発途中ファイルやキャッシュは含めず、Gitリポジトリの全体を配置している。

## 当日の操作

VOID1で `tester/server-bridge/distributed/run.sh` 冒頭の `SERVER` と `TOKEN` を編集する。

```sh
cd /home/voidlinux-1/procon37
./tester/server-bridge/distributed/run.sh
```

`status` を付ければ3台の状態確認、`stop` を付ければ停止できる。VOID2/3への手動操作は不要。URL・トークン未設定時は接続せず終了する。今回の配置ではコンパイル・再配置は不要。

## 配置と保存先

| 項目 | 場所 |
| --- | --- |
| リポジトリ全体 | `/home/voidlinux-1/procon37` |
| 当日編集する入口 | `tester/server-bridge/distributed/run.sh` |
| Git管理外の選択リンク | `.server/distributed-production/selected-release` |
| 選択した不変リリース | `/home/voidlinux-1/procon37-competition/releases/C013-6cdcd06d40e4` |
| 問題・マップ・HTTP記録 | `.server/submissions` → リリースの `records/` |
| 完全ログ | `.server/bridge-logs` → リリースの `logs/` |
| 元のリポジトリ全体の退避先 | `/home/voidlinux-1/procon37.before-repository-deploy-20261010-061937` |

旧リポジトリの変更・設定・ビルド成果物は退避先へそのまま保存した。旧配置を復元する場合は、まず新入口の `stop` と `status` で所有プロセスの終了を確認し、新しいリポジトリを別名へ退避してから上記バックアップを `procon37` に戻す。バックアップの起動設定は旧D0用なので、C013を使う場合は今回の選択設定も復元する必要がある。

実行ファイルは3台ともソルバSHA-256 `6cdcd06d40e476f4992cd2bc76d14db331f88e63b705e1004eeec82abecb4422`、bridge `e02fe74277a4178df5f0772bb3ad95af257fef4317df90841052e93ec83f2d90`。再ビルドしていない。各32・16・32論理スレッド、TCP39371、500ms共有、5・15・25・35・45・55秒の提出機会を維持する。同じフェーズの直前の実送信内容と同一なら次の提出を省略する。

## 配置方法と検証

1. 実在するホーム・リポジトリと停止状態をSSHで確認した。
2. 作業PCで全体をGit cloneし、originを通常のURLに設定した。全体アーカイブをSSH転送し、SHA-256一致後に展開した。保護された実装の内容は閲覧・検索していない。
3. 旧リポジトリをディレクトリごと退避し、新配置へ切り替えた。上記3つのGit管理外リンクを設定した。
4. VOID1にGit 2.55.0を通常のxbpsで導入した。外部DNSへ到達できないため、公式パッケージを作業PCで取得し、全パッケージの公開SHA-256を照合して転送した。新規15パッケージとlibxcryptの4.4.36→4.4.38更新だけを行い、他の既存パッケージは更新・削除していない。Gitのinit・commit・clone・fsckとPerl関連モジュールを一時リポジトリで検証した。
5. 通常入口のシェル構文と8項目の引数・環境変数・選択エラー・D0互換性を確認した。実機でも通常入口から3台のstatus、URL未設定時の起動拒否を確認した。3台の不変リリースmanifest・実行ファイル・設定・実行権限・CPU範囲・認証ファイル権限がすべて一致した。

本番問題サーバーには接続せず、対戦テストは追加していない。配置確認時は3台とも `running: false`、`identity_mismatch: false`、`orphans: []`。実測・Git導入証跡は [repository-deployment.json](repository-deployment.json) に保存する。変更は日本語Conventional Commitで保存・通常pushし、最終commit ID・push結果・VOID1との一致確認は作業PCとVOID1の `.server/distributed/repository-deployment-finalization.json` に保存する。
