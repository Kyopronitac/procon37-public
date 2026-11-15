# GPUを再試行する範囲と判断条件

2026-10-10。H2-D0/C012の短縮反復中に、保存結果と自作コードだけを読んで検討した。SSH、ビルド、ベンチ、GPU起動、配置変更は行っていない。現行の評価対象・設定は変更していない。

**C009bの有限preflightを対戦の区切りで1回行う価値は残る。ただしC012の正式評価判断を優先し、GPU統合や3台展開を先に進めない。** 前回はGPUを動かす前に起動処理が失敗したため、GPU性能を棄却する根拠がまだない。一方、CPU共有後にGPUへ移せる仕事量は限られ、GPU表生成が速いだけでは採用条件を満たさない。

## 確認済みの材料

| 材料 | 分かること | 分からないこと |
| --- | --- | --- |
| 旧r3常駐GPU・CPU探索併用 | VOID1で約45%、VOID3で約25〜28%、27構築/queryの合計時間がCPU compactより短かった。各2反復で再現 | CPU探索はGPU表を使っていない。C009bや現在の環境での速度、順位改善 |
| 独立producerのVOID1 smoke | 16/24/32格子・各8 targetの6要求が全表一致。初期化2710.706 ms、終了63.303 ms | 実入力の162〜209 targetでの費用、CPU探索との競合 |
| C009の実サイズCPU-only probe | seed 0〜2×2反復、3,208,980経路scalarが一致。CPU-onlyのGPU側分岐はCPU fallback | 実GPUの正確性・速度。旧ローカルCPU計時をVOID2へ移すこと |
| C009のVOID2初回probe | OpenCL前のgateでEINVAL、GPU attempts=0、比較pair=0。プロセス・一時group回収済み | 失敗syscallの特定、GPUが速いか遅いか |
| C009b | TIDと改行を1つのbufferにし、1回の`write`で渡す修正と3つのmockテストを準備済み | 未ビルド・未実行。元C009の検証を引き継いだとは扱わない |

参照：[r3併用測定](../gpu-evaluation.md)、[r3集計](../gpu-probe-results.json)、[VOID1 smoke](../../../../distributed-gpu-lab/docs/v1-smoke-20261009-r1.md)、[C009失敗記録](../../../../distributed-gpu-lab/c009-owner-probe/docs/v2-idle-20261009-r1.md)、[C009b変更範囲](../../../../distributed-candidates/c009b-gpu-owner/docs/change.md)。

`writeln!`の断片書込みがEINVALを生んだという説明は**仮説**である。C009bの成功は修正の実用性を支持するが、元の失敗syscallを観測したことにはならない。失敗時は追加したopen/write文脈を確認し、原因未確定のまま再試行を重ねない。

## 実サイズと初期化の償却

C009のCPU-only probeが実際に生成したshapeは次のとおり。CPU時間は以前の作業PC・CPU 3の値で、現在の5800HやVOID2の性能予測に使わない。現在のC009bはGPUの一時領域を`148 × cell × target` bytesと推定し、既定64 MiBで制限する。

| seed / 格子 | cell / target / source | 旧ローカルCPU builder 2回 (ms) | 一時領域推定 |
| --- | --- | ---: | ---: |
| 0 / 16×16 | 256 / 209 / 209 | 67.752 / 61.616 | 7.55 MiB |
| 1 / 24×24 | 576 / 162 / 447 | 119.373 / 123.353 | 13.17 MiB |
| 2 / 32×32 | 1024 / 195 / 776 | 270.027 / 284.585 | 28.18 MiB |

これらのshapeは64 MiBの事前制限内にある。ただし、最終Graph、allocator、driver、VRAMはこの推定外である。小さいsmokeの「GPU約1〜2 ms」をtarget数比で線形外挿しない。CPU全表検証、tree DP、Path作成、mode maskの費用も増える。

償却はGPU kernel時間ではなく、同じbuilder全体を比較する。要求`i`の節約を`d_i = CPU構築時間 - GPU試行を含む構築時間`、初期化と終了の合計を`F`とし、実際に採用可能だった要求列の`sum(d_i) - F`を記録する。GPU側には入力変換、受付、待機、allocation、upload、dispatch/read、全表検証、CPU組立、fallbackを含める。初回要求やJITが遅くても取り除かない。

既存probeの`ceil(F / mean(d_i))`は、6要求と同じ入力分布が続くという**仮定上の損益分岐**である。平均節約が0以下なら償却不能。旧r3ではVOID1併用の概算損益分岐が約8構築、VOID3が約4構築だったが、VOID1の初回起動条件では約155構築になった。これは初期化変動の証拠であり、C009bの必要要求数ではない。

VOID2 workerは12試合を通じて常駐するため初期化を分散できる。VOID1 coordinatorは試合ごとに再起動するため、初版GPU offを維持する。VOID3は新しいkernel/統合版のRusticl検証を終えていない。既存C009bはVOID2専用であり、制限を外す変更は別候補にする。

## CPU共有後の上限と競合

[C008実測](../graph-metrics/C008-summary.md)では、VOID2の完了要求1〜83にGraph取得2685、build 1404、cache owner 483、private bypass 921があった。GPU owner限定方式の対象はbuildの34.4%、取得の18.0%にとどまる。これらは**要求84を含まない旧環境C008の集計**であり、新環境の要求数ではない。

同じ記録のplan.primary Graph取得中央値は55.5 ms、forecastは75 ms。前者を半分にしても単純な10秒要求時間比では約0.28%分であり、直ちに対戦順位へ反映する規模とはいえない。ただし同じ表を待つ複数workerの解放や複数回の構築へ効く可能性があるため、この比を全体効果の厳密な上限にも使わない。forecastのprivateが主な残量という説明は推論であり、現ログはbypassをprimary/forecastへ直接帰属していない。

