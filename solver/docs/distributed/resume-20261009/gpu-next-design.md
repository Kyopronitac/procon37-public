# C007後の機内Graph共有とGPU producer案

2026-10-09の設計メモ。C007の短縮評価結果を見て実装順を決めるための案であり、実装・配置・採用は行っていない。作成中のリモート操作、計算負荷追加、未回収ソースの参照はない。

## 判断

**C007の機内共有をCPUのみで先に測り、その同じ共有層へ常駐OpenCL producerを追加する順がよい。** CPU探索との併用は現実的だが、GPUを使うために全workerが1つの遅いproducerを待つ構成は避ける。

既存r3の同一プロセスwarm GPUは、30本のD0探索との併用でも、初期化・終了込みでVOID1は約45%、VOID3は約25〜28%のGraph構築/query時間短縮を2回再現した。しかし、これはGPU結果を探索へ渡していない人工入力の測定である。C007は別実装であり、r3の時間をそのままC007へ当てはめない。[測定条件と結果](gpu-evaluation.md)を参照。

C007自体にも機内で同じ最短路を重複構築する余地が残る。共有によってCPU構築がほぼ1回で済むなら、GPUが短縮できる残りの時間は小さくなり、driver初期化・CPU配置・GPU待ちの費用を償却できない可能性がある。したがって、GPUなしの共有版を比較基準から省略しない。

## 読んだ版と現状

対象は `solver/distributed-candidates/c007-compact` と採用D0、および回収した自作ラボ/003-r2の設計・計測資料のみ。

| ファイル | 設計時SHA256 |
| --- | --- |
| C007 `solver/planner.rs` | `368982f8f68a28270e76561239354aa52bc5a3b13e823b3ebecdc9a376d38d07` |
| C007 `solver/compact_tests.rs` | `9318f2f8b411c6bd8bd0c168f5aec28132995b31527c8dc09f29cae038f72982` |
| C007 `pool.rs` | `74a981b5f1423b16e07b8f0e888e00c797b41b670140921112b874b4a6b27214` |
| C007 `runtime.rs` | `a8ebea03ba5ef47dafe65305488bb2c60af8a1a7e9c6efaf4ae386dada045922` |
| D0 `solver/planner.rs` | `11e043e33018dee34f41f295d6ae1500497b0bfc7c8c33d5b253a5621587461a` |

C007の`RouteTree`はtarget、`u16 next`、fingerprintを持つ不変表で、同一Graph内のsource間では`Arc`共有されている。各`Path`はtime/fuel/lengthを保持し、実際のセル列を`std::cell::OnceCell`で遅延生成する。`Graph`にはさらに`RefCell`のroute memo、supplier cache、燃料距離cacheがあるため、現状の`Arc<Graph>`をworkerへ配るだけでは共有できない。

`Graph::new`は地形・交通からduration/costを用意し、店・車・追加anchor・空間gridなどから順序付きpositionsを作る。その後、6種類の経路modeをtargetごとに逆向きDijkstraで作り、燃料のみの距離表、mode重複mask、potential、mobilityを構築する。探索パラメータや乱数はこの構築には使わない。

## 第1候補：CPUだけの機内共有

最小の分割案は`GraphTables`とworkerごとの`Graph`である。

| 共有する `Arc<GraphTables>` | workerごとに保持する `Graph` |
| --- | --- |
| paths/alternatives、RouteTree、time/fuel/length | route memo (`RefCell<RouteMemo>`) |
| positions、target数、cell_node、spot_at | supplier cache (`RefCell<Option<...>>`) |
| durations、costs、mode重複mask、potential、mobility | 非target燃料距離の遅延cache（初版は私有のまま） |
| 構築済みtarget向け燃料距離 | selection_graphのworker内寿命、探索状態 |

`Path.cells`だけは`std::sync::OnceLock<Box<[usize]>>`へ置き換える。生成closureは不変next表を歩くだけなので、同じ経路の初回生成を1workerに限定し、完成後は全workerが同じ不変セル列を読むことができる。`same_route`のfingerprint比較後の全経路照合は維持する。ハッシュだけでmodeを統合しない。

`Graph { tables: Arc<GraphTables>, memo, supply_cache, ... }`から`GraphTables`への`Deref`を使えば、`g.paths`など既存のフィールド参照の多くを残せる。`Path::cells()`の返すsliceの寿命は共有表の寿命に結び付ける。`Rc<Graph>`を使うselection_graph自体はworker私有のままでよい。`fuel_distance`では共有済みtarget表を読み、未構築の非targetだけ現在と同じCPU Dijkstraを私有cacheへ作る。

