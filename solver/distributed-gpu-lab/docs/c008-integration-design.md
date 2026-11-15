# C008共有GraphへのGPU producer最小統合案

2026-10-09、C008短縮評価中に作成した設計のみの記録。**C009実装、ビルド、リモート操作、GPU起動は行っていない。** C008もGPU試作も現在の評価済みコードを変更せず保存する。

親担当からのv2途中集計はGraph要求420、実構築214、private bypass139、wait timeout/failureとも0。構築数は要求の約51%だが、139 bypassを全てGPUで置換できるという意味ではない。GPUはsingle-flightであり、まず共有のowner構築を速めたときにBuilding滞留・bypassが減るかを測る。

## 統合の境界

最小候補はC008を別ディレクトリへ固定コピーしたC009で、独立producerをその候補内の固定sourceとして持つ。C008、D0、固定比較相手は変更しない。`SearchPool`にprocess寿命のproducerを最大1つ持たせ、その参照を`GraphCache`へ渡す。GPUは距離とnextだけを作り、共有cache、Path組立、経路選択、評価関数、乱数列は既存の責務に残す。

```text
Solver::graph(..., local_deadline)
  -> PreparedGraph::new（C008と同じ）
  -> GraphCache::get_until（同じ完全key・4entry・128MiB）
       Ready                -> Arc<GraphTables>を返す
       Building             -> 既存の局所期限付き共有待機
       private bypass       -> CPU builderへ直行
       Owner + GPU不適格    -> CPU builderへ直行
       Owner + GPU適格      -> try_gpu（待ち行列に並ばない）
          成功 -> CPU検証済み表からGraphTables組立 -> 同じReady publication
          失敗 -> 同じownerのBuildingGuard内でCPU構築 -> Ready publication
```

`GraphCache::build`はすでに`entry: Option<Arc<Entry>>`を知るため、`Some`だけGPU候補とすればprivate bypassを分離できる。`get_until`から絶対局所deadlineをbuilderへ引き渡す。テスト用builder注入は維持し、CPU/GPUの切替をmap lock・entry lockの外側で行う。新しいGPU用待ち行列、全体GPU mutex待ち、同じkeyの別cacheは設けない。

初版でprivate bypassをCPUに残すのは、並行する異なるkeyがGPUを占有して、多数の待機workerがいるownerを遅らせることを避けるため。GPUが頻繁にidleでCPU bypassの構築費用が残ると分かった場合に、owner優先のprivate tryを次の独立候補で検討する。

## 入力変換と厳密な表の復元

`PreparedGraph`は既に交通を反映したduration/cost、店舗対応、全source列・target列を持つ。変換時に問題から再計算しない。

| producerの入力 | C008からの変換 |
| --- | --- |
| `neighbors[cell][dir]` | `prepared.map.neighbor(cell, dir)`、欠番は`NO_CELL`、方向順保持 |
| `passable[cell]` | `map.terrain(cell) != 3` |
| `durations` / `costs` | 最終の`prepared.durations` / `prepared.costs`そのまま |
| `shops[cell]` | `prepared.spot_at[cell] >= 0` |
| `targets` | `prepared.positions[..prepared.targets]`を順序・重複保持でu16へ変換 |

変換後も`Prepared::validate`を通す。cell上限1024、target列上限1088、duration 1〜4、cost 1〜2、隣接の双方向性、passableなtargetを満たさなければCPUへ戻す。GPU側の制約に合わせて入力を削除・丸め・deduplicateしてはいけない。

GPUの距離配列は`(mode*T+j)*N+cell`、nextは同じlayoutのmode0〜5。受信表は既存の全要素Bellman/next検証を通過した`VerifiedTables`に限って使う。初版では検証を省かない。

`GraphTables::build`の最短距離構築部を共通の組立関数から呼べる形に切り出す。CPUとGPUは同じ最終assemblerを使い、後半のmask/potential/mobilityを二重実装しない。

