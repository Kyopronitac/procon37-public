# C012: 独立探索を残すPC間協調（未採用・ローカル検証済み）

2026-10-09追記：実機の固定4チーム短縮12件は全48AC、平均順位1.1667、種類222・日別1277・玉2968。ユーザー指示で初回終了後に一時停止。反復・正式評価・採用は未実施。実機で候補の送受信と種変更を確認済み。詳細は [停止記録](../../docs/distributed/resume-20261009/pause-20261009.md) を参照。

採用済みD0を独立コピーし、各PCの1 search workerだけを受信専用にする候補。残る独立workerの7つのreserve群から、探索途中の候補を有界batchで送る。受信担当は停滞時だけ別の探索枝の種に使う。**CPU 1・Cargo -j1でrelease build、単体18件、localhost TCP 5件が通過した。実対戦は未実施で、成績改善や正式採用を意味しない。**

- 親：D0、初期採用commit `5d6c740962a1f3e20e77e13df243078df1d29ad2`。
- D0 source SHA-256：`61b56e0ae79cfae1ca86ddc62e8e38bb94a9410871508cadbd783a9b9a469d65`。
- 再利用：自作C010のTCP framing、任意hintの優先度制御、full context照合、要求単位busの寿命管理。C010の全reserve15 workerへのbest取込方式は使用しない。
- C006の時間配分、C007/C008のGraph変更、C011の評価項、GPUは含めない。D0のGraph、評価式、探索重み、RNG初期値、通常restart、外側91%予算を保持する。
- source識別：[source-manifest.json](source-manifest.json)、実行状態：[validation.json](validation.json)。本体変更後にはソースのSHAを更新し、レビュー・検証をやり直す。

## 起動と役割

3台とも同一C012 binaryに `--cross-elite on` を付ける。`--cross-elite off` が既定で、比較診断用に共有を無効化できる。parallel modeでonは受け付けない。wire protocolはVERSION 3で、D0のVERSION 1、C010のVERSION 2、on/offが異なる相手を認証・Ready時に拒否する。配置・ハーネスはこの候補内から変更しない。

各nodeのsearch worker範囲内で、`worker % 2 == 1 && (worker / 2) % 7 == 3` を満たす最小IDを受信専用にする。存在しない範囲には受信担当を置かない。現配置ではVOID1が7、VOID2が35、VOID3が49で、計77 search＋3 controlの80論理CPUを維持する。

受信担当3本は、その要求の機内Eliteの送受信から外し、PC間donorにもならない。残る74本は輸入解を受け取らない。追加の乱数消費と探索枝の切替は受信担当だけに生じる。ただし独立側のreserve15群から共有相手が1本ずつ減るため、offと完全に同じ軌跡を保証するものではない。最終的に採用した提出stateを翌日全workerへ同期するD0の規則は維持する。

## 全7群のbatch

独立workerは既存の16,384 iterationごとのpublication地点で、自群の改善候補を最大500 msに1回更新する。群はreserve `[4,8,12,15,20,30,45]` に対応する。valueを比較するのは同じorigin・同じ群内だけで、追加canonical評価や群をまたぐvalue比較は行わない。

1 batchはsession/request/input digest、共通のfull context、最大7 offerを含む。offerはorigin/group/worker/sequence/value/Solutionを持つ。送信は各方向500 msに最大1 frame、32 KiB以内。保持はorigin×groupごとに最新1件、3 origin×7群＝最大21件である。

- full contextはProblem全値、agents/day/steps/supplier/seen brands、Graphのpositionsの順序と重複、targets、durations/costsを含む。digestだけでは受理しない。
- sequenceと既送信・既受信の記録はorigin×group単位。新しい群の小さいsequenceを別群の更新と比較しない。
- 未送信の更新をround-robinで選ぶ。batchに入り切らない群を次回の先頭に残す。単独でも上限を超えるofferは共有だけ省略し、上限は増やさない。
- coordinatorの中継でもorigin/group/sequenceを保持する。宛先自身のoriginを送らない。leafは輸入解を再送しない。
- envelope・context・範囲・群とworkerの対応・有限value・Solutionの形・重複slotなど、**いずれか1 memberでも不正ならbatch全体を棄却**する。合法な旧sequenceや再送重複だけは個別に無視する。
- Graph構築前にも受信できるが、局所contextをbindするまで転送・取込しない。不一致の仮保持をbind時に除外し、そのsequenceで後続の正しい候補を抑制しない。

現在の選出規則で受信担当となるworkerを送信者として記載したofferも拒否する。受信解の機内EliteやPC間donorへの伝播を防ぐ仕組みであり、受信候補が次の正式提出に選ばれることは禁止しない。

## 受信担当の探索

実日primary、future/coverage weightが0、`next_steps == 0` の場合に限る。Select・forecast rollout・probeにはbusを接続しない。短時間fallbackは従来の早期returnを通る。

固定係数は全形式・全seed共通。

| 項目 | 値 |
| --- | --- |
| 通信の送信間隔 | 500 ms |
| 受信担当の確認間隔 | 250 ms、既存1,024 iteration処理点で確認 |
| bestの停滞 | 1秒、確認時に評価値が0.00001より大きく改善すればリセット |
| 前回の種変更からのcooldown | 1秒 |
| primary残時間 | 750 ms以上 |
| 次の通常restartまでの余裕 | 100 ms以上 |
| 1回に再評価する候補 | 最大3件、群をまたいで公平に選択 |
| 部分破壊 | 非空の収集車経路1本から連続 `ceil(length/4)` 個を削除 |
| 修復 | 既存 `beam_car` に最大25 msの局所締切を渡す |

