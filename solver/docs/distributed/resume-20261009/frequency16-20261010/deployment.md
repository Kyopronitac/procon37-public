# C013の本番配置完了

2026-10-10 06:06:13 JST に、選定したC013をVOIDLINUXの3台へ配置し、起動準備と停止確認を完了した。比較評価終了後のcleanup記録で `status=completed` / `productionProbePermitted=true` を確認してから配置した。詳細は [deployment.json](deployment.json) に保存した。

## 当日の入口

VOIDLINUX-1で、次のファイルの冒頭にある `SERVER` と `TOKEN` を当日の値に編集して実行する。

```sh
/home/voidlinux-1/procon37-competition/run.sh
```

この入口からVOIDLINUX-2/3をSSHで起動し、3台が1チームとして探索する。ユーザーが編集する入口はリリース本体とは分離しており、設定の編集で評価済みファイルの識別ハッシュは変わらない。

```sh
/home/voidlinux-1/procon37-competition/run.sh status
/home/voidlinux-1/procon37-competition/run.sh stop
```

従来D0用の `procon37/tester/server-bridge/distributed/run.sh` は置換していない。本番では上記の新しい入口を使用する。`current/run.sh` と `releases/` 配下は識別用ハッシュで固定している。

SERVER/TOKENは未設定のまま保持した。未設定での起動は `endpoint_unset` を表示して終了コード1で停止することを実機確認した。問題サーバーへのHTTP通信は行っていない。SSH認証情報はVOID1のリリース内 `private/ssh-password` に所有者限定権限で保持し、Git・配布アーカイブ・本記録には含めていない。環境変数にSSHパスワードを渡さず `status` が3台へ接続できることも確認した。

## 配置と識別情報

| 項目 | 値 |
| --- | --- |
| 試合用選定 | C013、解共有間隔500ms |
| ソルバSHA-256 | `6cdcd06d40e476f4992cd2bc76d14db331f88e63b705e1004eeec82abecb4422` |
| bridge SHA-256 | `e02fe74277a4178df5f0772bb3ad95af257fef4317df90841052e93ec83f2d90` |
| 配布アーカイブSHA-256 | `ed17d98e11334ab129311865db9cca2a388a19ec444f650b690424ed14bee497` |
| 各PCのリリース | `~/procon37-competition/releases/C013-6cdcd06d40e4/` |
| 安定した選択先 | `~/procon37-competition/current` |
| worker通信 | TCP 39371、wire protocol 4、cross-elite有効 |
| スレッド | VOID1=32、VOID2=16、VOID3=32、計80論理スレッド |
| CPU割り当て | VOID1=0–31、VOID2=0–15、VOID3=0–31 |
| 提出機会 | 5・15・25・35・45・55秒、bridgeとcoordinatorの両方でstream応答有効 |

直前に実際に送信した回答と同一なら、受理・却下・採否不明のいずれでも次の提出機会での再送を省く。1回のHTTP提出処理内での通信失敗時の再試行は維持する。選定ソルバとbridgeは評価した実行ファイルをコピーし、コンパイルや実装変更を行っていない。

起動単位の詳細ログは選択リリースの `logs/`、試合ごとの問題全体・マップ・日別道路状態・提出通信は `records/` に保存する。端末表示は指定されたアクセス確認・問題受信・起動・日・提出秒数・採否・終了に限定する。

## 確認結果

- 運用wrapperのローカル11テストが成功し、独立レビューでも阻害点なし。
- 3台の全manifest対象ファイルが一致。ソルバ・bridge・run.shの実行権限、シェル構文、CPU割り当てを確認。
- VOID2の16スレッドworkerとVOID3の32スレッドworkerを起動し、VOID1からwire 4/cross-eliteのTCP認証に成功。
- VOID1の32スレッドcoordinatorを空の標準入力で起動し、正常終了を確認。Solve要求0、HTTP通信0。
- probeで起動したworkerはPID・起動時刻・boot ID・UID・session・cwdを照合して停止。
- `status` は3台すべて `running=false` / `identity_mismatch=false` / `orphans=[]`。別途 `/proc` でもリリースに属する残留プロセス0を確認。
- 1台または2台だけ残る部分起動は再利用せず、起動を拒否して `stop` を案内する。

本番プロセスは停止済みで、当日の接続先設定とユーザーによる起動を待つ。採用判断・評価範囲・commit/pushの記録は同ディレクトリの選定記録で管理する。