1. 新しい`Arc<SharedAllocation>`を1つ作る。各(mode,j)について既存`RouteTree`を作り、target=positions[j]、next、fingerprint、同じallocationを格納する。
2. 全source `positions[i]`に対し、距離が有限であることを確認し、`Path{tree, source, length, time, fuel, cells: OnceLock::new()}`を構成する。source列の重複も保持する。到達不能のsourceを既存のassert相当より緩く受け入れない。
3. `fuel_to`は**長さsources**で確保し、`j<targets`だけmode6のN cell距離を入れる。GPUのINF `i32::MAX`をC008の未到達値 **99999**へ変換する。`j>=targets`は空Vecのままにして、各Graph私有の`fuel_cache`へ委ねる。mode0経路のfuel合計でmode6を代用しない。
4. mode0を`paths`、mode1〜5を`alternatives`へ。`Path::same_route`による完全経路比較を残して`path_mode_mask`を作る。fingerprint一致だけで異なるmodeを同一扱いしない。
5. `potential`、`mobility`、`positions`、`cell_node`、`durations/costs`、`spot_at`、targetsをC008と同じ処理で組み立てる。
6. `allocation.set_base_bytes`は組立完了時、`allocation.attach`は現在のGraphCache構築完了経路で**1回だけ**行う。Ready公開前に可変metadataを完成させる。遅延cellsの増分会計とRAIIによるFailed通知を保持する。

time/fuelは次セルではなく出発cellを積算し、targetの出発費用は含めない。nextは同距離候補の最大cell。この同値性は「edgeが出発cellだけに依存、正のedge、heapが同距離cell降順、strict更新」を前提にする。経路重み・tie policyを変えた場合は別の同値確認が必要。

GPU試作の`scalars()`は全6modeの一時配列を持つ。最短の接続ではこれを利用できるが、flat distance/next/scalarsだけで概ね`148*N*T` bytesを要し、GraphTablesの最終配列とは別に生存する。device表は概ね`40*N*T+22*N+30*T` bytes。初版は推定temporary bytesとRSSを記録し、必要ならそのshapeのGPU利用を見送る。cacheの128MiBを一時bufferやRSSの上限と誤認しない。

入力shapeが大きくtemporaryが問題になる場合の小さな追加APIは、検証済み表から1(mode,j)ずつ木DPを返す`for_each_route_tree`と、所有権を移す`into_parts`。一時scalarをN cell分に絞ってassemblerへ渡す。これは試作版にまだないAPIであり、導入するなら同値テストと測定をやり直す。初版から複数shapeのdevice buffer cacheやGPUメモリ共有を追加しない。

## fallbackと局所deadline

現在のC008はCPU構築そのものを中断できない。GPU追加で保証できるのは**GPU待ちの上限と遅いGPU結果を公開しないこと**であり、未計測CPU fallbackまで含めた厳密な終了期限ではない。既存coordinatorの提出締切・有効候補の保持・遅延worker結果の排除を変更しない。

絶対局所期限をD、現在時刻をt、同host/shapeのCPU構築見積りをEcpuとする。GPU予算候補は次を使う。

```text
remaining = D - t
cpu_reserve = 1.5 * Ecpu + 20ms
gpu_budget = min(configured_cap, remaining / 8, remaining - cpu_reserve)
```

`configured_cap`の初期検討値は250ms。ただし実際のtarget数を使う有限ベンチでGPU全処理/検証/組立を測ってから固定する。小さい8targetのsmoke時間だけで上限を決めない。Readyでなく、busy、未検証host/shape、見積り未確立、budget不足、予測GPU合計がbudgetを超える場合はGPUへ送らずCPU直行。CPU見積りの初期fallbackはC008の1µs/(N*T*6)を引き継ぐか、同条件で測ったCPU上限を候補設定へ固定する。

GPU admission判定と入力変換の後にも残量を再確認する。producerのdeadlineはDそのものではなく`min(D-cpu_reserve, t+gpu_budget)`。受信後のCPU scalar/assemblerにも同じ局所GPU期限を渡し、treeごと・source行ごと・mask行ごとに確認する。部分表はReadyにしない。組立中に期限へ達した場合は部分表を捨て、まだ同じownerでCPUへ戻す。捨てる費用も実測へ含める。

GPU要求のchannel送信も現試作の`try_send`を維持し、送信待ちでBuilding ownerが止まる経路を作らない。GPUの結果受信だけに期限を付けても、受付待ちが無期限ならfallbackへ進めないため、admissionから組立完了までを1つの局所予算として扱う。

GPU timeout・driver error・検証不一致はそのprocessのproducerを停止状態へ移し、以後の要求はCPUへ。timeout後のGPU計算はdriver内で直ちに止まるとは限らないが、single-flightなので余分なin-flightは最大1件。返答receiverを破棄して遅い結果のcache公開を防ぐ。提出経路でdriver threadをjoinしない。process終了時の回収は外側launcher/controllerが担う。

**CPU再構築見積りにGPU成功時間を混ぜない。** 現C008の`build_ns_per_work`は実構築時間を全て集計するため、統合時は`cpu_build_ns_per_work`を分離し、CPUを実際に完了した構築だけで更新する。GPUが最初に成功して短い時間を記録すると、CPU fallbackの余裕を過小評価する恐れがある。既存の合計build数/buildMsは全Graph構築として残し、CPU/GPU内訳・変換・待機・検証・組立・失敗後CPUの時間を追加する。

