# テスタ

ヘキサうどんの対戦・単独テスタ。通常の評価は5人対戦の `game`、単独動作の確認は `solo` を使える。どちらもCUI・GUIで実行、履歴比較、盤面の再生、ZIP入出力、GIF出力ができる。複数PCのソルバを公式簡易サーバー経由で対戦させる `server` も同じ画面・履歴形式で使える（ZIP入出力を除く）。

| 用途 | ディレクトリ | Cargoパッケージ | GUIポート | 履歴保存先 |
| --- | --- | --- | --- | --- |
| 対象ソルバとサンプル4種の5人対戦 | [game](game/README.md) | `game` | 8081 | `.game/runs/` |
| 対象ソルバ1チームの単独実行 | [solo](solo/README.md) | `tester` | 8080 | `.tester/runs/` |
| 公式簡易サーバー経由の対戦 | [server](server/README.md) | `server` | 8082 | `.server/runs/` |

以下はリポジトリルートで実行する。

```sh
# 対戦を短時間で確認する
cargo run --release -p game -- run 3 --time-limit-ms 1500 --label '対戦・接続確認' -- 0

# 対戦GUI
cargo run --release -p game -- serve

# 単独GUI
cargo run --release -p tester -- serve

# 公式簡易サーバー経由の対戦GUI（各ソルバPCで run.sh を起動しておく）
cargo run --release -p server -- serve
```

件数・制限時間・保存先は目的に合わせて選べる。指定しなければ60ケース・既定制限時間で実行し、上記の既定保存先へ履歴を残す。[フルテストの定義](../README.md#テスト) では、対象が単一スレッドの場合はsoloが60件、gameが30件なので、gameは `run 30` と明示する。

- [対戦の操作・設定](game/docs/usage.md)
- [単独の操作・設定](solo/docs/usage.md)
- [対戦サーバーの操作・設定](server/docs/usage.md)
- [AI向けの操作スキル](../.agents/skills/procon37-tester/SKILL.md)
- [共通基盤](../common/README.md)

## ディレクトリ移動について

旧 `tester/` は `tester/solo/`、旧 `game/` は `tester/game/` に移動した。Cargoパッケージ名、`tester::model` などの公開パス、履歴保存先は引き続き同じ。保存済み履歴の移行は不要。

移動前にデスクトップコマンドを登録していた場合は、使用する側のセットアップを再実行すると新しいパスへ更新できる。

```sh
bash tester/game/desktop/setup.sh
bash tester/solo/desktop/setup.sh
bash tester/server/desktop/setup.sh
```

登録後は `game`（対戦）、`solo`（単独）、`server`（対戦サーバー）で起動できる。
アプリ一覧の表示名も `game`・`solo`・`server`。セットアップは、このチェックアウトを参照する旧 `procon37`・`procon37-game` の管理済み登録を新しい名前へ移行する。