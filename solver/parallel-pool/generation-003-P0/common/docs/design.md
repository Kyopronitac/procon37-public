# 共通基盤の構成

| 場所 | 責務 |
| --- | --- |
| `src/model.rs` | 問題・通信・車両・得点・リプレイの型 |
| `src/generator.rs` | 公式サンプル読み込み、seedからの生成。`generate_for_players` は生成後の `players` だけ変更。`generate_with_arena_maps` / `generate_for_players_with_arena_maps` で取り込みを切り替え |
| `src/engine.rs` | 1チームの計画検証と原子的シミュレーション、交通量判定 |
| `src/process.rs` | ビルド、JSON Lines通信、期限、ログ、プロセスグループの終了 |
| `src/runner.rs` | 設定・並列バッチ・中断・保存 |
| `src/competition.rs` | 5人の同期進行、他チームの観測、交通量合算、順位、対戦リプレイ検証 |
| `src/archive.rs` | ZIPの安全な入出力と保存内容の再計算 |
| `src/comparison.rs` / `report.rs` | 比較対象の照合、CUI/GUI共通の集計 |
| `src/cli.rs` / `server.rs` | 共通コマンド・ローカルHTTP API |
| `web/` | 盤面・履歴・再生・GIF・画面遷移。盤面の描画は `board.js`、ズーム・ドラッグ操作は `viewer.js` にまとめ、対戦サーバーも共有する。対戦固有部は `tester/game/web/`、対戦サーバー固有部は `tester/server/web/` を使用 |
| `desktop/` | 起動・登録処理。各アプリのwrapperから名前・保存先を指定 |

## シミュレーション

`Game::play_day_with_traffic(plan, traffics)` は当日の道路状態を受け取る。検証と実行は状態の複製に対して行い、成功時にチーム状態を更新して `DayReplay` を返す。不正計画では状態を変更しない。日ごとの滞在数は `last_stays()` で取得できる。

単独テスタの `play_day` は自分の過去2日分の滞在数から道路状態を作って同じ関数を呼ぶ。対戦では5チームの滞在数を合算し、`traffic_for(problem, history, 5)` に渡す。閾値を5倍した整数比較を使い、平均時の丸めによる境界の差を避ける。

ソルバ内のシミュレーションには `Game::set_observed_traffic(traffics)` で当日の観測道路状態を設定できる。設定後は `traffic()` と `play_day()`、計画検証用の複製にも反映する。計画が却下された場合は保持し、1日を正常に進めると解除する。観測がまだない先読みの日程では、自チームの滞在数による従来の予測を使う。単独テスタでは設定せず、従来の計算を保つ。

5プロセスへの車種選択要求と日ごとの要求は `JoinSet` で並行処理する。全員の回答が揃うか各期限を迎えてからシミュレーションし、次の日に進む。個別の通信エラーでは該当チームだけ代替動作を使う。中断でタスクを終了すると、保持中の子プロセスもグループ単位で終了・回収される。

## バッチの実行順

単独・対戦、CUI・GUIとも、車種選択と全日程の制限時間の合計が長いケースから開始する。`--time-limit-ms` がある場合は「指定時間 ×（日数＋車種選択1回）」で見積もる。同じ見積もりならseedの小さい順。対戦の5人は並行して回答するため、見積もりを5倍にはしない。

実行枠が空くたびに次のケースを割り当てる。並列数の自動設定と `--jobs` は従来どおりで、内部並列指定も尊重する。開始順を決める際は生成器から日程だけを取得し、盤面と子プロセスは実行枠が空いてから作る。生成器は日程取得と実際の問題生成で同じ処理を使い、seedに対応する問題を変えない。

制限時間近くまで探索するソルバでは、長いケースが最後に残って実行枠が空く時間を減らせる。すぐに回答するソルバでは実時間とのずれがあり、1並列では順序変更による短縮はない。進捗は完了順、確定した履歴はseed順で保存する。

## 互換性

`tester/solo/src/lib.rs` は本クレートを再公開する。単独の `Problem::validate()` は引き続き1人を要求し、対戦側は `validate_players(5)` を呼ぶ。既存の `generate()` はそのまま1人の問題を返す。`RunConfig.arena_maps`（JSON: `arenaMaps`）は既定で `true`。CUIの `--arena-maps` / `--no-arena-maps`、GUIの実行設定・問題プレビューから変更でき、実行時の問題生成と開始順の見積もりに共通して反映する。

単独履歴は `run.json` schema 3、リプレイ schema 1を継続する。対戦用の省略可能フィールドは単独履歴へ出力しない。古い履歴の判定読み替え・比較・ZIP入出力も維持する。

両アプリは同じサーバー実装を使うが、入口HTML・既定設定・ポート・履歴保存先・launcher識別子を分ける。別形式のZIPを取り込むことはできない。

対戦サーバー（`tester/server`）は公式簡易サーバー経由で実行するため、独自のHTTP APIを持つ。構成は `src/server.rs` と同じで、盤面の描画（`board.js`）と盤面操作（`viewer.js`）、画面部品（`style.css`・`navigation.js`・`playback.js`・`record-stats.js`・GIF）は本クレートの `web/` をそのまま配信する。`tester/game/web/board.js`・`tester/server/web/board.js` はNode.jsのテストが `match-ui.js` を読み込むための転送で、配信はしない。履歴は `run.json` schema 3・対戦リプレイ schema 2 と同じ形で保存し、比較・集計には `comparison`・`report` を使う。画面遷移のプレイヤー番号は対戦サーバーの最大16チームまで保持し、各画面ではその画面の参加人数の範囲に収める。

## 検証

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
node --check common/web/app.js
node --test tester/solo/tests/web.test.mjs tester/solo/tests/viewer.test.mjs tester/solo/tests/board.test.mjs tester/game/tests/web.test.mjs tester/server/tests/web.test.mjs
python3 tester/solo/tests/gif_test.py
cargo build --release --workspace
python3 tester/solo/tests/desktop_test.py
python3 tester/game/tests/desktop_test.py
python3 tester/game/tests/server_test.py
python3 tester/game/tests/samples_test.py
python3 tester/server/tests/server_test.py
python3 tester/server/tests/desktop_test.py
```

プロセス試験にはPython 3、GIF試験にはNode.jsとPillow、サーバー・デスクトップ試験にはlocalhostでのポート利用が必要。単独の画面テスト用モジュールは `tester/solo/web` から共通実装を再公開する。
