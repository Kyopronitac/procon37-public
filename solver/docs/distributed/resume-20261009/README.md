# 分散ソルバ開発の再開記録（2026年10月9日）

2026年10月10日02:29:33 JSTにC012の正式12件が終了した。C012は全12ACだが、固定第8世代が車種提出の締切超過で4WAとなり、全員AC条件不成立。**正式改善・採用は未確定**。[正式評価の結果と保存D0との参考比較](C012-formal-20261010/result.md)を参照。Automationは未設定だったためユーザー照会で終了を確認し、失敗記録として確定した。評価用プロセスの停止・時刻同期の片付けは同記録に保存する。[直前の停止記録](pause-20261010.md)は履歴として保持する。

採用版は引き続き **D0**。初期正式12件は全48判定AC、平均順位1.6667、種類数222・日別種類数1276・玉数2989。実装commit `5d6c740962a1f3e20e77e13df243078df1d29ad2` の通常pushは完了済み。C012正式は全員AC条件不成立。今回の追加採用・commit・pushはない。

## 最新の評価状況

再開後の新環境でD0の短縮2回とC012の短縮1回を完了した。全て12件・全48AC、seed0〜11、各10秒、形式自動、練習場有効、固定4チーム。

| 版・試行 | 平均順位 | 種類数 | 日別種類数 | 玉数 |
| --- | ---: | ---: | ---: | ---: |
| H2-D0 短縮1 | 1.6667 | 222 | 1277 | 2961 |
| H2-C012 on 短縮1 | 1.2500 | 222 | 1277 | 2965 |
| H2-D0 短縮2 | 1.2500 | 222 | 1276 | 2982 |

