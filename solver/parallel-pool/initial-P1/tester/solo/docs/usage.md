# CUI・実行設定

- テスタの入口 → [../README.md](../README.md)
- セットアップ → [setup.md](setup.md)
- GUI操作 → [gui.md](gui.md)
- 履歴の比較・保存 → [history.md](history.md)

すべてのコマンドはリポジトリのルートで実行する。Rust 1.94以降が必要。

## CUI

### 基本の実行

```sh
# ソルバをビルドし、seed 0〜59の60ケースを実行
cargo run --release -p tester -- run

# 準決勝を60ケース、4ケースずつ同時実行
cargo run --release -p tester -- run 60 --format semifinal --jobs 4

# 単一seedを再現
cargo run --release -p tester -- run --seed 42 --format final
```

既定の対象ソルバは焼きなまし法による経路・補給の探索と複数日の先読みを行う。
車種選択と各日の探索予算は要求の制限時間から決め、検証・出力用の余裕を確保する。探索seedは `-- 0` のように第1引数へ指定でき、省略時は0。

- 対象ソルバの実装・設定 → [../../solver/README.md](../../../solver/README.md)

### seed・件数・形式

| 指定 | 動作 |
| --- | --- |
| `run` | 60件、seed 0〜59 |
| `run N` | N件、seed 0〜N−1。Nは1以上 |
| `run --seed S` | Sの1件。件数Nとは併用不可 |
| `--format auto` または省略 | `seed % 3` の0・1・2を1回戦・準決勝・決勝に対応 |
| `--format first / semifinal / final` | 全ケースを指定した形式で生成 |
| `--label "貪欲法 v2"` | 履歴に実行名を付ける |

件数は1〜10000。実行前に件数を検証し、範囲外はビルドや履歴作成をせず拒否する。CUI・GUI・設定JSON・履歴からの再実行に共通で適用する。

seedの範囲は0〜18446744073709551615。自動形式の60件では各形式20件になる。

| seedと形式の組み合わせ | 入力 |
| --- | --- |
| 0＋`first` | 16×16公式サンプル |
| 1＋`semifinal` | 24×24公式サンプル |
| 2＋`final` | 32×32公式サンプル |
| それ以外 | 生成器による乱数生成 |

自動形式・明示指定のどちらでも同じ規則。公式サンプルはプレイヤー数だけ1に変更し、地形・店舗・初期位置・日程などを保持する。GUIの実行・プレビューも同じ入力を使う。

- 生成器の仕様 → [generator.md](generator.md)

### 並列数

同時実行数は「同時に動かすseed数」。ソルバ内部のスレッド数は制限しない。

| ソルバの動作 | 指定 | `--jobs auto` の同時seed数 |
| --- | --- | --- |
| 内部並列なし（既定） | `--solver-serial` | 利用可能CPU数とケース数の小さい方 |
| 内部並列あり | `--solver-parallel` | 1 |
| 同時seed数を明示 | `--jobs N`（1〜256） | Nを優先 |

内部並列は自動検出しない。ソルバに合わせて指定する。利用可能CPU数は `std::thread::available_parallelism()` で取得し、実際の実行条件は `run.json` の `execution` とGUIに表示する。

ケースは、車種選択と全日程の制限時間の合計が長い順に開始し、空いた実行枠へ次を割り当てる。同じ合計ならseed順。`--time-limit-ms` の上書きも考慮する。終盤に長いケースだけが残る待ち時間を減らすための順序で、対象のseed集合・問題・制限時間は変わらない。CUI・GUIで共通し、完了後の履歴はseed順に保存する。

### 別のソルバ・時間制限

```sh
# ビルド済みの並列ソルバを使う。-- 以降はソルバの引数
cargo run --release -p tester -- run 10 --solver-parallel --no-build --solver /absolute/path/to/parallel-solver -- --threads 4

# タイプ選択・各日の回答時間を1000 msへ上書き
cargo run --release -p tester -- run 5 --time-limit-ms 1000
```

ソルバ内部のスレッド数は、ソルバ引数や設定ファイルの環境変数で指定する。

