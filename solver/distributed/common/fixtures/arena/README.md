# 非公式練習場から取り込んだ回帰ケース

サイズ別に5件、計15件の問題JSONを保存している。既存の公開試合3件に、ユーザーが2026-10-04に手動で提供した12件を追加した。元の地形、店舗、在庫、初期位置、日程、燃料、交通量基準は保持する。

## 出典と重複確認

既存3件は「終焉の使徒」が低順位かつ0点でなかった公開試合から選んだ。2026-10-04に公開APIで成績と問題の一致を確認している。

| 形式 | 出典 | 当時の順位 | 種類数 / 日別種類数 / 玉数 | 最初に使うseed |
| --- | --- | ---: | --- | ---: |
| 1回戦・16×16 | [試合 #937](https://procon37arena.online/arena/matches/937) | 6 / 10 | 13 / 130 / 317 | 3 |
| 準決勝・24×24 | [試合 #941](https://procon37arena.online/arena/matches/941) | 6 / 10 | 16 / 80 / 258 | 7 |
| 決勝・32×32 | [試合 #916](https://procon37arena.online/arena/matches/916) | 3 / 10 | 28 / 140 / 363 | 11 |

追加分の `pasted-{サイズ}-{番号}.json` はサイズ別の貼り付け順で命名している。試合番号・URL・成績は提供されていないため、既存3件と同じ条件で選んだ問題とは扱わない。出典区分、ファイルのSHA-256、固定した使用順は [sources.json](sources.json) に保存する。

最初の貼り付け11件と既存3件には重複がなかった。その後に届いた24×24は `pasted-24-02.json` と完全に一致したため、追加しなかった。代わりに届いた24×24を `pasted-24-04.json` として保存した。採用した全15件について、開始時刻・参加人数を正規化した問題全体、盤面＋店舗＋車両、地形だけの3通りで重複がないことを確認した。

## 使用順とseed

生成器 **hex-udon-v24** は、取り込みが有効な場合に `seed % 4 == 3` のケースをサイズ別の盤面群から選ぶ。既存の1件を各群の先頭に残し、追加4件は `SHA-256("2026-10-04:{width}:{file}")` の昇順で固定シャッフルした。試合ごとに再シャッフルすることはなく、同じseed・形式・設定なら同じ問題になる。

自動形式のseed 0〜59では、次の15枚を各1回だけ使う。

| 順番 | 16×16（1回戦） | 24×24（準決勝） | 32×32（決勝） |
| ---: | --- | --- | --- |
| 1 | seed 3：[match-937.json](match-937.json) | seed 7：[match-941.json](match-941.json) | seed 11：[match-916.json](match-916.json) |
| 2 | seed 15：[pasted-16-01.json](pasted-16-01.json) | seed 19：[pasted-24-03.json](pasted-24-03.json) | seed 23：[pasted-32-04.json](pasted-32-04.json) |
| 3 | seed 27：[pasted-16-04.json](pasted-16-04.json) | seed 31：[pasted-24-02.json](pasted-24-02.json) | seed 35：[pasted-32-03.json](pasted-32-03.json) |
| 4 | seed 39：[pasted-16-02.json](pasted-16-02.json) | seed 43：[pasted-24-01.json](pasted-24-01.json) | seed 47：[pasted-32-02.json](pasted-32-02.json) |
| 5 | seed 51：[pasted-16-03.json](pasted-16-03.json) | seed 55：[pasted-24-04.json](pasted-24-04.json) | seed 59：[pasted-32-01.json](pasted-32-01.json) |

seed 0〜29では7枚、0〜59では15枚を含む。取り込み盤面の選択は60 seed周期で、seed 63・67・71から既存の先頭盤面へ戻る。形式を明示した場合も同じseed・サイズの問題は自動形式と一致する。インデックスは `floor(seed / 12) % 5` なので、形式を固定して連続実行した場合は同じ12 seed区間内の取り込み枠で同じ盤面を使う。15枚を重複なく評価するには形式を自動選択する。

生成時には過去の絶対開始日時を `startsAt: 0` にし、参加人数をsoloは1、gameは5、serverは指定人数に合わせる。元の問題は10人対戦であり、既存試合の当時の順位をそのまま再現するものではない。#937は回答時間による順位差を含む。

問題JSONはビルド時に実行ファイルへ埋め込む。生成・実行時に練習場サイトへアクセスしない。公式サンプル3件と通常の生成手順は従来どおりで、保存済み履歴の問題・リプレイも変更しない。

## 問題の生成

```sh
cargo run --release -p game -- generate --seed 3 --output /tmp/arena-937.json
cargo run --release -p game -- generate --seed 7 --output /tmp/arena-941.json
cargo run --release -p game -- generate --seed 11 --output /tmp/arena-916.json
cargo run --release -p game -- generate --seed 15 --output /tmp/arena-extra-16.json
```

GUIの問題プレビュー、通常のCUI/GUI実行でも同じseedで利用できる。取り込みは既定で有効で、CUIの `--arena-maps` / `--no-arena-maps`、GUIの「非公式練習場のマップを含める」、設定JSONの `arenaMaps` で切り替える。無効にすると取り込み対象のseedも通常の生成マップになる。