共有cacheは**同一プロセス内だけ**に置き、TCP通信やcoordinatorの採点・採否は変更しない。`SearchPool::new`でserviceを1つ作り、各workerの`Solver`へcloneしたhandleを渡す。後続の`select`で`Solver`を作り直してもserviceはpoolと同じ寿命を保つ。並列モードにも同じ注入箇所を用意するが、固定NOMLABの既存バイナリと設定には適用しない。

最初のcache keyは完成した入力を表す`PreparedGraph`の**全要素一致**にする。具体的にはmapの幅・高さ・通行可否/隣接関係、duration/cost、店indexの対応、順序と重複を含むpositions、targets数を含める。rawの交通配列順序だけで比較せず、C007と同じ順番で反映した最終duration配列を比較する。positionsをsort/dedupすると車indexやanchor indexの意味が変わるのでしない。day番号、fuel、乱数seed、stock、supplierだけをkeyに足して同一性の代用にもしない。hashはbucket選択用とし、衝突時は全要素を比べる。

cacheのmap mutexはentryの検索/挿入/除去だけに使う。entryごとの`Building/Ready/Failed`と通知を用い、構築中に全cacheのmutexを保持しない。別入力のCPU構築を1本へ直列化しない。構築ownerのRAII guardが異常終了時にも`Failed`を通知し、現行workerの`catch_unwind`後に`Building`が残らないようにする。最初は保持4入力程度・目標128MiB程度のLRUを設定値にし、計測で調整する。ただしArc参照中の表と遅延生成されたセル列は即時破棄できないため、これはプロセス全体の厳密なメモリ上限ではない。構築/生成/退去したbytes、live bytes、peak RSSを併記する。

CPU共有の確認では、同一入力を31/15workerから同時に要求して構築回数が1になること、交通・agent位置・extraが違う入力を取り違えないこと、maskと経路列がC007/D0と一致することを確認する。初版ではtarget単位の複雑なcache分割を導入せず、full Graph共有のhit率を測る。異なるpositionsでも多くのtarget木が同じことが律速として残った場合だけ、`(topology, weights, target cell, mode)`単位の木共有を次案にする。

## 第2候補：同じ共有層の常駐OpenCL producer

GPU producerは各VOID機のソルバプロセスに1本。context/program/device bufferを複数要求の間保持し、OpenCL関連の操作をそのthreadに集約する。CPU探索は既存の候補生成・評価・rolloutを続ける。初版でGPUへ移すのは次の整数計算だけに限定する。

| GPUが返す表 | CPUが引き続き作るもの |
| --- | --- |
| 6経路mode × target × cell の距離 `i32` | PreparedGraph、source/target/anchorの順序 |
| 上記と同値のnext cell `u16` | 木のtime/fuel/length/fingerprint（後述の一度のDP） |
| 燃料のみのtarget × cell距離 `i32` | Path metadata、mode重複mask、potential/mobility |
| 収束/shape/失敗の診断 | 遅延セル列、探索・補給のシミュレーション、得点 |

候補003-r2のpipeを再利用せず、producerが所有する連続host bufferへ結果を読み戻し、検証後に`Arc<GraphTables>`を公開する。同一プロセスなので表のencode/pipe転送/decodeを不要にできる。初版はsingle in-flight、bounded queueとし、埋まっている場合に無制限に要求を溜めない。入力が同じworkerは共有entryで合流するが、異なる入力はGPU待ちの利益がなければCPU構築を選ぶ。

### 重みと逆向き距離

targetを`t`、出発セルを`y`とすると、C007の各modeの重みは次のとおり。

```text
A = [100, 100, 10, 100, 100, 100]
B = [  3, 100,100,   3,   3,   3]
R = [  0,   0,  0,  20,  80, 160]
w_mode(y) = max(1, duration[y] * A[mode] + cost[y] * B[mode]
                   - (shop_at[y] >= 0 ? R[mode] : 0))
w_fuel(y) = cost[y]
d[t] = 0
d[y] = min_over_passable_neighbors_x(d[x] + w(y))
```

出発セルの費用であり、到着セルの費用に置き換えない。全て整数で、経路modeでも重みは正。現在の盤面上限32×32では最大edgeは600、単純経路の距離は最大613800に収まるが、実装ではinput shapeとchecked arithmeticで上限を検査し、INFへの加算を避ける。blocked/unreachable cellの値とtargetの未使用next値も参照と合わせる。

