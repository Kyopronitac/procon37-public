# C010: PC 間 elite 共有（未採用候補）

採用済み D0 を独立コピーし、実日の primary 探索の reserve15 群に、探索途中の PC 間候補共有を追加する。C006 の予算変更、C007 の経路表圧縮、C008 の機内 Graph 共有、GPU は含まない。

CPU1・-j1 で release build、unit 16 件、localhost network 3 件が通過した。実送受信と合法な終端 Result を確認し、全テスト子プロセスは終了済み。remote 配置・server 評価は未実施で、正式採用や性能改善を意味しない。

- 親: D0 / 初期採用 commit `5d6c740962a1f3e20e77e13df243078df1d29ad2`
- 親 source SHA-256: `61b56e0ae79cfae1ca86ddc62e8e38bb94a9410871508cadbd783a9b9a469d65`
- コピー範囲: D0 の Rust workspace source、Cargo files、公開 common fixture、既存 network test の 32 ファイル。tools/runtime/target は除外。
- 現候補の識別: [source-manifest.json](source-manifest.json)。source と local binary の SHA、実行記録は [validation.json](validation.json) に保存した。

## 動作

3 台とも `--cross-elite on` を付ける。同じ binary の `--cross-elite off`（既定値）で共有を無効にできる。network protocol は VERSION 2 で、認証時と Ready 時に on/off の一致も確認する。parallel mode で on は受け付けない。

既存の 31/15/31 search worker、control thread、ポート、認証、request budget、最終 Candidate 選定・state 同期は維持する。実日 Plan の `(global_worker / 2) % 7 == 3` のみ、future_weight=0 / coverage_weight=0 / reserve_weight=15 のとき共有する。Select、forecast rollout、probe には接続しない。

各ノードは既存の 16,384 iteration ごとの elite publication から、そのノードの改善済み候補を最大 1 件保持する。各方向 500 ms ごとに最新候補を最大 1 件送る。root は自分と他ノードの候補を中継し、origin/sequence は保持する。共有の受信は終端 Result ではなく、completed / remote_results / pending を変更しない。実際の取込は既存の restart 境界だけで、受信側が自分の Graph で再評価して改善した場合に限る。

## 状態と境界

packet は session/request/input digest、origin/global worker/sequence、full context、Solution、送信側 value を持つ。full context は schema、Problem 全値、観測 agents/day、steps/supplier/seen_brands、Graph の順序・重複を保持した positions、targets、durations/costs。primary evaluate が参照する全データを全値比較し、64-bit digest だけで受け入れない。Graph の経路・cell_node・spot_at・mobility は上記から D0 の固定ルールで決まる。forecast_other と beam/temperature は primary の評価値に使わない。

有界 mailbox は node ごと 1 件、合計最大 3 件。未 bind の受信も最大 2 origin までで、Graph ができるまで再送・取込せず、bind 時に完全一致を確認する。旧要求の bus は close され、探索が残っても新要求へ候補が流れない。反射、重複、逆順 sequence、異なる日/入力/anchor 順序/交通/燃料/目的群は拒否する。Solution の範囲と経路長も検証する。

worker は mutex の try_lock のみを使う。network I/O は既存 control loop が行い、新しい thread / server / port は増やさない。primary 残量 500 ms 未満は publish を止め、250 ms 未満は取り込まない。通信側も request 残量 500 ms 未満は送信しない。バリアや待機・追加 restart は入れない。

full context を毎 packet に付け、1 frame は 32 KiB まで。超過時は optional sharing だけを省略する。通常送信 queue が空のときだけ hint を入れ、終端 Result は未送信 hint を破棄して優先する。部分送信済み hint は framing を壊さないため完了してから Result を送る（最大 32 KiB）。各 origin の送信 sequence を記録し、同じものを往復させない。root の 4 方向合計上限は毎秒 256 KiB（frame prefix 除く）。

## 検証と記録

`cross_elite.rs` の unit tests は実生成 3 形式（16/24/32 四方）で、full context と実際に合法な初期 Solution、および最大許容 route 配列のサイズを確認する。受信再評価と game.play_day、一致条件の各変異、stale/反射/重複/close/受信前 bind、目的群/期限を検証する。

`wire.rs` は oversized/backpressure/最終 Result 優先/部分送信維持、既存 framing/authentication を検証する。`tests/network.rs` は既存 reconnect/rejected-day に加え、reserve15 workers の on-mode localhost 対戦を行う。取込数や得点改善は時間依存のため数値を assertion せず、全提出の合法性と終端 Result を確認し trace を保存する。

評価区切り後、repo cwd で `. .server/toolchain/env.sh`。候補 workspace に対し `taskset -c 1 cargo test --release --offline -j 1 --manifest-path solver/distributed-candidates/c010-cross-elite/Cargo.toml -- --test-threads=1` を実行する。localhost socket tests は親の許可と実行環境の権限に従う。検証実行時には source/binary SHA・CPU1 制約・子 PID の終了を validation.json に保存する。

request close 時の `cross_elite` 行は published/received/rejected/context_mismatch/imports/improved/sent/oversized を記録する。close 時点の atomic snapshot であり、close と同時に実行中の publication の完了まで待たないため、最終厳密合計ではない。各 worker の graph_ms / iterations と既存 request completion はそのまま。

比較は D0 対 C010 on と同一 C010 off/on の両方を考慮し、通信量、context mismatch、imports→improved、探索 iteration、deadline、全 AC、得点と順位を確認する。これらは改善の保証ではない。

最終の 3 形式 packet サイズは first 3,517 B / semifinal 6,826 B / final 11,185 B。最大許容 route 配列でも final 12,930 B で 32 KiB 内だった。最初の既存 off-mode reconnect テストは、CPU1 上の 5 search worker と D0 eager Graph 構築に対し 900 ms が短く、全 remote 完了の assertion を満たさなかった。テスト要求だけを 1,800 ms にし、既存の検証条件を保ったまま全件再実行して通過した。ソルバ本体の予算は変更していない。失敗した初回の raw log も保存している。
