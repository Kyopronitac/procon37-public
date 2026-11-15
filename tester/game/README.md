# 5人対戦テスタ

対象ソルバ1個とサンプルソルバ4個を同じ盤面で対戦させる、独立したCUI・GUI。対象はプレイヤー0、相手は1〜4。5人固定で、在庫・燃料・訪問履歴・得点を各チームで管理する。順位は **種類数 → 日別種類数 → 玉数** の順に比較し、同点は同順位。回答時間は順位に使わない。

Rust 1.94以降を用意し、リポジトリのルートで実行する。

```sh
# 単一スレッド対象のフルテスト：対象とサンプル4個をビルドし、seed 0〜29で対戦
cargo run --release -p game -- run 30 --label '対戦・標準構成・フル30件'

# GUI（既定 http://127.0.0.1:8081/）
cargo run --release -p game -- serve

# 1ケースを実行
cargo run --release -p game -- run --seed 0 --label '対戦・公式1回戦'
```

フルテストの件数は [ルートREADME](../../README.md#テスト) に従う。対象が単一スレッドなら30件を明示する。件数省略時のテスタ既定値は60件。

保存先は `.game/runs/`。単独テスタの `.tester/runs/`、ポート8080とは分離される。同じseed・形式の問題は `players` 以外すべて単独テスタと一致する。

| 既定値 | 内容 |
| --- | --- |
| ケース・形式 | 60件。seed % 3で1回戦・準決勝・決勝 |
| 対象 | ルートの `main.rs` を `cargo build --release -p solver` でビルド |
| 相手 | Pool第2・第1・第4・第3世代のソルバ（プレイヤー1〜4の順、各単一rs） |
| 制限時間 | 単独テスタと共通。選択は形式の上限、各日は問題の `daySeconds` |
| 同時試合数 | `max(1, CPU数 / 5)`。誰かに内部並列ありなら自動は1試合 |
| サンプル乱数seed | 各コマンドの第1引数。既定 `0`、同じ入力・seedで同じ回答 |

Linux / WSLでのアプリ登録：

```sh
bash tester/game/desktop/setup.sh
game
# 変更を反映する際
game --restart
```

`solo` と同時に登録・起動できる。`--stop` はそれぞれのサーバーだけ停止し、履歴を残す。アンインストールは `bash tester/game/desktop/setup.sh --uninstall`。

- [CUI・GUIの操作と設定](docs/usage.md)
- [通信・対戦ルール](docs/protocol.md)
- [保存形式・比較・ZIP](docs/history.md)
- [検証結果](docs/validation.md)
- [共通実装](../../common/README.md)
- [サンプル利用と閲覧制限](samples/README.md)

対象ソルバの開発・評価中にサンプルの内部を閲覧・検索・逆解析してはいけない。[AGENTS.md](../../AGENTS.md) と [サンプル側の規則](samples/AGENTS.md) を参照。ビルド・実行・成績・リプレイ確認は許可される。
