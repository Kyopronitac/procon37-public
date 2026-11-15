# 探索途中の PC 間 elite 共有案

2026-10-09。D0/C008 の自作 runtime・pool・wire・planner を読んだ設計メモです。**未実装・未測定・未採用**。現在の C008 対戦には介入せず、source、設定、ハーネス、リモートを変更していません。

## 判断

最初の候補は、**実日の primary 探索だけで、同じ reserve_weight 群の途中最良 Solution を VOID1 経由で共有する方式**とします。既存 TCP と制御 loop を使い、500 ms に1回まで最新値を送ります。受信側が自分の Graph と評価関数で再評価し、改善した場合だけ次の restart で使います。車種選択・予測 rollout・Snapshot の途中移植は初版に入れません。

C007 の実対戦では Graph 取得時間が短縮した一方、全形式の plan.primary iterations 中央値はほぼ横ばいでした。演算高速化に加え、ある PC が見つけた良い経路から別 PC が改良を続ける余地を調べる価値があります。ただし、3機の探索が同じ解へ収束して独立探索の多様性を失う可能性があり、効果を前提に採用しません。[既存ログの比較](graph-metrics/README.md)

## 現在の共有範囲

| 場所 | 現在の動作 |
| --- | --- |
| PC 間 | VOID1 が同じ Task を配布。各機は独立して探索し、全 worker 完了または締切で各機最良の Result を1回返す。探索途中の Solution 配信はない |
| 機内 | 要求ごとに7個の `Arc<Mutex<Elite>>` を作る。global worker ID に対する `(worker/2)%7` で共有先を決める |
| 共有するデータ | `Elite { value, solution: Option<Solution> }`。Solution は hubs、hub_modes、departures、各車の符号化 route。Graph/乱数/Snapshot は共有しない |
| 公開頻度 | primary の探索 loop で16384 iteration ごと。最良値が改善したときだけ slot を更新 |
| 取り込み頻度 | 探索予算を12分割した restart 境界。候補を自分の評価関数で評価し、自分の最良値を上回れば採る |
| 評価の群 | 同じ `(worker/2)%7` で reserve_weight = 4/8/12/15/20/30/45 を選ぶので、機内 slot はこの重みと対応する |
| 車種選択 | `Operation::Select` は exchange を設定しない。supplier の異なる候補間の途中共有はない |
| 予測日 | `plan_with_rollout` は実日を解いた後に `exchange.take()` し、rollout では外す。予測状態が異なる worker 間では共有しない |

現状も全 worker が完全に同一の探索をするわけではありません。同じ採用 Snapshot と観測から始めつつ、global worker ID ごとの乱数 seed、beam の係数、temperature、7種類の reserve_weight を持ちます。3機間では終端集約まで独立ですが、各機内には重み群ごとの協調があります。

`SearchPool::begin` の7個の exchange は各 Job へ渡した後、制御 loop から参照する仕組みがありません。PC 間共有では、この要求専用 handle を pool/control loop にも保持することが最初の接続点です。現在の機内 `Elite` は Graph の node 番号が一致する前提なので、そのまま無条件にネットワークへ公開することはできません。

## 最小の経路

```text
VOID2 primary ─ offer ─┐
                       ├─ VOID1 の制御 loop ─ 同群の最新候補 ─ VOID2/VOID3
VOID3 primary ─ offer ─┘           ↑
                          VOID1 primary の offer
```

全3機の31/15/31探索 thread と各機主threadを維持します。通信専用thread、別port、別serverは追加しません。

