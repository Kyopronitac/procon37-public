# 本番運用パッケージの独立レビュー

2026-10-10 05:10 JST。実装担当者とは別の担当者が `tester/server-bridge/competition/` を確認した。実行中の候補、評価ハーネス、固定相手は変更していない。保護対象ディレクトリの実装は参照していない。

## 結論

確認した最終版に配置を妨げる未解決事項はない。実機での配置後照合と `probe` は評価プロセスの停止後に行う。本レビューだけでは当日の競技サーバーへの接続成功を保証しない。

- `runtime_base.py` は従来の運用コントローラー、`console.py` は従来のログフィルターとバイト単位で一致する。
- パッケージは評価証拠に含まれる実行ファイルの SHA-256 を要求し、再コンパイルせずコピーする。bridge は評価済みの `e02fe74277a4178df5f0772bb3ad95af257fef4317df90841052e93ec83f2d90` に固定する。
- 3台は wire protocol 4 と認証トークンで接続し、32・16・32論理スレッド、worker offset 0・31・46、`cross-elite=on` を使用する。VOID1 が探索・集約・提出を担当する。bridge と coordinator の両方に streaming を指定する。
- 車種選択と各日の提出機会は5・15・25・35・45・55秒。同一回答の再送省略は評価済み bridge が行い、公式の制限時間は変更しない。
- `SERVER`・`TOKEN` 未設定でも `deploy`・`activate`・HTTPを使わない `probe` は可能。実際の `start` は未設定時に SSH や問題サーバーへの接続前に拒否する。preflight はソルバや bridge の `--help` に依存しない。
- 編集用の `~/procon37-competition/run.sh` は immutable release の外にあり、当日のURL・トークン編集で manifest は変化しない。release は別ディレクトリ、worker は評価用39370と異なる39371を使う。
- 起動・停止はUID、PID、start ticks、boot ID、session、cwdを照合する既存管理処理を再利用する。別配置のプロセスを検知した場合は起動を拒否し、既存D0や進行中の評価を停止しない。トークンとSSH認証ファイルは配布パッケージに含めず、配置先の private データとして保持する。

## 指摘と解消

既存の base 実装は部分起動済み worker を再利用できるが、終了時にはその起動操作で作成した worker だけを停止する。このため本番で想定する「Ctrl+Cで3台停止」と停止対象が一致しない場合があった。competition wrapper が1台または2台だけ稼働する状態を起動前に拒否し、`run.sh stop` 後の再起動を案内する修正を実装担当者が加えた。全停止からの起動と全3台起動済みの処理を維持し、base のバイト列は変更していない。

既存ログフィルターは任意のエラー本文を端末へ通さないため、competition専用 `progress.py` が `partial_cluster` と `endpoint_unset` の2種類だけを終了理由として表示する。他のログ処理・シグナル転送は既存フィルターへ委譲する。

## 検証

レビュー担当者が次を実行し、11件すべて成功した。SSH、実ソルバ、対戦サーバーを起動しないローカルfixture検証であり、CPUは1・3・5・7に限定した。

```sh
taskset -c 1,3,5,7 python3 -m unittest discover -s tester/server-bridge/competition/tests -v
```

検証対象は起動引数、実行ファイルの同一性と改変拒否、評価証拠必須、入口編集とmanifestの分離、認証情報未設定時の停止、wire 4認証、認証失敗時のprobe後片付け、部分起動拒否、表示理由の限定、既存ファイルとのバイト同一性。

## 確認版の識別情報

| ファイル | SHA-256 |
| --- | --- |
| `competition/distributed.py` | `5c252667e7d24fe2da2b7fb8516812580b671470be8a1a0f854024b16b4d4724` |
| `competition/progress.py` | `2cd0631299adf89ecb3e3c4b913fa0e6856bcbb78a219cd11d12787268a2a94f` |
| `competition/run.sh` | `39a52a1416436205213bf7117a47c63365169bf4a4a7569391f041a2f2b310df` |
| `competition/runtime_base.py` | `7ab56ef168dbb703b9c97f0f62226d93d5bbd0cb79df8d9939c0e43099c4bab9` |
| `competition/console.py` | `253ece9c44c1b2e7a1cce189740ecc913566513045cbb1759be59fd173d5626f` |
| `competition/tests/test_competition.py` | `61b1c13031be579ede61362c4a31537349533364a8ca9fab8c0d8bb8c64200e3` |

同時進行中の有限評価ハーネス `solver/distributed-lab/frequency_matrix.py` の凍結ハッシュは `aa46f22ca4314411a7614d3a11e17f52f7c4ebdd6c5eb25d4fff7d0e5eed8d53`。これは本レビュー担当者自身の作成物であり、本番パッケージの独立レビューには含めない。