最初のkernel案は`(mode,target)`ごとに1workgroupを割り当て、local memory上の距離2面でJacobi型の緩和を行うもの。各laneが複数cellを担当し、各roundの読取り面と書込み面を分離する。全laneがbarrierへ到達し、group内の変更有無をreduceする。正重みなので必要経路は高々cell数−1辺、最終の無変更確認までcell数round以内で扱える。上限に達して収束確認できなければ失敗を返す。実workgroup幅はdeviceの上限・local memory容量を初期化時に確認して選ぶ。

1targetを全体の唯一の仕事にすると並列度が足りないので、6経路modeと燃料mode、複数targetをまとめてdispatchする。初版ではGPU上でtime/fuel/fingerprintまで融合せず、距離/nextの同値性とCPU後処理の費用を分けて測る。

### D0/C007と同じnextを得る条件

現Dijkstraのheap要素は`(Reverse(distance), cell)`であり、最短distanceの中では**cell番号が大きいものから**取り出す。更新は`candidate < distance[y]`のときだけで、同値の再更新はしない。

現在は固定した出発セル`y`についてedgeが次セル`x`や方向に依存しない。そのため、`y`の最短経路になれる隣接`x`は全て同じ距離`d[y] - w(y)`を持つ。さらに`w(y)>0`なのでそれらは必ず`y`より先に確定し、その中で最大cell番号が先に緩和する。したがって、距離確定後に

```text
next[y] = 最大の x
          条件: x は通行可能な y の隣接セル
                d[x] + w(y) == d[y]
```

とすれば現在のstrict-update Dijkstraのnextを再現できる。この導出は**edgeが次セル非依存、全edge正、heapの同距離順がcell降順、同値更新なし**という4条件に依存する。方向別費用、到着セル依存、ゼロ辺、tie更新方針のいずれかを変えた場合は再利用しない。隣接方向の番号を最小化する規則とは異なる。

GPUで距離緩和中に「最後に書いた親」を採用する方法は実行順に依存するので使わない。距離の確定とnextの確定を分ける。C007の1,608,270経路比較を再利用し、さらに同距離の隣接候補が複数ある小盤面を明示的に追加して、next全表と実セル列の双方を照合する。

### time/fuel/length/fingerprintのCPU集積

GPUの距離表を使ってtimeを除算推定してはいけない。重みには燃料と店rewardが混ざるため、実際に選ばれたnext木に沿って積算する。

```text
time[t] = fuel[t] = length[t] = fingerprint[t] = 0
time[y] = duration[y] + time[next[y]]
fuel[y] = cost[y] + fuel[next[y]]
length[y] = 1 + length[next[y]]
fingerprint[y] = rotate_left_17(
    fingerprint[next[y]] XOR (next[y] + 0x9e3779b185ebca87 mod 2^64)
) * 0xc2b2ae3d27d4eb4f mod 2^64
```

各mode/targetの木で、未計算セルからnextをたどって計算済みセル/targetに達したらstackを逆順に処理する。計算済みmarkを使えば各cellを一度処理し、sourceごとに経路全長を積算し直す必要はない。nextの隣接性、距離の厳密な減少、target到達、length上限を検査し、異常な循環・未到達を公開前に拒否する。fingerprint一致だけでmaskの同値判定を省略しない。

燃料専用表は経路mode0のfuel累積とは異なる。最短時間寄りの経路の燃料量と、燃料だけを最小にする距離を混ぜない。非target向けの遅延燃料Dijkstraは最初はCPUのままにして変更範囲を限定する。

## 締切、fallback、プロセス寿命

GPUは`Starting / Ready / Disabled`の状態を持つ。OpenCL初期化はproducer内で非同期に行い、Startingの間はCPUで構築する。VOID1で観測した約2.7秒の初回初期化を、最初の探索worker全員の待ち時間へ加えない。起動からreadyまでの時間と、その間のCPU探索への影響は成績測定から除外しない。

Graph構築要求にはjobの絶対deadlineと、`plan_inner`/rolloutで割り当てられた局所deadlineの早い方を渡す。現行の91%、110ms、rollout配分、coordinatorのcutoffは変更しない。queue待ち・GPU待ち・全表検証・CPU fallbackに必要な余裕を含め、余裕がない場合はGPUへ送らない。

workerが待つのはRust側の期限付き通知だけとし、worker/main/coordinatorから`clFinish`などの完了待ちを呼ばない。GPU service全体のmutexを持ったまま待たない。期限切れ時にはその入力のGPU世代を失効させ、遅れて返った結果で現在のCPU結果を置き換えない。CPU構築に十分な残り時間があるときだけCPUへ切り替える。残り時間が足りなければ、そのworkerは候補を返さず、coordinatorの既存の安全なfallbackを維持する。rollout中の失敗では、既に確保した最初の日の候補・snapshotを残す。

