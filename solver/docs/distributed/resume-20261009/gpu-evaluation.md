# GPUとCPU探索の併用再測定（2026-10-09）

これは既存 `gpu-lab-compact-r3` の有限マイクロベンチである。4チーム対戦、候補の採用評価、分散ソルバへのGPU統合の評価ではない。NOMLABの接続復旧後に、採用D0との同条件の短縮12件・必要時の正式12件を別に実施する。

## 測定の識別と条件

- GPUラボ実行ファイルSHA256: `93e9347eedb2ce9d9fcfb0575cf425dd02e33c7a565557747987ec95e617e692`。過去のcompact-r3と同じバイナリで、Rust/OpenCLソースは未回収。
- 併用するD0実行ファイルSHA256: `e80474cc5f65a30b18d8143eec656e3abe093c927d75080aed2bd9dce72a5de0`。
- VOID1はRTX 5070 Ti Laptop、VOID3は2CUの内蔵Radeon。GPU構成と既存資産は [gpu-inventory.json](gpu-inventory.json)。両機のCPU governorは測定前にperformanceへ復元済み。
- 各profileはseed 0/1/2、初期・交通変更・重複位置の3状態、各3反復、計27回のGraph構築。毎回4096件の人工的な経路queryを処理する。対戦seed 0〜11の12件とは異なる。
- GPUラボ本体・helper・GPUドライバのスレッドを一時的なv1 cpusetのCPU31に制約する。CPU31はCPU15とSMTの物理コアを共有する。
- 単独条件（idle）はラボのみ。併用条件（coload）はD0の独立parallelモード、30探索＋制御1スレッドをCPU0〜30へ置き、同じ公開問題に60秒の車種選択要求を1回与える。対戦サーバーには接続せず、GPUラボの表をD0へ渡していない。
- coldは構築ごとにhelperとOpenCL contextを作り直すfinite profile。OSやドライバのディスクcacheは消去していない。warmは1つのcontextを27構築で共有するprofileで、最初の初期化費用を合計から除かない。
- cold/warmとも直接CPUは同じcompact表現を使う。coldのGPUにはhelperの通信と終了を含む。warmのGPUは同一プロセス内のproducerで、候補003-r2のpipe通信を含まない。

実行ラベルは `v1-r3-resume20261009-*` / `v3-r3-resume20261009-*`。既存ラベルは上書きしていない。併用controllerは [gpu_coload_probe.py](gpu_coload_probe.py)、集計と各生ログのSHA256は [gpu-probe-results.json](gpu-probe-results.json)。生ログは `.server/recovered/gpu-probes-20261009/` に保存する。

## 結果

各値は27構築とqueryの合計ms。`1回目 / 2回目` の順で記載する。warmのGPUは初期化を合計に含め、さらにOpenCL終了時間を加算した値である。

| PC | CPU探索の併用 | GPU context | 直接CPU compact (ms) | GPU (ms) |
| --- | --- | --- | ---: | ---: |
| VOID1 | なし | cold | 704.8 / 715.8 | 6880.1 / 9720.7 |
| VOID1 | なし | warm | 720.3 / 731.8 | 2980.1 / 475.0 |
| VOID1 | あり | cold | 1301.3 / 1302.8 | 11351.3 / 11365.9 |
| VOID1 | あり | warm | 1407.2 / 1419.4 | 768.7 / 773.2 |
| VOID3 | なし | cold | 809.6 / 816.4 | 2053.6 / 2025.4 |
| VOID3 | なし | warm | 827.3 / 826.2 | 731.5 / 722.4 |
| VOID3 | あり | cold | 1307.9 / 1323.3 | 3230.9 / 3247.1 |
| VOID3 | あり | warm | 1488.1 / 1384.4 | 1066.9 / 1032.8 |

両PCともcontextを継続利用する場合に計算上の利益があり、CPU探索との併用2回でも再現した。VOID1のwarmは直接CPU比で約45.4〜45.5%短縮、VOID3は約25.4〜28.3%短縮した。ただし、これはCPU探索がGPU結果を利用する実装の高速化率ではない。共存下でGPU側の計算利益が残ることを示す。

