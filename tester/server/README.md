# 対戦サーバー

大会公式の簡易サーバー（`procon-server`）を使い、複数のPCで動くソルバを対戦させるCUI・GUI。このPCをサーバー役にして、各ソルバPCが対戦窓口へ接続して参加する。操作・画面・履歴の形式は [5人対戦テスタ](../game/README.md)（`game`）と同じで、チーム数は1〜16から選べる。

**本番のテストも兼ねる。** ソルバPCは公式API（`/setting`・`/agent`・`/`）だけを使い、本番の競技サーバーにも同じ `run.sh` で参加できる。違いは接続先とトークンだけ（[本番との互換性](#本番との互換性)）。

順位は **種類数 → 日別種類数 → 玉数** の順に比較し、同点は同順位。回答時間は順位に使わない。

## 準備

| 役割 | 台数 | 手順 |
| --- | --- | --- |
| サーバーPC | 1台 | [サーバーPCのセットアップ](docs/server-pc.md) |
| ソルバPC | チームごとに1台 | [ソルバPCのセットアップ](docs/solver-pc.md) |

サーバーPCには、公式配布の簡易サーバー（Linux版）を `tester/server/procon-server` に置く。配布物はgit管理対象外で、このリポジトリでは内容を変更しない。

各ソルバPCでは、リポジトリでソルバと変換器をビルドし、チーム番号を指定して `run.sh` を常駐させる。

```sh
cargo build --release -p solver -p server-bridge
TEAM=0 ./tester/server-bridge/run.sh
```

| ファイル | 内容 |
| --- | --- |
| `run.sh` | 試合が始まるたびに自動でソルバを実行する（[../server-bridge/run.sh](../server-bridge/run.sh)） |
| `server-bridge` | 公式APIとソルバの入出力を変換する（[../server-bridge](../server-bridge/README.md)） |
| `solver` | ソルバ本体 |

`TEAM` はそのPCが担当するチーム番号（0, 1, 2, …）。対戦窓口のURLやソルバの場所は既定値で動く。変更する方法は [ソルバPCのセットアップ](docs/solver-pc.md#5-runshを起動する) を参照する。

## 使い方

リポジトリのルートで実行する。

```sh
# GUI（既定 http://127.0.0.1:8082/）
cargo run --release -p server -- serve

# seed 0〜11 を2チームで対戦
cargo run --release -p server -- run 12 --players 2 --label '改善版 vs 基準版'

# 1試合だけ、短い制限時間で接続確認
cargo run --release -p server -- run --seed 0 --time-limit-ms 3000 --label '接続確認'
```

| 既定値 | 内容 |
| --- | --- |
| ケース・形式 | 12件。seed % 3で1回戦・準決勝・決勝（`game` と同じ生成器） |
| チーム数 | 1（`--players N`、最大16）。チームNは `token-pN` で参加 |
| 制限時間 | 公式条件。選択は形式の上限、各日は問題の `daySeconds` |
| 実行順 | seedの小さい順に1試合ずつ（公式簡易サーバーを1台ずつ起動するため並列なし） |
| 対戦窓口 | ポート11451（`--match-port`）。ソルバPCから接続できるよう全インターフェースで待ち受ける |
| 履歴 | `.server/runs/` |

Linux / WSLでのアプリ登録：

```sh
bash tester/server/desktop/setup.sh
server
# 変更を反映する際
server --restart
```

`solo`・`game` と同時に登録・起動できる。`--stop` はGUIサーバーだけを停止し、履歴を残す。アンインストールは `bash tester/server/desktop/setup.sh --uninstall`。

## 動き

```
[サーバーPC]                                   [ソルバPC × チーム数]
 server run / serve
   seedごとに問題を生成し、公式簡易サーバーを起動
   対戦窓口(:11451)に試合を公開         ←──  server-bridge が /setting で問題を取得
   提出を中継しながら記録              ←──  server-bridge がソルバの回答を提出
   試合終了後、記録から結果を再現して保存
   GUIで順位・盤面・提出記録を表示
```

公式簡易サーバーは最終結果を返さないため、対戦窓口が記録した提出（車種選択・行動計画とその採否）から、共通エンジンで結果を再現する。各日は最後に受理された計画を使い、受理された計画が無いチームは全車待機になる（公式簡易サーバーと同じ）。

## 本番との互換性

ソルバPCから見た対戦窓口は、本番の競技サーバーと同じように振る舞う。練習で動いた手順が、そのまま本番で動くことを確かめられる。

| 項目 | 対戦窓口の動き | 根拠 |
| --- | --- | --- |
| API | 公式の `/setting`・`/agent`・`/` だけを中継する。独自のAPIは無い | [API仕様](docs/api.html) |
| 試合の開始 | 試合開始と同時に `/setting` が問題を返し始め、そこから車種選択の時間が始まる | 本選マップに関するお知らせ |
| 試合が無いとき | 公式簡易サーバーの終了後と同じ `403 match has ended` を返す | [挙動メモ](docs/behavior.md) |
| 車種選択・各日の時間 | 公式条件（形式ごとに60/90/120秒、各日は問題の `daySeconds`） | 本選マップに関するお知らせ |
| トークン | チームNは `token-pN`。本番では配布されたトークン | — |

本番と異なるのは、`--time-limit-ms` などで制限時間を短縮したときだけ。本番のサーバーから車種選択の締切を知る手段は無いので、短縮時に限り `/setting` の応答に `Procon37-Select-Remaining-Ms` ヘッダ（車種選択の残り時間）を添える。server-bridge はこのヘッダが無ければ、本番と同じくマップの大きさから時間を決める。また、試合を続けて開くので、進行中の試合への応答に `Procon37-Match-Id` ヘッダ（試合ID）を添え、server-bridge が前後の試合を取り違えないようにする。

ソルバPC側で本番に参加する方法は [ソルバPCのセットアップ](docs/solver-pc.md#本番で使う) を参照する。公式資料は [official/](../../official/README.md) にある。

- [サーバーPCのセットアップ](docs/server-pc.md)
- [ソルバPCのセットアップ](docs/solver-pc.md)
- [CUI・GUIの操作と設定・保存形式](docs/usage.md)
- [公式簡易サーバーのHTTP挙動メモ](docs/behavior.md)
- [公式簡易サーバーの設定JSON](docs/config.md)
- [公式簡易サーバーのHTTP API仕様](docs/api.html)

## 同梱ファイル

| ファイル | 説明 |
| --- | --- |
| `procon-server` | 公式簡易サーバー（Linux版。各自で配置、git管理対象外） |
| `example.json` | 公式簡易サーバーの設定例 |
| `CREDITS.txt` | 公式簡易サーバーの依存ライブラリのライセンス |
| `desktop/` | Linux/WSL向けのアプリ登録（アイコン含む） |
| `web/` | GUIの対戦サーバー固有部。盤面・再生・GIF・画面遷移は [共通実装](../../common/README.md) を使う |
| `tests/` | 画面・HTTP・CUI・アプリ登録の試験。`fake_official.py` は試験用の簡易的な代役で、公式実装ではない |
