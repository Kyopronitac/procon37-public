# procon37

高専プロコン競技部門「ヘキサうどん」の開発リポジトリ。
Linux / WSL / Mac に対応する。

## 審査員の皆さまへ

本チームの解法方針は以下の動画・資料で紹介しています。
ぜひご覧ください。

- [紹介動画（YouTube）](https://youtu.be/kOTjYqDpXg4)
- [スライド資料「ヘキサうどん」解法方針（PDF）](docs/slide/slide.pdf)
- [システム詳細説明書（PDF）](docs/procon37_システム詳細説明書.pdf)

## 公開にあたり

本リポジトリの公開にあたり、LLM を使った開発の戦略をまとめた資料を追加しました。

- [スライド資料「ヘキサうどん」LLM 戦略（PDF）](docs/slide-llm/slide.pdf)

## ディレクトリ構成
```bash
procon37/
├── official/            # 大会公式配布資料（募集要項・競技フォーマット・Q&A）
├── docs/                # 解法紹介資料・システム詳細説明書・参考文献
├── solver/              # Rust 製ソルバの探索実装・設定
│   └── docs/            # アルゴリズム・設定・検証資料
├── tester/              # テスタの入口
│   ├── game/            # 5人対戦テスタ・サンプル4種
│   ├── solo/            # 単独テスタ・ビジュアライザ
│   ├── server/          # 公式簡易サーバー経由の対戦
│   └── server-bridge/   # 公式対戦サーバーとソルバの変換器
├── common/              # 問題生成・判定・画面などの共通基盤
├── main.rs              # ソルバの JSON Lines 入出力
└── README.md
```



## ドキュメント

- テスタ一覧 → [tester/README.md](tester/README.md)
- 5人対戦テスタ → [tester/game/README.md](tester/game/README.md)
- 単独テスタ → [tester/solo/README.md](tester/solo/README.md)
- 対戦サーバー → [tester/server/README.md](tester/server/README.md)
- 共通テスタ基盤 → [common/README.md](common/README.md)
- 大会公式配布資料 → [official/](official/)
- 現行の試合用分散ソルバC013のアルゴリズム → [solver/docs/algorithm.md](solver/docs/algorithm.md)
- 単一スレッド第8世代（ルートの `solver`） → [探索方法](solver/docs/algorithm-generation8.md)・[設定](solver/docs/settings.md)
- 16×16試合用C013の選定・起動 → [共有周期比較と配置結果](solver/docs/distributed/resume-20261009/frequency16-20261010/result.md)
- 分散ソルバD0の本番配置・運用 → [ソルバPCのセットアップ](tester/server/docs/solver-pc.md#分散ソルバd0を3台で本番に使う)
- ソルバの保存版と世代別記録 → [solver/docs/solver-pool-experiment/README.md](solver/docs/solver-pool-experiment/README.md)

## 規約

- ルート直下のディレクトリには `README.md` を作成すること（`.github/` 等のツール/設定ディレクトリは除く）。
- ドキュメントは各ディレクトリ直下の `docs` ディレクトリに格納すること。
- 必要に応じてAGENTS.mdとCLAUDE.mdを作成すること。CLAUDE.mdの内容は「@AGENTS.md」のみとすること。
- mdファイルは第三者が読むことを想定し、分かりやすく記述すること。

日本語文書の推敲には [yomiyasu](.agents/skills/yomiyasu/SKILL.md) を利用できる。

### テスト

このプロジェクトは [対戦・単独テスタ](tester/README.md) を提供している。通常の評価には5人対戦の `game`、単独動作の確認には `solo` を利用する。

AI向けの操作スキルは `$procon37-tester`。

- AI向けのテスタ操作手順 → [.agents/skills/procon37-tester/SKILL.md](.agents/skills/procon37-tester/SKILL.md)

- 簡易的な実行・比較に適宜利用すること
- CUI を提供しているので、利用すること
- ビジュアライザを備えた GUI も、必要に応じて利用すること
- 実行時は、変更内容・比較上の役割・テスト範囲が分かる `--label` を付けること（例：`補給計画改善前・game基準・フル30件`）。
- 比較では種類数・日別種類数・玉数の合計と、以前に対する勝・同点・敗の件数を確認すること。PRには下記の形式で記載する。

重要な変更後、PR時、その他必要と判断した時は、以下のフルテストを行うこと。

対象ソルバが内部で単一スレッドの場合、フルテストの件数は次のとおり。

| モード | 件数 | seed | 形式 |
| --- | ---: | --- | --- |
| solo | 60件 | 0〜59 | 自動選択（各形式20件） |
| game | 30件 | 0〜29 | 自動選択（各形式10件） |

現時点では、特段の指定がない限り対象ソルバは単一スレッドとして扱う。「単一スレッド」は対象ソルバ内部の実行方式を指し、テスタの同時ケース数（`--jobs`）とは別。内部並列・分散ソルバの件数は従来どおり60件（seed 0〜59、形式自動選択）とする。

- 件数は明示する。単一スレッドのgameは `cargo run --release -p game -- run 30`、soloは `cargo run --release -p tester -- run 60` を使い、以下の制限時間・保存先と適切な `--label` を指定する。
- 本番と同じ、デフォルトの制限時間で実行する
- `--output-dir` で独自の保存先を指定せず、既定の保存先（gameは `.game/runs/`、soloは `.tester/runs/`）に履歴を残すこと。

### コミットの命名規則

[Conventional Commits](https://www.conventionalcommits.org/ja/) に従い、説明は日本語で記述する。
```php-template
<type>: <変更内容を簡潔に>
```

| type       | 用途                |
| ---------- | ----------------- |
| `feat`     | 新機能の追加            |
| `fix`      | バグ修正              |
| `docs`     | ドキュメントのみの変更       |
| `refactor` | 挙動を変えないコード整理      |
| `test`     | テストの追加・修正         |
| `chore`    | ビルド・補助ツール・依存などの雑務 |

### PR の書き方

以下のテンプレートに沿って記述する。ソルバ変更時のみ「フルテスト結果」を記入する。

```markdown
## 📋 実施タスク

- {issue の URL}
- この PR で何を実装・修正したのか簡潔に記述

## 🧪 フルテスト結果

| 指標 | 以前（ブランチ開始時） | 変更後 |
| --- | ---: | ---: |
| 種類数の合計 | {値} | {値} |
| 日別種類数の合計 | {値} | {値} |
| 玉数の合計 | {値} | {値} |

以前に対して **{A}勝 {B}同点 {C}敗**（変更後から見た勝敗）。

## 🔍 レビューしてほしいこと

- 主な変更点を箇条書きで
  - 例: 新しい API エンドポイント `/v1/users` を追加
  - 例: `UserController` の `show` メソッドをリファクタリング

## 備考

- なにか他に伝えるべきことがあれば記述
```

### Review 方法

- AI による Review を行う
- 必要に応じて人間による Review を行う
- Review によりソルバを変更した場合、フルテストを再実施すること
- フルテストには時間が掛かるので、ソルバへの Review は重要度の高いもの以外は行わない