D0短縮2終了後に一時停止した後、ユーザーの考察・正式実行指示を受けて同じ実機・設定を確認し、専用teamと時計を再起動した。C012短縮2は不要との指示を優先した。正式run `1791561513817407923-45859` は公式時間、固定4チーム、seed0〜11で終了したが第8世代が4WA。全員ACの共通8試合では保存D0平均1.625に対しC012平均1.500、得点比較2勝0同点6敗で、明確な改善とは判定しない。保存D0正式とのローカル実行ファイル識別差も残る。追加の探索実装・GPU試行はしていない。最新状態は `.server/distributed-state.json` と[正式評価の結果](C012-formal-20261010/result.md)、考察は[並行分析](restart-C012-02/README.md#反復中の考察)を参照する。

## 一時停止前の評価履歴（旧環境）

以下は2026年10月9日昼までの旧環境・10秒・固定4チームの確定済み結果で、全て12件・全48AC。一時停止と全評価プロセスの終了は当時の記録であり、現在は再開済み。[停止記録](pause-20261009.md)は履歴として保持する。新環境の成績と直接混ぜない。

| 分散版・試行 | 平均順位 | 種類数 | 日別種類数 | 玉数 |
| --- | ---: | ---: | ---: | ---: |
| D0 短縮1 | 1.3333 | 222 | 1277 | 2964 |
| D0 短縮2 | 1.2500 | 222 | 1277 | 2956 |
| C006 短縮1 | 1.1667 | 222 | 1277 | 2970 |
| C006 短縮2 | 1.5000 | 222 | 1277 | 2963 |
| C007 短縮1 | 1.5000 | 222 | 1276 | 2955 |
| C008 短縮1 | 1.6667 | 222 | 1273 | 2965 |
| C010 on 短縮1 | 1.4167 | 222 | 1276 | 2976 |
| C010 on 短縮2 | 1.6667 | 222 | 1276 | 2966 |
| C011 短縮1 | 1.6667 | 222 | 1276 | 2949 |
| C012 on 短縮1 | 1.1667 | 222 | 1277 | 2968 |

C012の初回はD0短縮1比6勝2同点4敗、短縮2比5勝2同点5敗。平均順位は改善したが、準決勝では悪化し、反復再現をまだ確認していない。その時点では停止指示に従い追加評価をせず未採用で保存した。正式な採否には現配置D0の正式基準も必要。C006/C010は反復で順位改善が確認できず、C011も改善なし。GPU C009bは未ビルドのまま保持している。

[D0の盤面サイズ別集計](D0-by-board-size.json)では、現配置の短縮2回平均は16×16が1.625、24×24が1.000、32×32が1.250（各8件）。初期正式の参考値はそれぞれ2.250・1.750・1.000（各4件）で、固定相手のビルドなどの条件が異なるため直接比較しない。

作業場所は実際に確認した `procon37` チェックアウトを使う。スクリプトは自身の配置からリポジトリルートを求め、旧作業者の絶対パスを前提にしない。開始時のGit作業ツリーはclean、HEADは `0a17ab83ed768cbbf590f96ee2f25cf6cc57905c`。`git ls-remote` でもoriginの同ブランチがこのcommitを指すことを確認した。D0の正式評価対象47ファイルは全SHA一致。

## 確認済みの引き継ぎ

- 保存正式評価5回の再集計では、第3世代P0 v3が平均順位1.6667で最良。開始版の再選定は不要。
- 今回の作業コピーには `.server` と固定単一2体のバイナリが存在しなかった。旧記録は別ユーザーのチェックアウトを指すが、現在のユーザーには読取権限がない。管理者読取は自動承認レビューが権限範囲不足で拒否し、ユーザーの明示回答待ち。許可なくアクセスしない。
- 初回再開時、NOMLAB `192.168.0.55` は複数回のSSH試行で `No route to host` だった。08:37 JSTの再確認でSSHログイン成功。実機は従来どおりRyzen 9 5900HSの8C16Tで、存在する16論理CPUを全使用する。solver・bridge・launcherのSHAは初回正式評価と一致。
- VOID1・2・3へSSHログイン成功。32・16・32論理CPU、NUMA各1ノード。開始時に対戦関係の稼働プロセスはなかった。

## 復元と設定

[local-restoration.json](local-restoration.json) にローカルのソース識別照合、実行ファイルSHA、CPU割当、ビルド環境を保存した。固定第6・第8世代は内部実装を閲覧せず、許可されたビルドのみで再作成した。ソースの識別hashは過去記録と一致する。ビルド環境が変わって実行ファイルSHAは異なるため、採用D0もこの条件で基準を取り直す。単一2体のCPUは異なる物理コアの0・2。

Rust 1.94.0とUbuntu GCC 15.2の必要パッケージは、Git管理外 `.server/toolchain` に配置した。システムのパッケージや他ユーザー領域は変更していない。ビルド時はリポジトリルートで次を実行する。

```sh
. .server/toolchain/env.sh
cargo build --release -p server -p server-bridge -j 4
```

環境復元時のRust導入手順は [Rust公式](https://rust-lang.org/tools/install/) を参照した。ローカルの固定実行ファイル・採用D0・評価時bridgeは `.server/distributed/bin`、単一2体の起動設定は `.server/distributed/launchers` にある。評価用bridgeは初回正式評価のバイナリSHA `24cb1008…` を保持する。現在の `run.sh` は新bridge用引数を含むため、そのまま旧bridgeへ組み合わせない。

`solver/distributed-lab/prepare.py` で三台それぞれの `$HOME/procon37-distributed-resume/20261009-D0` にD0・bridge・launcher・専用認証ファイルを配置し、転送後SHAを照合した。既存の本番配置、旧開発候補、NOMLABの固定配置は変更していない。この準備段階では起動せず、後述の時計同期と4チーム準備を終えてから開始した。配置manifestは `.server/distributed/deployments/20261009-D0/deployment.json`。秘密情報はGit記録に含めない。

[cpu-environment.json](cpu-environment.json) に再開時のCPU・LAN情報を保存した。VOID3だけgovernor/EPPが省電力側へ戻っていたため、前回正式評価と同じperformanceへ復元した。[変更後](v3-performance-restored.json)も保存済み。CPU割当と制御・探索の合計はD0の32・16・32を維持する。環境差の対戦成績への影響は、NOMLAB復旧後のD0再評価で確認する。

## 改善候補とGPU

[C006](../../../distributed-candidates/c006-budget/README.md) はD0の独立コピーに対し探索割合上限を91%から97%へ変えた2行の候補。固定110ms控除と通信余裕は維持した。単体11件・通信2件が成功したが、対戦未評価・未採用。D0本体を変更していない。[D0既存ログのCPU計測](d0-cpu-profile.json)では大盤のGraph構築中央値638ms、p90は902msだった。

[C006単日計測](c006-microbench-20261009.md)はVOID2の16論理CPUでD0と3ペアを比較した。全6提出は公開engineで合法、10秒以内（C006最長9773.3ms）。当日探索反復は6.14〜9.23%増え、初日得点は1勝2同点0敗だった。一方、翌日先読みは減り、全探索反復の変化は-0.29〜+2.04%に留まる。単日・少数の診断であり、4チーム12件の短縮評価や採用根拠には代用しない。

[gpu-inventory.json](gpu-inventory.json) に現在のGPU・ドライバ・既存資産と過去計測を記録した。VOID1はRTX 5070 Ti Laptop（12,227MiB）、VOID2はRTX 5060 Laptop（8,151MiB）、両方ともNVIDIA 595.104.02/OpenCLが稼働済み。VOID3のAMD GPUはrusticlと既存libdrm overlayで利用できる。追加ドライバ導入はしていない。

既存GPU試作の実行ファイル、計測スクリプト、結果111ファイルを `.server/recovered/gpu-20261009` へ回収し、全SHA一致。Rust/kernelソースは既知のリモート配置にないため、旧ローカル引き継ぎ資料の回収が望ましい。過去のGPU helper方式は27要求でCPU970msに対しGPU1007ms、初期化等を含め1322msとなり、通信費を含む改善は未確認。GPUの同一プロセス常駐化・共有メモリ転送が次の検討対象。

[今回のGPU追試](gpu-evaluation.md)では、CPU探索を約30論理コア相当で同時実行した際、同一プロセス常駐方式の27回分の経路表構築・queryが、初期化・終了込みでVOID1は約45%、VOID3は約25〜28%短縮した（各2反復）。毎回helperを起動する方式は不利だった。全16profileは正常終了、表不一致・fallback・CPU配置違反は0。人工queryを使い、GPUの表をD0に渡していない資源競合のmicrobenchなので、分散ソルバの得点改善は未証明である。[終了時確認](gpu-probe-postflight.json)では関連プロセスと一時cgroupの残存は0。

## 再開手順

1. NOMLABの接続と現在のCPU数・固定版SHA・設定・残存プロセスを確認する。旧引き継ぎ資料の管理者読取は別途許可が得られた場合だけ行う。
2. [再開ハーネス](../../../distributed-lab/README.md)に4チームの実配置とSHAを登録する。VOID2/3のworker、VOID1のteam0、NOMLABのteam1、ローカル固定2体を専用の起動記録付きで開始する。
3. 同じ4チーム、12件、seed0〜11、形式自動、練習場有効、10秒のD0基準を取得する。全48ACと前後の実ファイル・設定・配置一致を確認する。保存先は既定 `.server/runs` のみ。
4. C006やCPU/GPU候補を同条件の短縮評価で反復比較する。明確な改善だけ正式12件へ進め、全員AC・順位・三得点・勝同敗を確認する。今回は固定相手のビルドと配置条件が変わったため、正式な採否比較の前にはD0の正式12件も新しい同一条件で取得する。過去の初回正式結果を新条件の直接比較基準にはしない。採用時は評価した実装・設定・記録を日本語Conventional Commitsでcommitし、通常push成功を記録する。

進行情報は `.server/distributed-state.json`。2026年10月9日07:50 JSTの[終了確認](postflight.json)では、ローカルと接続できたVOID3台に関連プロセスの残存はなかった。この時点のNOMLABは接続不可で、Wake-on-LANの試行でも[復旧しなかった](nomlab-connectivity.json)。その後08:37 JSTに再接続できたため、4チーム評価の準備を再開した。初期D0のcommit・pushは完了済みだが、今回の作業ファイルは未commitで、新規採用はない。

## 再接続後の評価配置

[再接続時の監査](reconnected-inventory.json)で固定版SHAを確認。再接続後には本番用D0の待機起動がVOID1/2でポート39370を使用していたため、VOID1の本番 `run.sh stop` で所有プロセスだけを停止した。対戦窓口11451は未稼働で、進行中の試合はなかった。VOID3は再起動後に省電力設定へ戻っており、評価前に再びperformanceへ統一した。

専用の `team_runner.py` と `lifecycle.py` を作成し、4チームを起動した。NOMLABの既存solver・bridge・launcher・run.shは変更せず、試合待ちrunnerだけ追加した。全チームの環境を明示し、選択時間や探索予算の上書きは渡さない。ローカル固定2体はCPU0/2、問題serverと開発ビルドはCPU1/3/5/7を使い、固定相手の物理コアと分離する。VOID2/3のworkerは16/32thread、VOID1のcoordinatorは要求時に32threadで起動する。

配置と必須プロセスは `.server/distributed/resume-config.json` と `lab-config.json`、実起動PID・boot ID・開始tickは `.server/distributed/lifecycle-start.json` に保存。`.server/distributed/reconnected-readiness.json` で4台とローカル固定2体のSHA・CPU配置を確認済み。採用済みD0のソース47ファイルは変更していない。

[時計同期](clock-sync.md)を準備し、4台のchronyを問題server PCのユーザーポートUDP32123へ同期した。永続設定は変更せずバックアップも保存。評価前に一度だけ時計を合わせ、その後は連続補正を使う。時刻サーバーPID90752はCPU1/3/5/7で継続中。開始時の実測残差は全機0.15ms未満だった。

D0基準1回目を `resume-D0-short-01`、run `1791503276339409187-91328` として開始した。server PID91328、12件・4チーム・10秒、labelは「分散D0・再接続配置と時計同期の基準1回目・短縮12件・10秒」。[開始参加確認](participation-start-resume-D0-short-01.json)では、分散探索31/15/31本、NOMLAB探索15本、ローカル固定各1本の実働とCPU割当を確認した。結果確定前の途中成績を採用判定に使わない。

## 短縮評価の結果

D0基準1回目は全48ACで終了し、開始・終了のソース／実行ファイル／配置の一致を確認した。自動形式各4件、seed 0〜11、練習場有効、全員10秒。詳細は[集計](evaluations/resume-D0-short-01/results.json)、同ディレクトリのmanifest・前後照合・run.jsonを参照。既定履歴97ファイル（約20.3MB）のSHAを`history-artifacts.json`へ保存した。

| 版・役割 | 平均順位 | 種類数 | 日別種類数 | 玉数 |
| --- | ---: | ---: | ---: | ---: |
| D0 分散 | 1.3333 | 222 | 1277 | 2964 |
| 固定並列 NOMLAB | 2.3333 | 222 | 1276 | 2911 |
| 固定第6世代 | 3.3333 | 222 | 1277 | 2847 |
| 固定第8世代 | 2.5000 | 222 | 1276 | 2916 |

次は[C007](../../../distributed-candidates/c007-compact/README.md)を同配置で比較する。経路木のmetadataを一度に計算し、必要になった経路列だけ遅延展開する候補で、D0の探索時間配分を維持する。1,608,270経路の同値確認、単体13件・通信2件が成功した。[人工query計測](c007-microbench-20261009.md)の構築＋query合計は1.54〜1.98倍だったが、対戦での改善を保証する値ではない。C006は予備候補とし、独立したC008では機内の同一Graph共有を準備する。共有後のCPU構築量を測ってから[GPU併用](gpu-next-design.md)の利益を判定する。

C007短縮1はstage `C007-short-01`、run `1791504436521618109-94528`、server PID94528として開始した。分散3台のバイナリだけをSHA `49eb466a4eb127075bbb5ce9bd90e297ca19f8cb7e61f6af320d585aef53cecc`へ切替。開始前の34ソース・実行ファイル・配置照合に加え、[実働参加確認](participation-start-C007-short-01.json)で31/15/31探索、NOMLAB 15探索、全CPU割当と実行中SHA一致を確認した。clockは維持し、残差は全機0.063ms未満。評価中は実装・設定・配置を固定する。

C007短縮1は全48AC・前後識別一致で完了した。[結果](evaluations/C007-short-01/results.json)は平均順位1.5000、種類数222・日別種類数1276・玉数2955で、D0短縮1に対して **3勝3同点6敗**。今回の1回では順位・得点とも改善を確認できないため、正式評価・採用には進まない。人工queryでの高速化と対戦の改善は同一ではない。C007単体を変更せず保存し、次に機内での重複構築を減らす[C008](../../../distributed-candidates/c008-shared/README.md)を比較する。C008は単体21件・通信2件・重点レビューを通過し、未採用。

最初のD0/C007短縮選別中は、問題server用の物理コア側（CPU1/3/5/7）で次候補のビルド・検証も行った。固定単一2体の物理コアCPU0/2とSMT sibling4/6は使っていない。共有cacheやメモリ帯域を通じた影響までは排除できないため、採否につなげる短縮反復と正式評価では重い開発ビルド・別性能測定を停止する。

C008短縮1はstage `C008-short-01`、run `1791505730154449433-101944`、server PID101944で開始した。36ソース・実行ファイルSHA `651d075b36969555b360f6b21f77fcac6a2612dae5c6f0e1d7ef4758a7545491`を凍結し、[開始参加記録](participation-start-C008-short-01.json)で全4台の実働・CPU・実行中SHAを確認済み。重い開発ビルドとGPU測定を停止した状態で評価する。

C007終了とC008開始の間に、新規GPU producer試作をVOID1で1回だけ検証した。[実機記録](../../../distributed-gpu-lab/docs/v1-smoke-20261009-r1.md)は、synthetic 3size×2要求の距離・next全表、161,760経路のscalarがCPUと完全一致。初期化2710.706ms、正常shutdown63.303ms、CPU31以外の観測thread・timeout・強制kill・残存子孫は0。一時cpusetも削除済み。この試作はC008へ統合しておらず、対戦成績の改善を証明するものではない。[統合設計](../../../distributed-gpu-lab/docs/c008-integration-design.md)に共有owner優先、CPU fallback、初期化償却とホスト別有効化の案を保存した。

[D0/C007の実対戦Graph計測](graph-metrics/README.md)は、確定済みログ区間のSHAを照合して取得した。両版とも各機84要求の全workerが完了、parser欠落0。VOID3決勝のGraph取得中央値は638→292ms、p90は888→414ms、車種選択primaryの全形式p90は1519→351msとなった。Graph取得時間はwall timeであり、探索全体・CPU時間・GPUへ移せる時間ではない。iterationsや成績への効果は局面ごとに異なる。

C008短縮1も全48AC・前後識別一致で完了した。[結果](evaluations/C008-short-01/results.json)は平均順位1.6667、種類数222・日別種類数1273・玉数2965、D0短縮1比 **5勝2同点5敗**。玉数は1増えたが日別種類数と平均順位は悪化し、明確な改善とは扱わない。新規採用・正式評価には進めず、次に採用D0の短縮2回目で基準の時間制探索のばらつきを確認する。GPU owner統合C009は独立ディレクトリでコードを準備し、評価中の重いビルドやGPU測定を行わない。[PC間elite共有](cross-node-elite-design.md)も別改善案として設計段階にある。

D0の再現性確認2回目はstage `resume-D0-short-02`、run `1791506866153064781-104560`、server PID104560で開始した。[開始参加記録](participation-start-resume-D0-short-02.json)で分散3台とNOMLABのsolverがD0のe804…に一致し、単一2体を含む固定ファイルとCPU割当も同じことを確認した。この試行では重い開発ビルド・追加GPU実行を停止する。C008の[Graph/共有計測](graph-metrics/C008-summary.md)では構築回数を約48/51%減らした一方、worker最大メモリはC007比でVOID2 +7.37%、VOID3 +0.82%だった。

ユーザーから「GPUは改善が無理そうなら放棄してよい」と追加指示を受けた。GPU使用自体を目的にせず、C009はOwner-onlyの最小実装を未ビルドで保存し、次の評価間に有界な検証で判断する。実サイズで初期化・driver競合・検証/fallback費用を含めた利点がノイズ以下なら保留・放棄し、CPU探索とPC間協調を優先する。3台GPU展開や複雑なbrokerへは拡張しない。PC間elite共有C010はD0を親とする独立候補で準備中（コード編集のみ、対戦中のビルドなし）。

ユーザーは続けて「有用そうならGPUを進め、判断は任せる」と補足した。GPUを一律に除外せず、実サイズ・全費用込みの有界測定で有望なら継続し、対戦で評価する方針とする。

D0短縮2は全48AC・前後識別一致で終了した。[結果](evaluations/resume-D0-short-02/results.json)は平均順位 **1.2500**、種類数222・日別種類数1277・玉数2956。D0短縮1比は5勝1同点6敗で、順位1.2500〜1.3333・玉数2956〜2964のばらつきを観測した。C007/C008の順位1.5000/1.6667からは改善を確認できず、両候補とも正式評価・採用を保留する。C007/C008にはGraph構築とメモリ削減の効果があるが、採用には対戦成績の改善が必要。

[終了時の参加記録](evaluations/resume-D0-short-02/participation-completion.json)でVOID2/3は全84要求、15/31探索が全完了、入力列一致、fallback・invalid・timeout・panicは0。VOID1のstderrは最終試合のみ保存されるため全期間の診断を証明する記録ではなく、最終6要求では全31探索と2worker応答を確認した。全機performance、時計残差は0.15ms以内。D0の同一12件lifetimeでworker最大メモリはVOID2約5.0GiB、VOID3約10.3GiBだった。既定履歴97ファイルのSHAも保存した。

評価終了後、C010のCPU・通信テストとC009のCPU側の統合テストを別物理コアで開始する。次の対戦前に重い処理を完了し、探索予算配分だけを変えたC006を同条件短縮12件で比較する。新しい正式評価・採用・commit・pushはまだない。

C010はCPU1・ビルド並列数1で単体16件・localhost通信3件が成功した。共有onの両側で送受信各4件、取り込み8件・受信側改善3件を観測し、全提出が合法だった。これは動作確認であり、3台の対戦成績ではない。source `ce6f303f…`、binary `a47fbe2a…`を保存し凍結。切替ハーネスに`--cross-elite on/off`と初期launcherへの復帰を追加し、全11検査・読取レビューを通過した。実行中の比較相手・CPU・探索設定への変更はなく、評価開始前にハーネスSHAを再登録した。変更記録は`.server/distributed/harness-changes/20261009T101136-switch-elite/`。

C006の配置を10:08 JSTに完了した。切替記録は`.server/distributed/switches/20261009T100846-C006-budget97/`。binary `23c0d0ab…`、32ソース `4e8b4df8…`で、3台とも起動・識別照合済み。C009のローカル検証完了を待って、同じ4チームの短縮評価を開始する。

C006短縮1を10:17:39 JST、stage `C006-short-01`、run `1791508659730277323-111961`、server PID111961で開始した。C009はCPU単体29件・GPU library8件（GPUなし）・TCP1件・通信2件を通過して全子プロセス終了を確認し、評価前に重い処理を停止した。[C006開始記録](participation-start-C006-short-01.json)で31/15/31探索とNOMLAB15探索、全実行SHA・CPU配置一致、performance設定、時計残差0.15ms以内を確認した。最初の観測が要求間の待機に当たったため分散側だけ追加1窓で実働を確認しており、全機同時のCPU測定ではない。評価中は次のC011（D0直系の翌日終端位置評価）とGPU実機probeのコード準備だけを進める。

[D0短縮2の上限分析](D0-short-02-brand-upper-bounds.md)では、種類数222は在庫に基づく上限222に一致し、日別1277は上限1281より4少なかった。差はseed 0に集中する。この上限は移動・燃料・交通を無視しており、実際に残り4を取得できる証明ではない。種類数を保ちながら玉数を伸ばし、日別の取りこぼしも減らす方向で検討する。

[C011の設計](c011-terminal-reachability-design.md)をD0直系の独立候補へ実装準備した。翌日stepsの半分/全体以内に届くブランドmaskを使い、終端配置へ最大12点の小さい減点を加える。探索予算・通信・Graph生成・既存高速beamを維持し、canonical/forecast/selectionへ混ぜない。33ソース `de0d66c4…` はrootと別担当の読取レビューを通過したが、ビルド・6追加テスト・対戦は未実施。C006の評価中は実行しない。

C006短縮1は全48AC・前後識別一致で完了した。[結果](evaluations/C006-short-01/results.json)は平均順位 **1.1667**、種類数222・日別種類数1277・玉数2970。D0短縮2比は5勝2同点5敗、平均順位1.2500から1順位分改善、玉数+14。D0短縮1の玉数2964との差は+6に留まり、現段階は1回だけの小差として扱う。[反復判断](evaluations/C006-short-01/repeat-decision.json)を保存し、GPUの有限検証後にC006を同条件で反復する。正式評価・採用・commit・pushにはまだ進まない。

[C006終了記録](evaluations/C006-short-01/participation-completion.json)ではVOID2/3全84要求の15/31探索完了、入力列一致、fallback・timeout・panic観測0、全台performance、時計差0.16ms以内。VOID1は最終試合の6要求のみの診断だが全31探索と2worker結果を確認した。worker最大メモリはVOID2 5,247,376kB・VOID3 10,825,228kB。既定履歴97ファイルのSHAも保存した。評価終了後、C011と独立GPU owner probeのローカル検証をCPU1/3・各ビルド並列数1で開始する。

C006初回の玉数+14（対D0短縮2）の内訳はseed11が+16、他11件合計が-2だった。D0同士でもseed11の玉数は12変動しており、初回のみを明確な改善としない根拠として記録した。

C011はCPU1・並列数1でビルド、単体17件（追加6件）・通信2件を初回通過し、全子プロセスが終了した。ソース `de0d66c4…` は読取レビュー版から変更なし。GPU owner probeもCPU3・並列数1でビルドし、CPU-only 6組・3,208,980経路scalar照合、正常終了と全子プロセス終了を確認した。C009本体47ソースとbinaryは不変。probe source `49222ebe…`・binary `5854baa5…`で、VOID2の有限idle測定を1回だけ開始する。実探索との併用や対戦効果はまだ未検証。

VOID2の[初回GPU probe](../../../distributed-gpu-lab/c009-owner-probe/docs/v2-idle-20261009-r1.md)は起動gateで`EINVAL`となり、OpenCL初期化前に終了した。GPU比較0件なので速度効果は判断できない。専用controller/probe・子孫・cpusetは全回収済みで、既存C006と固定相手は変更していない。`writeln!`がTIDと改行を別writeにした可能性はあるが、失敗syscall自体は未特定。C009を保存し、独立C009bで1回のwriteと全byte数確認・エラー文脈の追加を準備する。評価中は再ビルド・実機追試しない。

C006短縮2は10:39:01 JST、stage `C006-short-02`、server PID117170で開始した。専用3台だけを同じbinary `23c0d0ab…`・同じlauncherで再起動して各workerのlifetimeを揃えた（切替記録`.server/distributed/switches/20261009T103833-C006-budget97/`）。固定相手・配置・CPU・時間・通信はそのまま。全追加ビルド・GPU処理の終了を確認してから開始した。C011の検証済みbinaryは`424cfb76596972a570ec0625ac0d247f2afb457087e02e4d2cb5a0e79c50d70a`、未対戦・未採用で保持する。

C006短縮2のrunは`1791509941774759940-117170`。[開始記録](participation-start-C006-short-02.json)で31/15/31＋NOM15探索、全SHA・CPU・performance一致、時計差0.033ms以内を確認した。VOID2/3の新worker PIDは9705/1683。

C009bは`solver/distributed-candidates/c009b-gpu-owner`に未ビルドで保存した。C009との差分は起動gateの単一writeと失敗時の扱い、その3テストだけ。47ソースSHA `18c63370e33d070da875afa630e66b2d2d529f653614586902fb679503b3985a`。追試probeは別ディレクトリ`solver/distributed-gpu-lab/c009b-owner-probe`、9ソースSHA `f55736f871fdb75b41707d1cd8dedf9c88592e8fdf6b30b585d9ad0d02353135`。別担当の差分レビューと識別照合を通過し、その後は古いコメントだけ訂正した。元C009・失敗probeは変更せず保持。次の評価間まで実行しない。

[seed 0の到達可能性診断](seed0-reachability-diagnosis.md)では、D0短縮2が日別種類を欠落した4日とも、少なくとも1台は開始燃料だけで対象店舗へ到達可能だった。全他ブランド・玉数を維持して追加取得できるかは未証明で、開始位置だけを原因と断定しない。時間内でも燃料が足りない車の例があり、C011の時間maskにもその限界がある。係数を特定seedに合わせず、全12件の比較で効果を判断する。

ユーザーから、平均順位がある程度改善しているなら区切りで最良候補のフルテストを行うよう追加指示を受けた。C006短縮2の完了後、順位改善が維持できていればC006正式12件を先に開始する。採否比較用として現配置・固定相手のD0正式12件も取得する。GPU追試・C010/C011の追加対戦はこの正式評価の間は保留し、重いビルドも行わない。短縮の10秒overrideを渡さず、公式時間と通常探索設定で評価する。

C006短縮2は全48AC・前後識別一致で完了した。[結果](evaluations/C006-short-02/results.json)は平均順位 **1.5000**、種類数222・日別種類数1277・玉数2963、D0短縮2比5勝2同点5敗。[2回の集計](evaluations/C006-short-02/repeat-decision.json)はC006平均順位1.3333・玉数平均2966.5に対し、D0平均順位1.2917・玉数平均2960。玉数の小幅改善はあるが順位改善は再現せず、現時点で正式評価へ進む明確な改善とは扱わない。この結果をユーザーへ説明し、C010のPC間候補共有を次の短縮12件で確認、明確な改善が出た段階で正式評価を優先する。GPU追試と新候補の拡大は保留を維持する。

C006短縮2の終了診断も全84要求の15/31探索完了、入力一致、観測fallback/timeout/panicなし、全performance・時計差0.20ms以内だった。[正式時間の監査](formal-time-audit-20261009.md)で、NOMを含む実launcherのSHA/内容とserver起動処理を確認した。`--short-ms`を省略すれば`timeLimitMs:null`となり、`--time-limit-ms`を渡さず、公式の選択時間と問題本来の各日秒数を使う。候補本体の予算配分変更と短縮テストのoverrideは区別して記録した。

C010 onを10:56:55 JST、stage `C010-on-short-01`、server PID119460で開始した。分散3台は33ソース `ce6f303f…`・binary `a47fbe2a…`、全launcherに`--cross-elite on`を明示したVERSION2 cohort。切替記録は`.server/distributed/switches/20261009T105628-C010-cross-elite-on/`。新規threadやポートは加えず、既存31/15/31探索のうち同じreserve15群の実日primary候補を共有する。固定相手・CPU・配置・10秒条件は維持し、評価中のビルド/GPU処理を停止する。

C010のrunは`1791511015750545757-119460`。[開始記録](participation-start-C010-on-short-01.json)で全実働・識別・CPU・共有onを確認した。[早期の実通信確認](participation-early-cross-elite-C010-on-short-01.json)は各機64KiB末尾を1回だけ読んだもので、例のrequest5でsent/received/imports/improvedはVOID1=6/4/12/4、VOID2=2/3/6/2、VOID3=2/3/10/5。観測8行のcontext_mismatch/oversized/rejectedは0。ホストごとに観測区間が異なり、全run合計でも対戦得点改善の証拠でもない。

ユーザーからD0の盤面サイズ別順位を質問されたため、[集計](D0-by-board-size.json)を保存して回答した。現在の10秒短縮2回（各サイズ8件）は16×16=1.625、24×24=1.000、32×32=1.250。初期正式（各サイズ4件）は2.250/1.750/1.000で、配置・固定単一バイナリの条件差がある参考値として区別した。

C010短縮1は全48AC・前後識別一致で完了した。[結果](evaluations/C010-on-short-01/results.json)は平均順位1.4167、種類数222・日別種類数1276・玉数2976。D0短縮2比8勝0同点4敗、短縮1比6勝2同点4敗。玉数は+20/+12で、seed11を除いても+8/+12と複数seedに分布する。一方、seed6の日別-1とseed0/4の玉数減が順位を押し下げた。[反復判断](evaluations/C010-on-short-01/repeat-decision.json)に基づき、C010同版で短縮を1回反復する。正式への直行や採用は行わず、新候補・GPU実行を増やさない。

[終了記録](evaluations/C010-on-short-01/participation-completion.json)ではVOID2/3全84要求の15/31探索完了と入力一致を確認した。[共有集計](evaluations/C010-on-short-01/cross-elite-completion.json)では両workerの全72日で送受信・取り込みがあり、VOID2のsent/received/imports/improvedは244/450/718/358、VOID3は260/440/1747/780。rejected・context mismatch・oversizedは0。VOID1の全run合計はログ仕様上取得できず最終試合の5日分だけである。時計残差は全台0.20ms以内、performance維持。既定履歴97ファイルのSHAを保存した。

C010短縮2を11:14:30 JST、stage `C010-on-short-02`、server PID122297で開始した。全3台を同じ33ソース `ce6f303f…`・binary `a47fbe2a…`・共有onのlauncherで再起動し、worker lifetimeを揃えた。切替記録は`.server/distributed/switches/20261009T111410-C010-cross-elite-on/`。固定相手・CPU・配置・10秒条件を維持し、追加ビルドとGPU処理を停止している。

C010短縮2のrunは`1791512070563519640-122297`。[開始記録](participation-start-C010-on-short-02.json)で31/15/31＋NOM15探索、CPU割当・実行SHA・全launcher一致、performance維持、時計差0.08ms以内を確認した。[C010の正式時間追補](formal-time-audit-C010-20261009.md)でも共有on以外の時間capがないことを確認。正式へ進む場合は同じ版とlauncherで`--short-ms`を省略する。

[C010のseed6診断](C010-seed6-diagnosis.md)では、日別不足は3日目のブランド10・cell194で、当日の開始位置・交通・燃料から2台が補給なしで到達できた。自作D0/C010の基本評価は日別種類×1500・玉×20で同一だが、終端評価と翌日rolloutも含むため、意図的な交換か探索未発見かは保存ログから区別できない。他ブランドを維持した改善経路や共有への因果は未証明。公開replayの移動1,498件・燃料収支3,368件をCPU1の軽い計算で確認した。

C010短縮2は全48AC・前後識別一致で完了した。[結果](evaluations/C010-on-short-02/results.json)は平均順位1.6667、種類数222・日別種類数1276・玉数2966、D0短縮2比5勝1同点6敗。[2回平均](evaluations/C010-on-short-02/repeat-decision.json)はC010順位1.5417・日別1276・玉数2971、D0順位1.2917・日別1277・玉数2960。玉数増だけでは順位/日別の悪化を補えず、正式評価を見送る。検証済みC011を次の短縮12件で確認し、新規候補の追加やGPU実行は保留する。

C010短縮2の[終了診断](evaluations/C010-on-short-02/participation-completion.json)も全84要求、15/31探索完了、入力一致、観測fallback/timeout/panicなしだった。[共有](evaluations/C010-on-short-02/cross-elite-completion.json)は両worker全72日で受信・取り込みを確認し、VOID2のsent/received/imports/improvedは220/415/661/322、VOID3は237/407/1645/714。rejected・context mismatch・oversizedは0。全performance・時計差0.09ms以内、既定履歴97ファイルのSHA保存済み。

C011短縮1を11:31:02 JST、stage `C011-short-01`、run `1791513062075302492-124172`、server PID124172で開始した。33ソース `de0d66c4…`・binary `424cfb76…`は既存の検証済み版と一致。切替記録は`.server/distributed/switches/20261009T113042-C011-terminal-reachability/`。3台とも元のlauncherへ戻しC010共有引数を残していない。次の日の店舗到達maskによる小さな終端評価補正を、同じ4チーム・CPU・配置・10秒で比較する。新しいビルド/GPU処理は追加していない。

[C011開始記録](participation-start-C011-short-01.json)で31/15/31＋NOM15探索、固定単一CPU0/2・serverCPU1/3/5/7、全実行SHA/launcher/CPUとperformance維持、時計差0.03ms以内を確認した。実launcher/argsにC010共有フラグはなく、C011の既定有効設定を無効化する環境変数もない。ただし開始監査は各実日の適用回数の証明ではない。

ユーザーから、有線LANを活かして日中のPC間通信を増やす案をD0ベースで検討するよう追加指示を受けた。C010で500ms周期のreserve15群共有は実動したが対戦順位は悪化したことを説明済み。C011評価はそのまま完了させ、D0の独立探索を大部分保つ少数受信担当、共有する候補の範囲、通信から再探索・最終採用までの追跡を検討する。原因を多様性喪失と断定せず、既存ログの通信実測と未測定事項を区別する。正式評価を優先する方針も維持する。

日中通信の次案C012を独立候補として実装準備する方針を決めた。D0本体・Graph・評価式・91%予算を維持し、C010の安全なtransport/contextを再利用する。各nodeの最小奇数reserve15 worker（現配置7/35/49）だけを受信専用とし、機内Elite送受信とdonorから外して他74本へ輸入解が伝播しないようにする。全7群はorigin×groupごとの候補を共通context付き32KiB batchで500msごとに公平に送る。受信担当は既存1024 iteration点で250msごとに非blocking確認し、1秒停滞/1秒cooldown/残750msの条件で最大3件を再評価、収集車経路の連続約1/4を削除して既存beamで最大25ms修復する。bestは保持し、実日primaryのみで通常の期限内に収める。係数は全形式・seed共通。これは未検証案で、成績改善は未証明。C011評価中はコード準備だけに留める。LANの有限idle測定スクリプトも準備し、実行は評価の区切り後に限る。

C011短縮1は全48AC・前後識別一致で完了した。[結果](evaluations/C011-short-01/results.json)は平均順位1.6667、種類数222・日別種類数1276・玉数2949、D0短縮2比4勝2同点6敗。[判断記録](evaluations/C011-short-01/decision.json)の通り、今回の順位・得点では改善が確認できず正式評価を見送る。採用D0はそのまま維持し、次にD0ベースのC012通信候補を準備する。既定履歴97ファイルのSHAを保存した。新たな採用・commit・pushなし。

[C011終了記録](evaluations/C011-short-01/participation-completion.json)でも全84要求/15・31探索完了、入力一致、観測fallback/timeout/panicなし、全performance・時計差0.10ms以内だった。

[LAN idle測定](lan-idle-20261009-r1.md)はC011確定後に実施し、3台とも1000Mbit full/MTU1500を確認した。v1↔v2/v3の32KiB往復各50回はp50=0.731/0.734ms、max=0.779/0.844ms。全500payload照合成功、controllerは全probe終了・強制signal/残存0を確認。これはPython echoのidle値で、実探索競合やJSON処理・取り込み待ち・対戦効果を含まない。固定対戦設定は変更せず、測定後のsnapshotも保存した。

[LAN後の独立確認](lan-idle-20261009-r1-postflight.json)もsuccess true。3台のprobe PID520/10206/2028と49371の待受は消滅し、C011元worker/runner・NOM固定runtime・local固定file/CPU・時計同期設定と時刻server PID90752は不変だった。

C011とLAN測定が完了した区切りで、切替ハーネスにC012の`--cross-elite on/off`必須指定を追加した。変更は候補名の許可と説明の2行で、実launcher生成処理は同じ。全11ハーネステストが成功し、`.server/distributed/harness-changes/20261009T115140-switch-c012/`へ旧/新ソースSHAと設定を保存して新SHAを登録した。C011・固定相手の実配置/バイナリは変更していない。C012はまだ未配置。

## C012の日中通信の初回短縮評価

C012はD0を基に、74本の独立探索と各PC1本の受信担当を維持する。7群の候補を500ms間隔で送り、受信担当は停滞時に部分破壊・修復して探索を続ける。ソース集約SHA `10acfa3bde81f1dddf1d80ed4ff2fbe0288395fc4eb6dd0923a410607ed8a541`、実行ファイルSHA `574dd99a8ff0d20035acccca3f4e207ad2d7701308bc10374694137aa0f33016`。単体18件・localhost TCP5件を通過し、ホスト側で記録28プロセスの終了を確認した。ローカルTCP試験では実時間の種変更は未観測のため、実対戦で確認する。

2026-10-09 12:08:32 JST、stage `C012-on-short-01`、server PID131487で短縮12件を開始した。共有は3台ともon、全員10秒、seed0〜11、自動形式、練習場有効。固定相手・CPU割当・時計同期と配置場所を維持し、評価中の実装・設定を凍結した。結果未確定・未採用。

C012の結果と停止確認を保存した。平均順位1.1667、222/1277/2968、全48AC、前後識別一致。実機で候補の送受信と種変更を確認した。2026-10-09T12:29:01+09:00時点で所有する評価用プロセス・待受はローカルと4台とも残存0、4台の評価用chrony sourceとローカルclock PID90752を停止済み。[停止記録](pause-20261009.md)の新しい指示後の手順で再開する。