- 設定ファイルと環境変数の指定 → [usage.md](#コマンド設定)

### 問題JSONだけを出す

```sh
cargo run --release -p tester -- generate --seed 42 --format first --output /tmp/problem.json
```

`generate` のseed省略時は0、形式省略時は自動。`--output` を省略すると問題JSONを標準出力へ出す。

## 結果と判定を読む

| 出力先 | 内容 |
| --- | --- |
| 標準エラー | ケースごとのスコア・回答時間・判定 |
| 標準出力 | 実行ID、完了件数、全体・形式別のスコア合計と判定件数 |
| 保存先 | 既定は `.tester/runs/<実行ID>/` |
| 終了コード0 | 全ケースACで完了 |
| 終了コード非0 | WA・TLE・RE・CE、または中断 |

- 保存先のファイル構成 → [history.md](history.md#保存結果)

サマリーは完了・中断・失敗時に表示する。合計は完了した全ケースを対象とし、不正回答時の代替動作による得点も含む。

| 判定 | 意味 |
| --- | --- |
| AC | タイプ選択・全日程の回答を受理。得点の高さとは独立 |
| WA | 不正な回答、通信形式違反 |
| RE | ソルバの起動失敗、途中終了、通信の入出力失敗 |
| TLE | タイプ選択または日別回答が時間超過 |
| CE | ビルドの起動・実行に失敗。ケースは開始しない |

- 複数の異常がある場合のケース判定は **TLE → RE → WA → AC** の優先順。
- CEはケース単位ではなく、実行全体の0／1件として表示する。
- WA・TLE・REでも、既定のタイプ・待機計画で試合を続行して原因とリプレイを保存する。
- Ctrl+Cでバッチを中断すると、完了済みケースを残す。

## CUIで履歴を比較する

```sh
cargo run --release -p tester -- compare RUN_ID_A RUN_ID_B
```

2〜4件を指定する。

- 比較基準の変更・形式の絞り込み・JSON出力 → [history.md](history.md#cuiで比較する)

## コマンド設定

`--config FILE` でJSON設定を読み込む。

**優先順：CLIオプション → 設定ファイル → 既定値**

```json
{
  "count": 60,
  "seed": null,
  "format": null,
  "jobs": null,
  "solverParallel": false,
  "label": "貪欲法 v2",
  "timeLimitMs": null,
  "build": {
    "program": "cargo",
    "args": ["build", "--release", "-p", "solver"],
    "cwd": ".",
    "env": {}
  },
  "solver": {
    "program": "target/release/solver",
    "args": [],
    "cwd": ".",
    "env": {"RUST_BACKTRACE": "1"}
  }
}
```

上の内容を `config.json` に保存した場合：

```sh
cargo run --release -p tester -- run --config config.json
```

| 設定 | 指定方法 |
| --- | --- |
| 単一seed | `"count": null, "seed": "42"` |
| 形式自動決定 | `"format": null` |
| 同時seed数の自動決定 | `"jobs": null` |
| ビルド省略 | `"build": null` |
| ソルバ内並列 | `"solverParallel": true` |
| 環境変数 | 親プロセスから継承し、`env` で上書き |

seedはJavaScriptでの桁落ちを避けるため文字列で保存する。

コマンドはシェル展開せず直接実行する。引数は `args` の要素に分ける。`cwd` は実在するディレクトリを指定し、相対パスはテスタ起動時の作業ディレクトリ基準で解決する。

## Linuxのコマンド・アプリ登録

- `procon37` の登録・起動・更新 → [setup.md](setup.md#linuxのコマンドアプリ登録)

## GUI

- 実行から再生・GIF出力までの操作手順 → [gui.md](gui.md)

## 実行履歴を端末間で移す

- 履歴ZIPの出力・取り込み → [history.md](history.md#実行履歴を端末間で移す)

## 保存結果

- ファイル構成・JSONフィールド・ログの制限 → [history.md](history.md#保存結果)

## 非公式練習場の取り込みケース

生成器v24は、取り込みを有効にすると `seed % 4 == 3` に非公式練習場の盤面を挿入する。各サイズ5枚を用意し、既存の #937・#941・#916 を先頭に、追加分はサイズ別に固定シャッフルしている。自動形式のseed 0〜59では15枚を各1回使用し、0〜29では7枚を含む。取り込み盤面の選択は60 seed周期。形式指定時はそのサイズの盤面を `floor(seed / 12) % 5` で選ぶため、同じ12 seed区間内では繰り返す。出典・使用順・参加人数の違いは [取り込みケース](../../../common/fixtures/arena/README.md) を参照。

### 取り込みの切り替え

既定は「含める」。CUIでは `--arena-maps` で有効、`--no-arena-maps` で無効にする。`run`・`generate`・`serve` で使え、`--config` の `"arenaMaps": true` / `false` よりコマンドライン指定が優先される。無効にすると取り込み対象のseedにも通常の生成マップを使う。公式サンプル3件は引き続き利用する。

GUIでは「新しく実行」と「問題を生成」の「非公式練習場のマップを含める（4ケースに1ケース）」で切り替える。`serve --no-arena-maps` は初期値をオフにする。設定は履歴に保存し、「この設定で再実行」へ引き継ぐ。保存済みの問題・リプレイは変更しない。

```sh
# 通常の生成マップで問題を確認
cargo run --release -p tester -- generate --seed 3 --no-arena-maps --output /tmp/problem.json

# 練習場を含めるGUI（既定）
cargo run --release -p tester -- serve --arena-maps
```

比較する実行は取り込み設定も揃える。設定が異なる場合、同じseedでも問題が変わるケースは比較対象から除外される。