## process寿命とhost別有効化

`SearchPool::new`でproducerを非同期起動し、そのGraphCacheと同じprocess寿命で持つ。SolverのselectやProblem切替ごとにproducerを作り直さない。初期化中のGraphはCPUで作り、GPU Readyを提出経路で待たない。driver/bufferはprocessに1組だけ。cacheのkeyとsession識別はC008のままなので、前試合の入力や位置番号を誤用しない。

| host | 探索thread | producer/driver CPU | 寿命と最初の判断 |
| --- | ---: | ---: | --- |
| VOIDLINUX-1 | 31 | 31 | coordinatorは毎試合再起動。今回のcold初期化2.71秒を各試合で払い得る。短縮初期選別ではGPU offを有力案とする |
| VOIDLINUX-2 | 15 | 15 | workerが全試合persistent。初期化を12件で償却しやすく、今回途中集計の構築214件がある。v2で正確性確認後の最初のGPU有効host候補 |
| VOIDLINUX-3 | 31 | 31 | workerはpersistent。旧r3では併用効果があったが、新kernel/FFIをRusticlで検証していない。まずoff、効果を確認してから別条件で試す |

最初は全hostでdefault off。`PROCON37_DISTRIBUTED_GPU=off|opencl`、device selector、cpuset path、GPU cap/temporary上限などをhost別の固定設定として記録する。環境変数は案であり、まだ実装していない。NOMLABの比較相手と単一スレッド2体へ渡さない。deviceの実名・UUID・driver/runtimeを起動ログに残し、selectorが一致しなければCPUへ戻す。

v2/v3のworker起動直後、試合開始前にReadyを確認できる運用は可能だが、同じprocessのまま12件を完走し、初期化時間を記録から除外しない。v1では「2件目以降がwarm」という前提は成り立たない。試合間でprocessが落ちた場合は、その回数と再初期化を記録する。正式評価だけ特別に初期化を隠す運用はしない。

## producer TIDだけを移すroot準備

現smokeのようにprocess全体をCPU 31へ閉じ込めるlauncherは、統合ソルバには使わない。探索workerとcontrol threadを通常のhost CPU集合へ残し、producer TIDとそこから作られるdriver/compiler子孫だけをsingleton cpusetへ入れる。

評価開始前にrootがhostごとに今回候補専用のv1 cpusetを作る。例は`/sys/fs/cgroup/cpuset/procon37-c009-v2-<run-id>`。group directory、`cpuset.cpus`、`cpuset.mems`、親group、他のcontrol fileはroot管理を維持する。CPUを15、memsを親の有効nodeから設定する。CPU 31のhostでも同様。既存groupを勝手に再利用・削除しない。

現実的な第一案は、そのgroupの **`tasks`だけ** を実験ユーザー所有・0600へ変更する限定委譲。directoryの書込みやcpuset設定変更権限は渡さない。producerのstartup gateは、まだOpenCLを読み込んでいない自身のTIDを`/proc/thread-self`から取得し、そのtasksへ1回書く。`cgroup.procs`へprocess PIDを書かない。書込み後、既存`Confinement::verify_current`でmount、membership、singleton/effective CPU、allowed CPU、tasks内TIDを検査してからdriverをロードする。

この方式は対戦ごとのsudo起動やrootとのRPC待ちを不要にする。v1の毎試合新しいproducerも同じ評価用groupへ自己参加でき、v2/v3のpersistent producerも維持できる。ただしユーザーに「自分のTIDだけ」をkernelが特別に限定する仕組みではなく、同一UIDの他のthreadもtasksへ書ける可能性がある。委譲範囲を今回groupへの参加だけに絞り、実装側では自身のTID以外を書かず、起動前後のsearch/main membershipを監視する。kernel/実機のtasks書込み権限条件は、対戦外の有限preflightで確認する必要があり、まだ確認していない。

限定委譲が実機で機能しない、または同UIDの権限をさらに絞る必要がある場合は、rootがstartup gateの認証済みUnix socket要求を受け、所有UID・TGID・TIDと候補PID/starttimeを照合して対象TIDだけ移す小さなbrokerを使う。brokerは実験ユーザー限定socket、候補専用group、期限付き1回応答とし、任意PID・任意path・shell commandを受け取らない。初版ではroot daemonを増やすよりtasks限定委譲の有限preflightを優先する。