1. Job ごとに `CrossEliteScope` を作り、session/request/task の識別、primary 段階、評価群、Graph context と最新値 mailboxes を保持する。
2. primary Graph が完成した時点で exact context を登録する。既存の16384 iteration 公開点で、途中 best の版番号だけを更新する。ネットワーク serialization は探索threadで行わない。
3. 主threadが500 msごとに変更された slot を `try_lock` で短時間だけ読み、lockを外してから符号化する。新しい版がなければ送らない。
4. VOID1 は同じ要求・context・群の最新候補を中継する。受信者自身の worker が、次の既存 restart 境界で1件だけ読み取り、lock外で再評価する。
5. 受信者の現在の最良値より改善する場合だけ `best[0]` を更新する。乱数状態は保ち、今の restart 処理で探索を再開する。新たな強制 restart は加えない。

途中 offer は**探索の種**です。`Candidate.answer` として提出したり、`Coordinator.state` を更新したりしません。最後は各 worker が現在と同じ処理で rollout・合法性確認を行い、既存の Candidate/Result と `valid_for` を通して採用します。

## 同一性と wire の案

既存の `Task::digest()` はJSONに対する64-bit FNV整合識別子で、暗号認証ではありません。認証 token、session、request、digest の検査を保持した上で、次を追加します。

| メッセージ | 必要な内容 |
| --- | --- |
| `EliteContext` | session、request、Task digest、primary段階、day、steps、supplier、objective群、実装/評価schema識別、context ID、完全な Graph context |
| `EliteOffer` | 上記要求識別と登録済みcontext ID、origin node/worker、origin単調sequence、objective群、Solution、診断用の有限評価値 |
| `EliteRelay` | 元offerのorigin/sequenceを保持した転送。VOID1の別sequenceを付けても原点は変更しない |

context ID は検索用です。初回に送る完全な Graph context と受信 worker が持つ context を**全値比較**して登録し、hash一致だけで受理しません。少なくとも C008 `PreparedGraph` と同じ map、最終 durations/costs、spot_at、positions の順序と重複、cell_node、targets、店舗位置を含めます。Taskの同じ観測・採用状態に基づくことも前提です。評価schemaと実装識別は3機で一致させます。

primaryでは同じ Snapshot と観測から Graph を構築するため一致が期待できますが、実行時に確認します。anchorsの順序をsortしたり、hub番号を推測で置き換えたりしません。異なるcontextは初版では共有対象外です。contextを1回登録すれば、500 msごとのofferに全配列を繰り返し載せる必要はありません。

受信は次の順で検査します。

1. 認証済み接続、互換protocol/実装、現在のsession/request/digest、active Jobが一致すること。origin workerが接続元に割り当てたID範囲内であること。relayは元のoriginを保持すること。
2. primary段階、観測day、steps、supplier、評価schema/objective群が一致すること。`Operation::Plan` かつ `future_weight=0` の対象だけを許可する。`selecting=false` だけでは車種選択の `select_role` を除外できないので、Job/段階による明示的なscopeを使う。
3. 同じcontextの origin sequence が既受信より大きく、自分が出した候補の反射ではないこと。内容が同じSolutionならsequenceが違っても再配信を抑制する。
4. hubsが `<targets`、hub_modesが `<PATH_MODES`、route本数が車数、route tokenが `(shops+2*STOPS)*PATH_MODES` 未満、departuresが許容時間内など、shapeと値域を確認する。payload・token数には固定上限を設け、超過は共有だけを破棄する。
5. worker自身のcontextとの完全一致を確認した上で、そのworkerのGraph/観測/seen_brands/重みで `evaluate` し直す。送信者の `value` だけで best を上書きしない。

JSON評価値は有限であることを要求しますが、中継時の値は候補整理に使う参考値です。現行の機内 Elite に remote value を直接代入すると、受信側で再評価する前に閾値を書き換えてしまうため、remote inbox は分けます。受信側で再評価して改善した解だけが、その後の通常の機内公開を経由します。

## 締切、状態同期、切断

時刻は各PCの `Instant` と既存の相対 `budget_ms` を使います。他PCのmonotonic時刻を比較しません。

