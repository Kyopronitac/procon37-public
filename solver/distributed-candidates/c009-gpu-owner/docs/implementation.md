# 実装状態と検証待ち

2026-10-09、D0短縮反復中にコードのみ準備し、その終了後の評価間隔でローカル検証を完了した。`--offline --locked` release buildと単体30件、GPU library CPU-only 8件、localhost通信2件が成功。CPU 3/-j1、全5子PID終了を確認。[validation.json](../validation.json)参照。**C009のリモート配置・実GPU検証は未実施**。以下の設計と追加testはこの評価済みsourceに対応する。

## 主な変更

- `solver/planner.rs`: CPU最短木のDijkstra本体を維持し、mode mask/potential/mobility/最終allocation計上を共通assemblerへ切り出す。GPU assemblerの期限検査をsource行ごとに行う。
- `solver/gpu_graph.rs`: PreparedGraph→Prepared、検証済み7mode距離/6mode next→RouteTree/Path、fuel INF→99999、CPU tree DP、host別opt-in、TIDだけのgroup参加、producer寿命を管理。
- `solver/graph_cache.rs`: OwnerだけGPU試行。CPU/GPU完了内訳を分離し、fallbackの高水位見積りはCPU構築だけで更新。既存のBuildingGuard・完全key・共有/私有bypass・attach1回を保持。
- `pool.rs`: processのSearchPoolに対応するGPUを非同期初期化し、Drop時に最大100msだけ終了を待つ。search thread数や探索係数は変更しない。
- `gpu/`: 独立prototypeの候補内コピー。新しい`scalars_until`はtree/64cellごとに期限検査する。`opencl.rs`、`producer.rs`、`shortest.cl`はコピー元と同じ。
- `tools/gpu_cpuset.py`: rootのprepare/inspect/cleanup専用。GPUやsolverを起動せず、cleanupはtasksが空の場合だけ削除する。今回ユーザー所有0600のtasksだけを委譲し、cpuset controlsはroot管理に残す。

## 予算と一時メモリ

GPU試行の予算は`min(cap, remaining/8, remaining-(1.5*cpu_estimate+20ms))`。初期capは250ms、CPU見積りの未測定時はC008の1µs/(cell×target×6mode)。1ms未満の予算では試さない。入力変換・producer待ち・CPU tree DP・Path組立・mask作成を同じ絶対期限へ含める。GPU受付はsingle-flight＋try_sendで、他のGPU ownerの後ろに待ち行列を作らない。

scalar/表一時領域は`148*cell*target` bytesの推定で上限64MiB。最終GraphTablesやallocator管理領域、driver/RSSはこの上限に含まない。予算不足やshape上限ではCPUへ直行する。GPU結果の期限超過・異なる入力の返答・組立失敗ではGPUをprocess内で無効化し、部分表をcacheへ出さずCPUを構築する。

hard deadlineを保証するのはGPUのcaller待ちだけ。CPU fallbackの見積り、巨大allocation/解放、driver内部の実行を強制中断する保証はない。既存coordinatorの提出時刻、候補保持、遅延結果排除は変えていない。

## 効果の限界と中止判断

C008評価終了後の集計では、C007→C008のplan.primary Graph中央値がv2で129→55.5ms、v3で126→47msに短縮した一方、forecastはv2で66→75ms、v3で76→83msだった。83要求分の構築削減率は47.71%/51.02%、private bypassは921/2226、timeout/failureは0。bypassは主に4entry全てBuildingによるもので、Owner-only GPUが既に速くなったprimaryばかりを処理し、遅いforecastをCPUに残す可能性がある。

ユーザーは「GPUは改善が無理そうだったら放棄してもいい」と明示した。GPU使用自体を目的にしない。この未ビルドsnapshot以上の機能拡張は保留し、次の区切りでlocal correctness、v2の限定tasks委譲とdriver制約の有界preflight、実サイズowner表のCPU off/on少数反復までで価値を判定する。

初期化/driver/全表検証/CPU組立/終了を含めた時間・CPU競合・RSSを計上し、共有後primary約55msに対する短縮とrequest全体のiteration/締切余裕の差が反復ノイズを超えない、またはGPU側の費用が勝るならC009を保留・放棄し、CPU探索とPC間協調へ戻る。性能不明のまま短縮対戦を増やさず、brokerの複雑化や3台GPU展開は行わない。

owner-onlyに小さいが再現する効果があり、対象外のprivate構築が明確な残課題と分かった場合に限り、GPU off/onを比較した後でprivateもnonblocking admissionできる**別候補・別設定**を検討する。現在のC009ではprivateへのGPU利用を実装しない。

## 追加した意味のあるtest（ローカルで成功）

1. 5/17格子の位置重複・交通反映・blocked INF・非target fuel_cacheと、CPU/GPU adapter全Path/scalar/mask/potential/mobilityの一致。
2. seed 0〜2の3生成入力でGPUなしの全表adapter照合。既存のD0 eager 1,608,270経路同値testも保持。
3. 異なる入力の検証済み表、期限切れ、late replyの拒否とGPU無効化。CPU reserveの確保。
4. mock GPU owner成功時のallocation単一attach、Ready再利用、CPU見積り非汚染。
5. NotReady/失敗/異入力時の同owner CPU fallback、private bypassがGPUを呼ばないこと、owner panicのFailed通知とCPU再構築。
6. コピーしたproducer testによる受付失敗・GPUを使わないmockの期限超過/有限shutdown。runtime/kernel実行の成功を意味しない。

## v2限定の起動案

評価開始前のroot準備で新規groupを作る。group名例は`procon37-c009-v2-<run-id>`。root準備scriptが作ったgroup pathを`PROCON37_C009_GPU_CPUSET`へ渡す。`PROCON37_C009_GPU=opencl`をv2にだけ設定し、v1/v3は`off`。selector既定は`RTX 5060 Laptop GPU`、CPUは15固定。GPU processの起動前にC008 baseline/current evaluatorが完全に止まった区切りを確認する。

startup gateはgroup controlsのroot所有・非group/world書込み、tasksの実効UID所有0600を確認し、自身TIDをtasksへ書く。`cgroup.procs`やTGIDは使わない。既存Confinement検査を通ってからloaderを呼ぶ。tasks委譲のkernel権限条件やdriver/compiler子孫の継承は実機未確認なので、試合投入前に有限preflightでmain/searchが移らないこととGPU threadのCPU 15制約を検査する。

v2 workerは全12件persistentとして初期化を償却する。v1 coordinatorは毎試合再起動するため初版off。v3も新kernelのRusticl検証がまだなのでoff。searchは31/15/31で、既存のhost全CPU集合を維持する。GPU CPUをsearchから完全予約する場合は配置変更としてC008も同条件で評価し直す。

## 次の順序

ローカルbuild/test/localhost検査は完了。次の評価間隔でv2のroot準備/producer参加の実機確認、実target数の3形式GPU表同値、driver thread制約、終了/子孫回収を確認する。GPUの利益を全費用込みの少数off/on測定で判定し、有望で必要な確認が通った場合だけ短縮12件へ進む。成績が明確に改善した候補だけ正式12件へ進む。
