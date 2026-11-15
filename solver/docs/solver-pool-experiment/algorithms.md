# 保存ソルバのアルゴリズム

[実験結果まとめ](README.md) / [ビルドと実行](reproduction.md)

第1〜7世代は最下位を独立設計のソルバで置き換えた従来のPool育成によるものです。第8世代はその方式とは異なり、第7世代の継続改善から選定した派生版です。世代ごとに相手の実装と配置が変わるため、順位合計を横並びの強さの順位として使いません。

| 世代 | 保存ソース | 実行ファイル | 主な設計 | 詳細 |
| --- | --- | --- | --- | --- |
| [solver01](#solver01) | [solver01.rs](solver01.rs) | `pool-solver-01` | 区間経路・移動補給・車両別探索 | [アルゴリズム](solver01/algorithm.md) / [設定](solver01/settings.md) / [検証](solver01/validation.md) |
| [solver02](#solver02) | [solver02.rs](solver02.rs) | `pool-solver-02` | 巡回と移動合流の同時最適化 | [アルゴリズム](solver02/algorithm.md) / [設定](solver02/settings.md) / [検証](solver02/validation.md) |
| [solver03](#solver03) | [solver03.rs](solver03.rs) | `pool-solver-03` | 翌日試走と計画引き継ぎ | [アルゴリズム](solver03/algorithm.md) / [設定](solver03/settings.md) / [検証](solver03/validation.md) |
| [solver04](#solver04) | [solver04.rs](solver04.rs) | `pool-solver-04` | 巡回経路と補給の共同計画 | [アルゴリズム](solver04/algorithm.md) / [設定](solver04/settings.md) / [検証](solver04/validation.md) |
| [solver05](#solver05) | [solver05.rs](solver05.rs) | `pool-solver-05` | 車両別ビーム・補給経路・2日評価 | [アルゴリズム](solver05/algorithm.md) / [設定](solver05/settings.md) / [検証](solver05/validation.md) |
| [solver06](#solver06) | [solver06.rs](solver06.rs) | `pool-solver-06` | 補給時刻・巡回計画・2日共同探索 | [アルゴリズム](solver06/algorithm.md) / [設定](solver06/settings.md) / [検証](solver06/validation.md) |
| [solver07](#solver07) | [solver07.rs](solver07.rs) | `pool-solver-07` | 訪問列の焼きなまし・ビーム探索・翌日の種類数確認 | [アルゴリズム](solver07/algorithm.md) / [設定](solver07/settings.md) / [検証](solver07/validation.md) |
| [solver08](#solver08) | [solver08.rs](solver08.rs) | `pool-solver-08` | 第7世代の継続改善・前日巡回の逆順再利用・他車位置からの道路予測 | [アルゴリズム](solver08/algorithm.md) / [設定](solver08/settings.md) / [検証](solver08/validation.md) |

各世代の説明は保存した実装と当時の世代記録に基づきます。出典は [source-manifest.json](source-manifest.json)、保存方法は [consolidation.md](consolidation.md) に記録しています。第8世代の生成方法と前後比較は [登録記録](records/generation-008.md) を参照してください。

<a id="solver06"></a>

## 第6世代：正式昇格版

第1世代を置き換えるため、旧実装を参照せず公開仕様から新規設計したソルバです。正式評価の順位合計64で単独首位となった昇格時の実装を保存しています。[アルゴリズム](solver06/algorithm.md) / [正式評価](solver06/validation.md)。

<a id="solver07"></a>

## 第7世代：正式昇格版v37

第5世代を置き換えるため、公開仕様から新規設計したソルバです。正式評価の試行3で順位合計65となり、単独首位で昇格したv37を保存しています。第8世代はこの世代からの継続改善です。[アルゴリズム](solver07/algorithm.md) / [正式評価](solver07/validation.md)。

<a id="solver08"></a>

## 第8世代：第7世代の継続改善版

第7世代の候補v117を変更せず保存しています。自作の経路探索・補給計画を継続改善し、前日の巡回を逆順にした初期候補や、公開される他車位置を根にした最短経路木による道路予測を使用します。従来のPool方式による独立した新規開発ではありません。[詳細](solver08/algorithm.md) / [生成方法・評価結果](records/generation-008.md)。

<a id="solver01"></a>

## 第1世代：区間経路・移動補給・車両別探索

区間ごとの経路選択、移動する補給車との合流、焼きなましと車両別ビーム探索を組み合わせる。翌日の行動余地を残すため、日末の燃料余力も評価する。 [詳細](solver01/algorithm.md) / [正式評価](solver01/validation.md)。

<a id="solver02"></a>

## 第2世代：巡回と移動合流の同時最適化

巡回順、補給地点、合流順、日末の燃料余力をまとめて探索する。第1世代とは独立に設計された。 [詳細](solver02/algorithm.md) / [正式評価](solver02/validation.md)。

<a id="solver03"></a>

## 第3世代：翌日試走と計画引き継ぎ

翌日を試走して当日の選択を評価し、予測した計画を次の要求へ引き継ぐ。複数の待合地点・区間経路と保守的な交通予測を組み合わせる。 [詳細](solver03/algorithm.md) / [正式評価](solver03/validation.md)。

<a id="solver04"></a>

## 第4世代：巡回経路と補給の共同計画

巡回車の経路と補給車の動きを共同で計画し、翌日の見通しも評価に加える。第4世代の候補3が昇格した。 [詳細](solver04/algorithm.md) / [正式評価](solver04/validation.md)。

<a id="solver05"></a>

## 第5世代：車両別ビーム・補給経路・2日評価

道路経路を前計算し、補給車の行動と巡回車ごとのビーム探索を組み合わせる。候補の局所改善と翌日試走で日別計画を選ぶ。 [詳細](solver05/algorithm.md) / [正式評価](solver05/validation.md)。