- 主threadのenqueueは要求探索期限の少なくとも500 ms前まで。workerの取り込みは自身のprimary局所期限まで250 ms以上残る restart 境界だけ。これらは初期値で、短縮対戦ログの費用を見て調整する。
- primaryが終わったworkerはscopeを閉じ、rollout・coverage probeへ引き継がない。探索済みworkerを再起動して取り込ませない。
- 新要求、接続切断、Result完了でその接続/Jobの受信状態を失効させる。前要求workerが遅れて公開しても、新しい要求専用mailboxには入らない。
- `EliteOffer/Relay` は `peer.pending`、remote_results、worker completed数を変更しない。現在の終了判定を早めない。`Result` が最終候補であることを保持する。
- `Snapshot.game/last/cache/seen_brands` をネットワークeliteで差し替えない。前日不受理は既存の `accepted` と状態同期処理だけが処理する。
- 通信が滞ればofferを落とし、独立探索と最終Resultを続ける。共有到着待ち、ACK待ち、barrierは作らない。締切前の再接続をeliteのために追加しない。

現在の `FramedSocket` は非blocking・FIFOで、frame上限4 MiB、送信queue上限8 MiBです。この既存queueへ各workerが無制限にofferを追加する実装にはしません。既存の2 ms制御loopも受信eliteの処理で占有しないよう、1回のrelay/parse作業を件数・bytesで制限します。

## 通信量と多様性を制限する初期設定

- request/context/objective群ごとの最新1件だけを保持し、未送信の古いofferを置き換える。
- 500 msごとにTCP方向ごと最大1 batch、batch 16 KiB、単一Solution 8 KiB、context登録64 KiBを上限案とする。上限を超えれば共有を省略し、元の探索・Resultは続ける。
- batchを実際のsocket queueへ入れるのは制御/Result待ちqueueが空のときだけとし、elite frameは同時に高々1つ。まだ送っていないeliteはResultを優先して破棄できるよう、socketへ渡す前のoutboxで保持する。
- 途中まで送信したTCP frameは捨てられない。16 KiBの上限で影響を小さくし、backpressure試験でResult遅延を測る。リンク停止時の厳密な到着保証とは扱わない。
- 2 peerの送受信4方向なら定常elite上限は VOID1 合計約128 KiB/s（context登録を除く）。毎回full Snapshotを送る構成は避ける。

最初の有効化は **reserve_weight=15 の群3だけ**を推奨します。他6群は今の機内共有・PC間独立性を維持し、機内共有による二次伝播でも群を越えないため独立探索を残せます。「各群の半数workerだけ直接受信を許す」方法では、受け取った解が既存機内Eliteを通じて残り半数にも伝わり、独立探索を保証できません。

群3で採用された輸入解が実際に改良されることを確認してから、全7群、750/1000 msなどを別条件として比較します。公平な対比は同じバイナリの共有off/onで行い、設定を変えた基準版も評価し直します。

## 実装する場合の変更境界

| ファイル | 想定変更 |
| --- | --- |
| `wire.rs` | bounded context/offer/relay、互換性識別。旧workerへ未知variantを送らない |
| `pool.rs` | Job専用CrossEliteScope、制御loopからの最新slot参照、失効/世代管理 |
| `solver/planner.rs` | primaryだけのscope、Graph context登録、既存publish/restart地点への非blocking連携と再評価 |
| `runtime.rs` | VOID1の有界relay、remote daemonのoffer送受信、期限/識別/重複/queue制御 |

D0/C008/C007は変更せず、別ディレクトリ・別候補として作ります。protocol versionを上げた3機同一版のcohortで評価し、off設定でも同じ通信制御コードを通す対照を用意します。旧versionとの混在時は途中共有を有効にしないことを明示します。NOMLABと固定単一thread2体は変更しません。

## 代替案