VOID1の最初のwarm起動は初期化だけで2675.2ms掛かり、27回構築ではCPUより遅かった。後の単独測定は151.1ms、併用測定は182.7〜183.3msだった。起動状態による差を除外していない。毎回contextを作るcold方式は全条件でCPUより遅い。VOID3のwarm初期化は単独33.8〜34.1ms、併用60.7ms程度だった。

併用中のD0はVOID1で29.897〜29.900、VOID3で29.884〜29.885論理コア相当のCPU時間を消費した。いずれもGPU測定の開始前から終了後まで動作しており、GPU区間だけ負荷が抜けた測定ではない。一方、CPU15/31のSMT共有、全CPU稼働時の周波数・電力・メモリ競合を含むため、単独時と併用時の差を特定の1要因には帰属できない。

全16profileがexit 0。1512件の計時対象で表一致・fallbackなし、144件の参照比較でD0一致、公開engineの経路確認は計5184件だった。全profileとCPU併用4回でaffinity違反・monitor errorは0。各profileは9入力を3反復する小規模測定であり、実戦におけるGraph要求数、cache hit率、種類数・日別種類数・玉数や順位の改善は未確認である。

2026-10-09 07:46 JSTに、対象のD0・GPUラボ・helper・monitor・controller・probeプロセスと、今回作成した一時cpusetがVOID1/VOID3の両方で残っていないことを確認した。各controllerも残存PIDなし・cgroup削除済みを記録し、強制回収は発生していない。証跡は [gpu-probe-postflight.json](gpu-probe-postflight.json)。再現用スクリプトと結果ファイルのみリモートに残している。

## 次の実装候補

既存候補003-r2は同一表現CPUの970.35msに対しGPU helperが1007.03ms、setup/shutdown込み1322.44msだった。GPU kernelだけの高速化では十分でなく、表の受け渡し費用を減らす必要がある。以下は回収した設計資料と今回の実測からの案で、未実装である。

1. **共有メモリで表を受け渡す常駐helper**。各PCに1つだけGPU helperを置き、`memfd`による共有領域へ直接OpenCLの結果を読み戻す。既存のpipeは要求と完了通知だけに限定し、数MBの表をencode・pipe転送・decodeする処理を省く。CPU探索は検証済みの距離・次セル・経路scalarを読み取り専用で共有する。最初はsingle in-flightのまま実装し、転送方法の変更と探索方式の変更を分ける。
2. **寿命と整合性を明示する**。表headerにwire version、要求ID、入力digest、shape、offset/length、世代番号を持たせ、各乗算・加算と上限を検査する。入力の同一性は既存のPrepared全要素比較を維持し、digestだけでcacheを一致扱いにしない。helperの書込み終了をrelease/acquireで通知し、親の検証完了後に公開する。参照が残る表をhelperが再利用しないようleaseを管理する。helperの停止・回収を確認するまで、その未完成領域をCPU fallbackへ転用しない。
3. **同一プロセス内producerも比較対象にする**。1本の専用producerがcontext/programを保持し、完成したimmutable compact表を`Arc`でworkerへ配布する。今回のwarm測定に近い経路になるが、GPU runtimeの初期化時に作られるthreadのCPU制約を継承させ、探索threadの配置を変えない設計が必要。helper分離による停止回収の容易さは失われるため、締切とGPU停止時の挙動を別途検証する。
4. **CPU探索との非同期併用**。同一入力の重複構築を避ける共有cacheを先に検証し、独立した入力はGPU待ちで全workerを止めない。cache hit・要求数・mutex待ち・GPU待ち・CPU fallback・実探索反復数を記録する。GPUの利益が小さい入力や、起動費用を償却できない要求列ではCPUを選べるようにする。
5. **比較条件をそろえる**。CPU compact、同一プロセスGPU、共有メモリhelperを同じ入力・同じGraph検証・同じ総CPU枠で比較する。初期化、同期、全表検証、query、終了、異常時の回収を計時に含める。今回の人工queryで得た差を対戦成績へ直接換算しない。

GPUソースの再編集には元ソースの回収またはD0からの再実装が必要である。現在のバイナリを逆解析してソースを再現する作業はしていない。
