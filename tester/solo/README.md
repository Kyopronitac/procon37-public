# テスタ・ビジュアライザ

ヘキサうどんのソルバを、問題生成から実行・採点・リプレイまで確認する開発ツール。**自分1プレイヤー**の試合を扱い、GUIとCUIで同じRust実装を使う。

実装は [共通基盤](../../common/README.md) と共有する。5人の対戦は独立した [game](../game/README.md) を利用する。

## 起動と実行

Rust 1.94以降を用意し、リポジトリのルートで実行する。

```sh
# GUIを起動。表示されたURLをブラウザで開く
cargo run --release -p tester -- serve

# CUIでソルバをビルドし、60ケースを実行
cargo run --release -p tester -- run
```

GUIでは「新しく実行」→「実行」→結果の「再生」と進む。
既定の対象ソルバは、焼きなまし法による経路・補給の探索と複数日の先読みを使用するRust実装。車種選択と各日の探索予算は要求の制限時間に基づき、検証・出力用の余裕を確保する。

- 対象ソルバの実装・設定 → [../solver/README.md](../../solver/README.md)

| 既定値 | 内容 |
| --- | --- |
| ケース | seed 0〜59の60件 |
| 形式 | 1回戦・準決勝・決勝を各20件 |
| ビルド | バッチ開始前にソルバを一度だけreleaseビルド |
| 並列数 | 利用可能CPU数とケース数から自動決定 |
| 保存先 | `.tester/runs/`。実行を終えても再生・比較可能 |

Linux / WSLでコマンド・アプリ一覧に登録する場合：

```sh
bash tester/solo/desktop/setup.sh
solo
```

> テスタや生成器を変更した後は `solo --restart` で再ビルド・再起動する。保存済み履歴の問題は変更されない。

## 現在の生成ケース

GUI・CUI共通の生成器は **hex-udon-v24**。既定では4 seedに1件（seed 3・7・11…）、[非公式練習場から取り込んだ盤面](../../common/fixtures/arena/README.md)を挿入する。各サイズ5枚を用意し、自動形式のseed 0〜59では全15枚を各1回使う。GUIの実行設定・問題プレビュー、CUIの `--arena-maps` / `--no-arena-maps` で取り込みを切り替えられる。公式サンプル3件と、それ以外の生成ケースは従来の幅広い条件20%・制約が厳しい条件80%で抽選する。
厳しい条件では初日のステップ数を公式の最小値に合わせ、幅広い条件では大きな初期値も残す。

v19の2026-09-21の評価では、基準ソルバ・既定55秒で日別種類数の上限未達率を計測した。

| 集合 | 上限未達 | 未達率 |
| --- | ---: | ---: |
| 調整用：seed 3〜59 | 26/57件 | 45.6% |
| 確認用：seed 3000〜3119 | 58/120件 | 48.3% |

両集合とも目標の45〜55%に収まった。全177件がAC・試合全体で全系列獲得・フォールバックなし。
入力が変わった23件を再実行し、問題全体が一致する154件はv17の結果を再利用した。確認用も既存seed集合の再評価である。

- 条件の抽選方法 → [docs/generator.md](docs/generator.md#条件の混合)
- 計測条件・形式ごとの内訳 → [docs/generator-calibration.md](docs/generator-calibration.md#難易度の計測結果)

## ドキュメント

- AI向けの操作スキル → [procon37-tester](../../.agents/skills/procon37-tester/SKILL.md)
- 環境構築・GUIの起動と更新 → [docs/setup.md](docs/setup.md)
- CUIの使い方・並列数とソルバの設定 → [docs/usage.md](docs/usage.md)
- GUIでの実行・再生・GIF保存 → [docs/gui.md](docs/gui.md)
- 履歴の比較・移動・削除と保存形式 → [docs/history.md](docs/history.md)
- ソルバの標準入出力・通信仕様 → [docs/protocol.md](docs/protocol.md)
- 問題の生成方法とサンプルとの対応 → [docs/generator.md](docs/generator.md)
- 生成ケースの難易度・上限未達率 → [docs/generator-calibration.md](docs/generator-calibration.md)
- テスタの設計・処理の責務 → [docs/tester-design.md](docs/tester-design.md)
- 依存ライブラリとライセンス → [docs/dependencies.md](docs/dependencies.md)

## ディレクトリ

```text
tester/solo/
├── src/       # 単独CUIの入口・共通クレートの再公開
├── web/       # 互換モジュール（実装は ../../common/web）
├── desktop/   # Linux/WSLの起動とアプリ登録
├── tests/     # CUI・画面・GIF・起動スクリプトの検証
└── docs/      # 操作ガイドと仕様
```

- モジュールごとの担当 → [docs/tester-design.md](docs/tester-design.md#実装一覧)

## 検証

通常利用にはPython / Node.jsは不要。プロセスを扱うRustテストにはPython 3、画面・GIFテストにはNode.js、GIFの検証にはPillowを使う。

| 確認するもの | コマンド |
| --- | --- |
| Rustの整形 | `cargo fmt --all --check` |
| Rust・CUI・ソルバ通信 | `cargo test --workspace` |
| Rustの静的検査 | `cargo clippy --workspace --all-targets -- -D warnings` |
| JavaScriptの構文 | `node --check common/web/app.js` |
| 画面の動作 | `node --test tester/solo/tests/web.test.mjs` |
| GIFの色・時間・ループ | `python3 tester/solo/tests/gif_test.py` |

Linuxの起動・登録テストは、releaseビルド後に実行する。ローカルポートを使用する。

```sh
cargo build --release -p tester
python3 tester/solo/tests/desktop_test.py
```