GPU threadのmembershipを設定する前に`dlopen`、device列挙、program buildをしない。全OpenCL操作をproducerに集約する。driverがaffinityを広げてもgroupのCPU上限内に留まる。実機samplingとgroup内tasksの確認で確かめ、設計だけで「全driverがこのthreadから派生する」と断定しない。

## CPU集合と比較条件

最小C009はsearch数31/15/31と、現在のC008 launcherが許可するhost全CPU集合を維持する。producer/driverだけCPU31/15/31に制限するため、利用可能な集合は32+16+32=80論理CPUの範囲内。**OS thread数はproducer/driver追加により80を超え得る**が、CPU利用可能集合は増えない。GPU threadがcontrol/searchと同じCPUで競合する費用も対戦成績に含める。

GPU CPUをsearchから完全に予約するなら、search maskを0〜30/0〜14/0〜30へ、main/controlとproducer/driverを31/15/31へ分ける。search数は31/15/31のまま。ただしこれは配置の変更なので、GPU offのC008も同じCPU割当で評価し直し、C009との比較条件を揃える。初版のGPU効果とCPU配置変更を1つの差にまとめない。

評価後はproducer終了を有限時間だけ待ち、driverが残る場合は外側が候補のPID/starttimeとgroup membershipを照合して回収する。producerのTIDだけをSIGKILLするとprocess全体が終了し得るため、試合途中の再起動方法にはしない。group tasksが空であること、候補由来driver/compilerの残存なし、group削除成功を記録する。persistent workerは12件の最後に止めて回収する。

## 実装後に必要な確認

1. GPUを使わないadapter検査。C008 CPU表と、独立heap/同期緩和由来のVerifiedTablesから組み立てたGraphTablesを全Path、scalar、mode mask、fuel_to、mobility、potentialで比較。1,608,270経路の既存同値テストも継続する。順序/重複、交通変更、店の上のagent、未到達値、非target燃料cacheを含める。
2. mock producerでStarting/Busy/期限切れ/不正表/late reply/初期化失敗を注入し、CPU fallback、ownerのReady/Failed通知、allocationの単一attach、CPU見積り非汚染、期限の累積防止を検査する。
3. 対戦間隔でtasks限定委譲の有限preflight。main/searchのCPU集合と所属groupが変わらず、producer/driver/compilerだけsingletonになることを確認して回収する。
4. hostごとの実GPU正確性。v1の合成8targetだけで済ませず、C008の実際のPreparedGraphを使う3形式・交通/位置の違い・順序付き重複を確認する。生産binary/source/configのhashを固定する。
5. 同条件の有限CPU/GPU併用計測。変換、upload、dispatch/read、CPU検証、scalar/assembler、共有owner数、busy率、CPU bypass、GPU timeout、累積構築時間、wall、RSS、iteration、初期化/終了を記録する。GPUがcache待機とprivate bypassを悪化させていないかを見る。
6. 採用済み版・C008と同じ4チーム・12件・seed・制限時間の短縮を反復。host enableやCPU配置を変えた場合は基準版も同条件で再評価。明確な改善だけ正式12件へ進め、全ACと順位/3得点指標から採否を決める。

## 設計時のsource識別

| source | SHA-256 |
| --- | --- |
| C008 `solver/planner.rs` | `a8d6abd6b2bdbe7c15945cf5f1b42092b6d53442dd5b5f820e785f2ac761955d` |
| C008 `solver/graph_cache.rs` | `f01cb2d11d919179dd1710fc099ae0fea600ef6384ab9e080025838e7ca58c5a` |
| C008 `pool.rs` | `c1196b294979edec465ebaa26bf51ca8679e4d20a4d0b2d968ca6532a864ff57` |
| C008 `runtime.rs` | `a8ebea03ba5ef47dafe65305488bb2c60af8a1a7e9c6efaf4ae386dada045922` |
| producer `src/lib.rs` | `9c5967e3c2cd6c23122ce3d4b3b411b62d8d217dcaf9e4767757b583a0176811` |
| producer `src/producer.rs` | `2aa52f2f6c1bf7d75c85cccf9dd03eb5f0f739515abaeac0fd16a8d125253ef8` |
| producer `src/opencl.rs` | `2ffb890d0aeb075e87990eec687ceba7243405137902faa313df90a43670f96d` |
| producer `src/shortest.cl` | `e0f6bbadcaf93c281a1bbd2224fd7146ba886f9df0a35e08592f840254d8b57e` |

参照した実機結果は[初回smoke](v1-smoke-20261009-r1.md)。C008の実装は`solver/distributed-candidates/c008-shared/`のみを読み、保護対象・未回収旧GPU source・binary逆解析は使用していない。