**同一プロセスのGPU driver呼出しを安全に強制中断する絶対時間保証は置けない。** producerがdriver内部で止まってもworkerの応答待ちを打ち切れるようにし、GPUをDisabledにして追加要求を止める。未完了buffer/contextを他threadから破棄・再利用しない。未完了jobは1件だけに制限する。正常終了ではproducerの正常な破棄を計測し、終了できない場合は外側の既存ソルバプロセス監視がそのプロセスを回収する。producerの`join()`を提出経路へ置かない。

この設計はGPU待ちを提出経路から外すもので、driver起因のプロセスクラッシュまで隔離するものではない。その隔離が必要になった場合は別プロセスhelper＋共有メモリへ進む。過去003-r2の終了不具合を踏まえ、異常終了をCPU fallback成功に書き換えず、回収失敗は評価失敗として記録する。

## CPU配置とdriver thread

GPUをCPU31（VOID2なら15）へ置き、使用できる論理CPU集合は各機32/16/32の内側に留める。driverのidle thread数をそのまま探索thread削減数へ置き換えない。最初の候補では既存の探索31/15/31本を維持し、producerは指定CPU上で時間を共有する。そのCPUの探索との競合も評価費用に含める。

単なるproducerの`taskset`だけでは、driverによるaffinity拡張を継続的に制約できた証拠にならない。現在のr3で使ったv1 cpusetの考え方を、プロセス全体ではなくproducer threadへ適用する案が具体的である。

1. launcherが通常のCPU集合を持つsolver用groupと、singleton CPUのGPU用groupを作る。
2. mainがproducerをspawnし、producerはOpenCLを読む前に自身のTIDをcontrollerへ通知して待つ。
3. controllerがそのTIDの所有者・親プロセス・開始識別を確認し、GPU groupの`tasks`へ移す。TID単位で移し、mainや探索workerをGPU groupへ移さない。
4. producerが自分の`/proc/self/task/<tid>/cgroup`とCPU集合を確認してからOpenCLの動的loader読込み・device列挙・初期化を行う。そこで生まれるdriver thread/processが制約を継承する構成にする。mainで先にdevice列挙しない。全てのOpenCL操作をproducerへ集約する。
5. driver threadのgroup/allowed CPU、CPU ticks、状態を監視し、想定外のthreadが制約外にいればGPUを無効にして評価を失敗扱いにする。正常/異常停止の両方で今回のgroupと対象プロセスだけを回収する。

このlauncherは未実装であり、既存r3の「プロセス全体をCPU31へ入れるcontroller」をそのままソルバへ適用してはならない。カーネル側の設定差があるホストでは、確認なしにthread単位制約が成立した扱いにしない。

CPU探索をCPU0〜30へ限定する等の別案を選ぶ場合は、CPUだけの共有版にも同じ配置を適用して再基準化する。今回のmicrobenchは30探索＋GPUだったので、探索31本を維持した統合後の競合は未測定である。

## 実装判断のために必要な数値

まずC007とCPU共有版で、1要求あたりのGraph呼出し数、distinct PreparedGraph数、cache hit率、実構築回数、構築/待ち時間、経路materialize量、RSS、探索反復数を測る。共有でgraph時間が小さくなった場合は、その時点でGPUを後回しにできる。

GPU追加時は同じ共有CPU版に対して、setup、queue wait、転送、kernel、CPU DP/全表検証、cache公開、shutdownを分け、合計で比較する。GPUを使わないCPUフォールバックの件数と理由、期限切れ件数、全worker参加率も残す。CPU-only、共有CPU、共有CPU＋GPUの効果を混ぜない。

GPU有効化の有望条件は「共有後にも十分な構築量が残る」「起動が複数要求で償却される」「driver処理で減るCPU探索より構築短縮の利益が大きい」である。逆に、ほぼ全てcache hit、少数の短い要求でプロセス終了、小盤面で検証/配置費用が支配的、または待ち行列で探索workerが止まるならCPU共有だけを選ぶ。VOID3はGPU余力が小さいため、NVIDIA2機に利益があっても同じ設定を一律に適用しない。

最終的な採否は固定4チーム・全員ACの短縮12件を反復し、明確な改善が確認できた版の正式12件で判断する。時間制探索のばらつきがあるため、microbenchの高速化率や1回の小さな順位差だけで採用しない。