| 案 | 利点 | 制約 |
| --- | --- | --- |
| 終了済みworkerのCandidateを早期Resultとして逐次送るだけ | 現在のvalid_forを流用しやすく、遅いworker/peerへの耐性を改善できる | 他PCの探索を進める協調にはならず、全workerがほぼ同時終了する要求では利益が小さい |
| Solutionをセル座標表現へ正規化して異なるGraph間にも共有 | 異なるanchor番号でも転用できる余地 | hub/route/補給の意味を保つ変換が必要。重複位置とmodeの同一性確認が難しく初版から外す |
| 経路の一部だけを共有して局所修復する | 解全体の同一化を抑えやすい | 部分routeと補給日程の整合・新しい交叉操作の品質評価が必要 |
| PCごとの探索係数/seed戦略だけを多様化する | 新しい通信・同期費用がない | 良い解を他PCが途中改良する効果は得られない。独立対照として有用 |
| 3機mesh/ringで直接交換 | VOID1の中継費用を分散できる | 接続/失効/ループ排除が増える。現在の小さな3機構成ではstarの実測後に判断 |

## 評価項目と進め方

最初に local correctness で同じSolutionの受信前後評価一致、hub/anchor順序違いの拒否、旧session/request/Job/sequence、異なるreserve群、異なるday/traffic/supplier、予測日からの誤輸入を検査します。切断・復帰、旧worker遅延、前日不受理、重複/反射/順序逆転、大frame、送信詰まりでも最終候補と状態同期が維持されることを既存network testに追加します。

実対戦で必要な記録は次です。

- context一致率と不一致理由、offerの公開/受信/重複/期限破棄数、群ごとの有効import数。
- import時の自分の再評価値、直前bestとの差、import後にさらに改善した数、最終採用解のorigin/系譜。単なる受信件数を改善件数にしない。
- 500 ms周期の遅れ、主threadのserialization/validation時間、探索threadのlock失敗/再評価時間、queue bytes、送受信量、Result到着時刻と期限余裕。
- 異なるSolution数、各群/PCの最終選出比率、iteration数。全PCが同じ解へ偏る副作用を確認する。
- 同じ4チーム・12seed・同一時間でoff/onを反復し、平均順位、種類数・日別種類数・玉数、全AC、変更前後の勝/同点/敗を比較する。

現行の機内共有だけの対照、PC間群3、全7群などを混ぜて1回の小差で結論を出しません。明確な短縮改善が得られた候補だけ正式12件で判定し、評価版と同一のsource/設定を採用commit・pushします。CPU/GPU共有との交互作用は、まず採用済み版のoff/onで切り分けます。

## 読み取り対象の識別

C008 source aggregate：`ff0e146e8ebf18f9934708eb85a274a3ede3f237e22c8430922dccf544da0a60`。

| 対象 | SHA-256 |
| --- | --- |
| D0/C008 runtime.rs（同一） | `a8ebea03ba5ef47dafe65305488bb2c60af8a1a7e9c6efaf4ae386dada045922` |
| D0/C008 wire.rs（同一） | `7f438dff1998ec9ff6008eb1242e50f2edd5017544f0d589be687147de845f7e` |
| D0 pool.rs | `74a981b5f1423b16e07b8f0e888e00c797b41b670140921112b874b4a6b27214` |
| C008 pool.rs | `c1196b294979edec465ebaa26bf51ca8679e4d20a4d0b2d968ca6532a864ff57` |
| D0 planner.rs | `11e043e33018dee34f41f295d6ae1500497b0bfc7c8c33d5b253a5621587461a` |
| C008 planner.rs | `a8d6abd6b2bdbe7c15945cf5f1b42092b6d53442dd5b5f820e785f2ac761955d` |

参照箇所は `SearchPool::begin`、worker Job実行部、`plan_inner`の16384 iteration公開/restart取込、`plan_with_rollout`、`select_role`、`Candidate::valid_for`、`Coordinator::solve`、`daemon`、`FramedSocket`。ブラックボックス2領域の参照はありません。
