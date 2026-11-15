# 日中共有・複数提出版の本番配置

評価済みの分散ソルバ実行ファイルを、そのままVOIDLINUXの3台へ配置するためのツールです。リポジトリ側の `../distributed/run.sh` が選択されているリリースへ接続します。詳細は [本番運用と配置手順](docs/operation.md) を参照してください。

本番ではVOIDLINUX-1の `~/procon37/tester/server-bridge/distributed/run.sh` 冒頭にある `SERVER` と `TOKEN` を編集し、同じファイルを実行します。設定されていない状態では問題サーバーへ接続しません。VOIDLINUX-2/3で手動起動する必要はありません。

```sh
cd ~/procon37
./tester/server-bridge/distributed/run.sh
./tester/server-bridge/distributed/run.sh status
./tester/server-bridge/distributed/run.sh stop
```

3台は32・16・32論理スレッドで、1チームとして探索します。VOIDLINUX-1が集約と提出を担当します。60秒の行動回答の提出機会は5・15・25・35・45・55・57秒です。車種選択では制限時間に応じて調整し、終盤の提出機会を締切5秒前・3秒前に設けます。直前に送信した回答と同じ場合は追加提出を省略します。

実行ログは選択したリリースの `logs/`、問題・マップ・道路状態・提出通信は `records/` に保存します。端末表示はアクセス確認・問題受信・起動・現在の日・提出秒数・採否・終了に限定します。
