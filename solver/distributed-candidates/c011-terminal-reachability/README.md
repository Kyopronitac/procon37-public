# C011: 翌日 steps による終端配置評価（未採用候補）

D0 直系の独立候補。C006 評価終了後、CPU1・-j1 で release build、unit 17 件（追加6＋既存11）、localhost network 2 件が初回ですべて通過した。テスト子プロセスは残存なし。性能測定・remote 操作・server 評価は未実施。

- 親 D0 初期採用 commit: `5d6c740962a1f3e20e77e13df243078df1d29ad2`
- 親 source SHA-256: `61b56e0ae79cfae1ca86ddc62e8e38bb94a9410871508cadbd783a9b9a469d65`
- 実装前の固定設計: [終端配置評価案](../../docs/distributed/resume-20261009/c011-terminal-reachability-design.md)
- ソース識別: [source-manifest.json](source-manifest.json)、実行記録: [validation.json](validation.json)

## 固定した初期式

在庫が正の店舗のブランド集合を B、翌日 steps を T とする。既存 mode0 potential の時間から、各 node に距離 `<=floor(T/2)` の near mask と `<=T` の full mask を一度作る。収集車の実際の `Outcome.endnodes` について OR した N/F から、以下を全形式共通で評価する。

```
far  = popcount(B & ~F)
thin = popcount(B & F & ~N)
penalty = -min(12, 4*far + thin)
```

既取得ブランドも翌日の種類数に意味があるため対象に含む。同一ブランドは OR でまとめ、在庫0店舗と補給車は含めない。交通は現在の Graph、燃料は D0 の既存評価だけを使う。翌日なし/T=0/不正な車種構成/短時間 fallback では無効。これは探索誘導の仮説であり、翌日の到達保証ではない。

## 接続と診断

`PROCON37_C011_TERMINAL_REACH=0` で診断用に off、それ以外・未指定は on。通常の D0 launcher は env -i を使うため、変更せず default on として配置できる。off の remote 比較を行う場合には環境が launcher まで届く設定が別途必要であり、事前に同条件の基準を再評価する。

Graph とその経路構築、予算、RNG、通信、thread 数、Snapshot は変更しない。既存 coverage_weight は0のまま fast beam を維持する。coverage/future/archive が非0の場合にはこの補助項を作成しない。

`plan_inner` に実日 primary を示す明示引数を追加し、公開 plan の本探索だけ true、selection/select_role/forecast/probe は false とする。要求限定の mask 表は対象 Graph の Rc を保持し、評価時に参照先の同一性・day・steps・supplier・agent数を確認する。同じ値の別 Graph も対象にしない。入口/restore/終了で消去し、古い node 番号を再利用しない。

全車項は route memo と route_values の確定後に1回だけ加える。fast beam は zero.value に含まれる全車減点を fixed から外し、他車 mask の OR に各 prefix の実際の node を加えて再計算する。fleet_pool の個別候補・上界は D0 のままで、候補の最終 full evaluate で新項を扱う。

canonical の直前に表を消去する。raw_value と rollout の式は D0 のまま、plan_inner の choice/last 値にも選択済み候補の代理減点が残らないよう除く。毎 primary 終了時の c011 terminal 行に active/build_us/far/thin/penalty を記録する。short/role fallback は理由付き active=0 行を出す。既存 g007 行の score は探索中の誘導項込み、canonical/提出比較は誘導項なしになる。

## 実施した検証

`terminal_reach.rs` の6テストは、閾値直前/一致/直後・T=1/0・同ブランド/在庫0、独立した車×店舗走査との全小盤面組合せ照合、補給車/既取得ブランド非依存、Graph identity/restore/off/phase/fallback、全評価と fast beam 局所式の一致・途中経路終端・memo再利用、canonical/forecastの分離、実生成3形式での合法提出を対象にする。

既存ネットワーク2件は維持し、CPU1上で5つの eager Graph worker を動かす reconnect test の要求だけ900→1800msにしている。これはC010検証時に判明したローカル実行条件の調整で、ソルバ本体の予算は不変。性能や当日成績を保証するテストではない。

再現する場合は repo cwd で `. .server/toolchain/env.sh`、`taskset -c 1 cargo test --release --offline -j 1 --manifest-path solver/distributed-candidates/c011-terminal-reachability/Cargo.toml -- --test-threads=1` を実行する。localhost socket の権限と親の実行許可に従う。source と binary の識別、子 PID の終了は validation.json に記録済み。

root と evaluation_audit の読取レビューでは重大指摘なし。レビュー対象ソースを変えずに全テストが通過した。性能・対戦成績は未評価であり、候補は未採用。