受信側でGraphを使って再評価し、senderのvalueは候補選択に使わない。best/currentと同一のSolutionは除外する。最大3件の中で自分の評価が最も高い候補を種にする。補給拠点・時刻を保ち、収集車経路だけを部分破壊・修復する。

修復結果は`current`へ入れ、元のbestは保持する。修復結果が既存評価でbestを上回る場合だけbestも更新し、以後はD0の通常探索・通常restartを続ける。要求全体で最大2回という上限は設けず、cooldown・残時間・通常restartとの距離で制限する。元のbestの保持はprimary評価の性質であり、翌日の実成績や最終rolloutの改善を保証しない。

250 msは確認機会の最短間隔であり、他のD0探索処理中に割り込む保証ではない。1回のevaluateとbeam内の細かい処理は中断不能で、25 msも既存beamへ与える締切である。実際の所要時間・超過量を検証時に測る。探索予算の延長、追加Graph、追加thread、受信待機、barrierは入れない。

## 最終提出と観測

共有hintはCandidateの完了、pending、completed、remote_results、authoritative stateを変更しない。最終Candidateの比較・rollout・合法性確認・提出・次要求のstate同期はD0の経路を使う。未送信hintを最終Resultより優先せず、部分送信済みframeだけはframing維持のため完了させる。最大frameは32 KiB。

request close時の有界ログは、群別published/received/sent、rejected/context mismatch、確認・条件成立回数、最大停滞時間、再評価件数と時間、種変更件数、修復時間、直接修復によるbest改善件数、receiver mailbox内の最大滞留時間を出す。`imports`は実際の種変更数、`improved`は修復直後のprimary best改善数である。後の探索や対戦結果への因果を表す数字ではない。最終選出workerは既存`dist request`行から別に確認する。

mailbox滞留は受信後から取込までの局所時間で、LAN往復・serialization込みの総遅延ではない。closeと同時進行の処理は待たないのでカウンタはclose時点のsnapshotとなる。

## 準備した検証

新規単体テストは、受信役の一般化・送信禁止、実生成3形式の合法な全7群batchと最大routeサイズ、全context変異、不正memberの全体棄却、origin×group sequence、重複・旧順序・反射・pre-bind・close、公平な容量分割、過大frame、停滞・cooldown・期限、部分破壊の範囲と元bestの保持・修復後の合法性を対象とする。

localhost統合テストは既存の再接続・不受理日同期・thread上限・on-mode実送受信を引き継ぎ、旧VERSION 1/2およびon/off混在を正しいtokenでも拒否し、同じdaemonへVERSION 3/onで再接続できることを追加した。既存のframeテストは過大hint省略・backpressure・Result優先・部分frame保持を確認する。

CPU 1、Cargo `-j1`、`--test-threads=1`、240秒timeoutで実行し、全23件が通過した。buildを含む実行全体は74.056秒、release compileは53.51秒、単体は2.32秒、TCPは17.81秒。監視28 PIDは全てCPU 1、test child記録10件は全て停止済み。rootのhost `/proc` 監査でも同一PID/startTicksの残存と同一binary実行プロセスはなかった。別sandboxの `/proc` はhost PIDを表示しないため、独立したhost残存判定には使っていない。採否は固定4チーム・12件の短縮反復と、改善時の正式フルテストで判断する。

## レビュー後の修正と検証

初回凍結 `906869314d90e4aab5d5d42d9fba95396d25a509b4247759a24e5b87c400155b` の独立レビューで、`SearchPool::begin_cross` にC010由来のreserve15限定条件が残り、他の6群へbusが渡らない問題を確認した。Planの全群へ渡す1条件変更で修正し、Select除外と受信担当の隔離は維持した。修正後のsource SHAは `10acfa3bde81f1dddf1d80ed4ff2fbe0288395fc4eb6dd0923a410607ed8a541`。

直接Bindingを作る単体テストでは配信漏れを検出できないため、実際の`begin_cross`へ記録用mpsc endpointを接続し、現配置77 jobの全7群配信・受信役7/35/49・Select/offの除外を確認する回帰テストを追加した。さらにlocalhostで非reserve15のgroup0 donor（worker 0/1）から、受信担当35を持つnodeへの実送受信と合法なResultを確認する統合テストを追加した。修正後の差分レビューを通過し、上記の実行で両回帰テストも成功した。検証後にsource34件のSHAを再照合し、変更がないことを確認した。

## 検証結果の限界と実行ファイル

release binaryは `target/release/distributed-solver`（2,047,432 bytes）、SHA-256は `574dd99a8ff0d20035acccca3f4e207ad2d7701308bc10374694137aa0f33016`。7 offerを含む実生成batchは1回戦4,619 B・準決勝8,006 B・決勝12,491 B、最大許容route配列を使っても11,619 / 16,756 / 24,706 Bで32 KiB以内だった。

localhostでは実送受信と非reserve15のgroup0送信3件／受信3件を確認した。一方、停滞などの条件を全て満たす機会はなく、`eligible/imports/probes`は0だった。停滞判定・部分破壊・修復後の合法性は単体で検証したが、実時間のplanner経路で種が切り替わる場面はこのTCP実行では観測していない。実対戦で取込数・費用・成績を確認する。

詳細は[検証記録](../../docs/distributed/resume-20261009/C012-validation-20261009.json)。raw logとprocess記録は `.server/distributed/c012-validation/` に保存した。5つのtest directoryからはlogとprocess JSON計20ファイルだけをコピーし、tokenファイルは読取・コピーしていない。