GPUはCPU計算を全て除去しない。producer/driverはCPU 15で動き、探索15本＋制御は既存の16論理CPU集合を使う。CPU 15は探索と競合し、SMT相方、メモリ帯域、電力配分にも影響し得る。過去のVOID1/3併用試験は30探索で、VOID2の15探索とは条件が違う。GPU off/onで同じCPU集合・同じ探索本数を保ち、driver CPU時間、全プロセスCPU、RSS、Graph待機、反復数を一緒に見る。GPUへCPUを完全予約するなら配置変更としてCPU-onlyも同じ条件で再評価する。

C008の旧短縮成績は平均順位1.6667、222/1273/2965で、CPU共有だけの採用には至っていない。C009bでGPU off/onが良化しても、それだけでは採用D0や最良通信候補を上回ったことにならない。

## 最小再試行と進退条件（今後の提案）

以下の数値は今後の候補選別用の実務的な基準であり、既存の測定結果や統計的有意性を表すものではない。実行は現在の対戦とその記録固定が終わった区切りに限る。

1. **ローカルの最小検証。** C009bのTID単一write/短いwrite/errno22の3テスト、既存CPU Graph一致・fallback・期限の関連テスト、補助probeのCPU-only 6pairをCPU 3・`-j1`で検証する。source aggregate `18c63370e33d070da875afa630e66b2d2d529f653614586902fb679503b3985a`、probe source aggregate `f55736f871fdb75b41707d1cd8dedf9c88592e8fdf6b30b585d9ad0d02353135`と実際のsource/build SHAを照合し、変更があれば新しい識別を記録する。
2. **VOID2で既存有限probeを1回。** [準備済み手順](../../../../distributed-gpu-lab/c009b-owner-probe/docs/procedure.md)の新規配置・新規groupを使う。起動前に再起動後のGPU、driver、cgroup mount、CPU/SMT、governor、既存プロセスを確認する。45秒monitor・外側65秒TERM＋10秒KILLを保持し、全6pairをCPU→GPU/GPU→CPUで比較する。最初からactive searchは加えない。
3. **正しさ・回収の合格は必須。** 全6pairの全Graph一致、GPU fallbackなし、producer Ready、main/sentinel配置不変、CPU/UID/group逸脱なし、正常shutdown、exit 0、強制回収なし、子孫とgroup消失を全て要求する。gate失敗・不一致・残存は性能評価未成立として止める。低性能は失敗run扱いで隠さず有効な棄却結果として保存する。
4. **性能の一次足切り。** 各形式の2回ともGPU builderがCPUより速く、6pair合計で10%以上かつ1owner平均5 ms以上節約できれば、active search併用の測定へ進む候補にする。初期化/終了込みの6要求が負なら、必ず償却件数も示す。旧483 ownerのうち約25%しか実際にGPU受付できない場合を仮置きし、損益分岐が120要求を超えるなら初版を保留する。120は実際の受付数ではないため、次段階で実測に置き換える。僅差・形式によって悪化・cap内に収まらない場合は、初版C009bを保留し、CPU/PC間協調を優先する。
5. **有望時だけ実探索を少数比較。** 固定した公開入力seed 0/1/2を、同じC009bバイナリのGPU off/onで各10秒、各2反復する。off/onとon/offの順序を交替し、計12要求を上限の目安にする。各process内は3入力を連続処理し、初期化・初回・終了を保持する。GPU側は15探索＋制御の実際のcache・forecastを使い、GPU off側も同じ探索本数・配置を使う。これはserver12件の代用ではない。全員ACはその後のserver評価で確認する。

段階5では、実際のGPU完了数、NotReady/Busy/予算・shape skip、CPU owner/private、Graph取得待ち、primary/forecast別反復、要求の完了時刻、選ばれた候補、driver/プロセスCPUとRSSを保存する。起動費用を引いたowner節約が2反復とも正で、形式を合算した探索反復も両反復で正、形式ごとの反復差が大きく悪化しないことを最低条件とする。実務上は反復平均+1%以上、各形式−1%以内を次のserver短縮候補の目安とし、単に集計Graph時間が短いだけなら進めない。時間制探索の反復は仕事量が変わるので、速度の証明ではなく選別材料として扱う。

条件を満たせば、新環境・同じ固定4チームでGPU off/onをそれぞれ12件・seed 0〜11・各10秒・arena有効で比較する。採用候補との比較も同条件で行い、平均順位と種類数/日別種類数/玉数、勝・同点・敗を反復で確認する。有望なら公式時間の正式12件へ進む。C012へ混ぜる場合はこの実験後に別識別の候補として構築し、元C012の評価済み版を上書きしない。

## 障害時の扱い

現コードは起動未完了、Busy、予算不足、shape制限でCPUへ戻れる。GPU computeの期限超過はproducerの以後の受付を無効化し、異なる表や組立期限超過は完成cacheへ公開しない。GPU callerの待機は有限だが、driver内部の実行やCPU fallbackそのものを強制中断する保証ではない。探索が続くことと、GPU driverが回収済みであることを分けて確認する。

起動時のGPU有効化失敗でもCPUだけで完走し得るため、全ACだけで「GPU版を評価した」としない。完了GPU要求が0ならCPU fallbackの評価である。初期化/JITの数秒が探索CPUを奪った場合も隠さず記録する。producer診断に`last_error`があるStoppedを正常shutdownと扱わない。

今回の結論はGPU採用でも全面放棄でもなく、**新しい機能を追加せず、準備済みC009bの起動障害修正と実サイズowner 6pairだけで価値を再判定する**というもの。そこで利益が薄ければGPU開発を保留し、現在の通信候補の改善・正式評価を続ける。
